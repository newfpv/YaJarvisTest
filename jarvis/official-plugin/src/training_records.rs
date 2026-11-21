//! The optional training plugin owns the recording buffer and local dataset.
use crate::{
    storage::{rand_hex, write_atomic},
    training_background::{Recorder, ShortUpload},
};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

const RATE: usize = 16_000;
const MAX_CLIP: u64 = 6 << 20;
pub const MAX_LIMIT_MB: i64 = 50;
const KINDS: [&str; 4] = ["pos", "false", "near", "bg"];
const CLIPS: &str = "rust-clips";
const OLD_CLIPS: &str = "clips";
const GZIP: &[u8] = b"\x1f\x8b";
const OFFLOAD_BATCH: usize = 10;
const UPLOAD_CUSHION: u64 = 1 << 20;

#[derive(Clone, Serialize)]
pub struct Clip {
    pub kind: String,
    pub name: String,
    pub reviewed: bool,
    pub size: u64,
    pub time: String,
    #[serde(skip)]
    storage: &'static str,
}

#[derive(Default)]
struct Observe {
    near: Option<Instant>,
    background: Option<Instant>,
}

pub struct Records {
    root: PathBuf,
    files: Mutex<()>,
    sample: Mutex<Observe>,
    pub background: Arc<Recorder>,
    offload: Mutex<Offload>,
    write_pending: AtomicBool,
    stopped: AtomicBool,
}

#[derive(Default)]
struct Offload {
    running: bool,
    sent: usize,
    retry_at: Option<Instant>,
    error: String,
}

impl Records {
    pub fn new(root: PathBuf) -> Self {
        let background = Arc::new(Recorder::new(root.join("background-cloud.json")));
        Self {
            root,
            files: Mutex::new(()),
            sample: Mutex::default(),
            background,
            offload: Mutex::default(),
            write_pending: false.into(),
            stopped: false.into(),
        }
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.background.close();
    }

    pub fn sample_kind(&self, hit: bool, prob: f32, loud: bool) -> Option<&'static str> {
        if hit {
            return Some("pos");
        }
        let mut s = self.sample.lock().unwrap();
        if (0.35..0.99).contains(&prob)
            && s.near
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(20))
        {
            s.near = Some(Instant::now());
            return Some("near");
        }
        if loud
            && prob < 0.15
            && s.background
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(420))
        {
            s.background = Some(Instant::now());
            return Some("bg");
        }
        None
    }

    pub fn save_soon(
        self: &Arc<Self>,
        rt: &tokio::runtime::Handle,
        kind: &'static str,
        pcm: Vec<i16>,
        prob: f32,
        limit_mb: i64,
    ) {
        if self.write_pending.swap(true, Ordering::AcqRel) {
            return;
        }
        let records = self.clone();
        rt.spawn(async move {
            let upload = records.background.short_upload();
            let files = records.clone();
            let connected = upload.is_some();
            let saved = tokio::task::spawn_blocking(move || files.save(kind, &pcm, prob, limit_mb, connected)).await;
            match saved {
                Ok(Ok(true)) => {
                    if let Some(upload) = upload {
                        records.start_offload(upload);
                    }
                }
                Ok(Ok(false)) => {}
                Ok(Err(error)) => log::warn!("запись для обучения: {error}"),
                Err(error) => log::warn!("запись для обучения: {error}"),
            }
            records.write_pending.store(false, Ordering::Release);
        });
    }

    fn start_offload(self: &Arc<Self>, upload: ShortUpload) {
        let mut state = self.offload.lock().unwrap();
        if state.running || state.retry_at.is_some_and(|at| at > Instant::now()) {
            return;
        }
        state.running = true;
        drop(state);
        let records = self.clone();
        tokio::spawn(async move {
            let result = records.offload_batch(upload).await;
            let mut state = records.offload.lock().unwrap();
            state.running = false;
            match result {
                Ok(sent) => {
                    state.sent += sent;
                    state.error.clear();
                }
                Err(error) => {
                    state.error = error;
                    state.retry_at = Some(Instant::now() + Duration::from_secs(60));
                }
            }
        });
    }

    async fn offload_batch(self: &Arc<Self>, upload: ShortUpload) -> Result<usize, String> {
        let records = self.clone();
        let batch = tokio::task::spawn_blocking(move || records.pack_oldest())
            .await.map_err(|error| error.to_string())??;
        let Some((name, bytes, paths)) = batch else { return Ok(0) };
        upload.put(&name, bytes).await?;
        let records = self.clone();
        tokio::task::spawn_blocking(move || {
            let _lock = records.files.lock().unwrap();
            for path in paths {
                // A clip reviewed or removed while the upload ran has a different path.
                if let Err(error) = fs::remove_file(path)
                    && error.kind() != std::io::ErrorKind::NotFound
                {
                    return Err(error.to_string());
                }
            }
            Ok(OFFLOAD_BATCH)
        }).await.map_err(|error| error.to_string())?
    }

    fn pack_oldest(&self) -> Result<Option<(String, Vec<u8>, Vec<PathBuf>)>, String> {
        let _lock = self.files.lock().unwrap();
        let mut clips = self.list_dir(CLIPS);
        clips.retain(|clip| !clip.reviewed);
        if clips.len() < OFFLOAD_BATCH {
            return Ok(None);
        }
        clips.sort_by(|a, b| a.time.cmp(&b.time).then_with(|| a.name.cmp(&b.name)));
        let mut paths = Vec::with_capacity(OFFLOAD_BATCH);
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let mut id = DefaultHasher::new();
        for clip in clips.iter().take(OFFLOAD_BATCH) {
            let path = self.path(CLIPS, &clip.kind, &clip.name)?;
            let data = read_clip(&path)?;
            if data.len() > RATE * 3 * 2 + 44 {
                return Err("Запись длиннее трёх секунд".into());
            }
            let name = format!("{}/{}", clip.kind, clip.name);
            name.hash(&mut id);
            data.hash(&mut id);
            zip.start_file(name, archive_options()).map_err(|error| error.to_string())?;
            zip.write_all(&data).map_err(|error| error.to_string())?;
            paths.push(path);
        }
        let name = format!("jarvis-clips-{}-{:016x}.zip", clips[0].name.trim_end_matches(".wav"), id.finish());
        let bytes = zip.finish().map_err(|error| error.to_string())?.into_inner();
        Ok(Some((name, bytes, paths)))
    }

    fn path(&self, storage: &str, kind: &str, name: &str) -> Result<PathBuf, String> {
        if !KINDS.contains(&kind) || !valid_name(name) {
            return Err("Неверное имя записи".into());
        }
        Ok(self.root.join(storage).join(kind).join(name))
    }

    fn locate(&self, kind: &str, name: &str) -> Result<(PathBuf, &'static str), String> {
        for storage in [CLIPS, OLD_CLIPS] {
            let path = self.path(storage, kind, name)?;
            if path.symlink_metadata().is_ok_and(|meta| meta.is_file()) {
                return Ok((path, storage));
            }
        }
        Err("Запись не найдена".into())
    }

    fn list_dir(&self, storage: &'static str) -> Vec<Clip> {
        let mut output = Vec::new();
        for kind in KINDS {
            let Ok(entries) = fs::read_dir(self.root.join(storage).join(kind)) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                if !valid_name(&name) || !entry.file_type().is_ok_and(|t| t.is_file()) {
                    continue;
                }
                let Ok(meta) = entry.metadata() else { continue };
                if meta.len() > MAX_CLIP {
                    continue;
                }
                let changed = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                let time = jiff::Timestamp::try_from(changed)
                    .unwrap_or(jiff::Timestamp::UNIX_EPOCH)
                    .to_string();
                output.push(Clip {
                    kind: kind.into(),
                    reviewed: name.ends_with("_r.wav"),
                    name,
                    size: meta.len(),
                    time,
                    storage,
                });
            }
        }
        output
    }

    fn list(&self) -> Vec<Clip> {
        let mut clips = self.list_dir(CLIPS);
        clips.extend(self.list_dir(OLD_CLIPS));
        clips.sort_by(|a, b| b.time.cmp(&a.time).then_with(|| b.name.cmp(&a.name)));
        clips
    }

    pub fn snapshot(&self, mode: bool, limit: i64, previous: bool) -> Value {
        let _lock = self.files.lock().unwrap();
        let clips = self.list();
        let counts = KINDS.map(|kind| (kind, clips.iter().filter(|c| c.kind == kind).count()));
        let used = |storage| {
            clips
                .iter()
                .filter(|c| c.storage == storage)
                .map(|c| c.size)
                .sum::<u64>()
                >> 10
        };
        let offload = self.offload.lock().unwrap();
        json!({"mode":mode,"limit_mb":limit.clamp(1,MAX_LIMIT_MB),"prev_model":previous,
            "used_kb":used(CLIPS),"legacy_kb":used(OLD_CLIPS),"local":clips.len(),
            "offload":{"connected":self.background.short_upload().is_some(),"running":offload.running,"sent":offload.sent,"error":offload.error},
            "counts":counts.into_iter().collect::<std::collections::BTreeMap<_,_>>(),"clips":clips.iter().take(400).collect::<Vec<_>>()})
    }

    fn save(&self, kind: &str, pcm: &[i16], prob: f32, limit_mb: i64, connected: bool) -> Result<bool, String> {
        let _lock = self.files.lock().unwrap();
        if self.stopped.load(Ordering::Acquire) {
            return Ok(false);
        }
        let wav = pcm_wav(pcm);
        let compressed = compress(&wav).map_err(|e| e.to_string())?;
        let data = if compressed.len() < wav.len() {
            compressed
        } else {
            wav
        };
        let limit = (limit_mb.clamp(1, MAX_LIMIT_MB) as u64) << 20;
        let name = format!(
            "{}-{}_p{}.wav",
            jiff::Timestamp::now().strftime("%Y%m%d-%H%M%S"),
            rand_hex(5),
            (prob.clamp(0.0, 1.0) * 100.0) as u32
        );
        let path = self.path(CLIPS, kind, &name)?;
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        if cfg!(unix) && crate::storage::free_mb(&self.root) < 4 {
            if connected {
                return Ok(true);
            }
            return Err("на колонке осталось меньше 4 МБ".into());
        }
        let mut clips = self.list_dir(CLIPS);
        let mut used = clips.iter().map(|clip| clip.size).sum::<u64>();
        if connected {
            let pressure = used + data.len() as u64 > limit;
            if used + data.len() as u64 <= limit + UPLOAD_CUSHION {
                write_atomic(&path, &data, 0o600).map_err(|error| error.to_string())?;
            }
            return Ok(pressure);
        }
        clips.sort_by_key(|clip| {
            (
                clip.reviewed,
                !matches!(clip.kind.as_str(), "bg" | "near"),
                clip.time.clone(),
            )
        });
        for clip in clips {
            if used + data.len() as u64 <= limit {
                break;
            }
            if clip.reviewed {
                return Ok(false);
            }
            if fs::remove_file(self.path(CLIPS, &clip.kind, &clip.name)?).is_ok() {
                used -= clip.size;
            }
        }
        if used + data.len() as u64 > limit {
            return Ok(false);
        }
        write_atomic(&path, &data, 0o600).map_err(|e| e.to_string())?;
        Ok(false)
    }

    pub fn read(&self, kind: &str, name: &str) -> Result<Vec<u8>, String> {
        let _lock = self.files.lock().unwrap();
        let (path, _) = self.locate(kind, name)?;
        read_clip(&path)
    }

    pub fn move_clip(&self, kind: &str, name: &str, to: &str) -> Result<(), String> {
        let _lock = self.files.lock().unwrap();
        let (source, storage) = self.locate(kind, name)?;
        if to == "delete" {
            return fs::remove_file(source).map_err(|e| e.to_string());
        }
        let stem = name.trim_end_matches(".wav").trim_end_matches("_r");
        let mut target = self.path(storage, to, &format!("{stem}_r.wav"))?;
        if target == source {
            return Ok(());
        }
        if target.exists() {
            target = self.path(storage, to, &format!("{stem}_{}_r.wav", rand_hex(4)))?;
        }
        fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::rename(source, target).map_err(|e| e.to_string())
    }

    pub fn clear(&self) -> Result<(), String> {
        let _lock = self.files.lock().unwrap();
        for clip in self.list() {
            fs::remove_file(self.path(clip.storage, &clip.kind, &clip.name)?)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Compact the internal files without changing their logical names or WAV downloads.
    pub fn compact(&self) -> Result<Value, String> {
        let clips = {
            let _lock = self.files.lock().unwrap();
            self.list()
        };
        let mut changed = 0;
        let mut saved = 0;
        for clip in clips {
            let _lock = self.files.lock().unwrap();
            if self.stopped.load(Ordering::Acquire) {
                break;
            }
            let path = self.path(clip.storage, &clip.kind, &clip.name)?;
            let Ok(meta) = path.symlink_metadata() else {
                continue;
            };
            if !meta.is_file() || meta.len() > MAX_CLIP {
                continue;
            }
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            if bytes.starts_with(GZIP) || !bytes.starts_with(b"RIFF") {
                continue;
            }
            let smaller = compress(&bytes).map_err(|e| e.to_string())?;
            if smaller.len() >= bytes.len() {
                continue;
            }
            write_atomic(&path, &smaller, 0o600).map_err(|e| e.to_string())?;
            if let (Ok(file), Ok(time)) = (fs::File::open(&path), meta.modified()) {
                let _ = file.set_modified(time);
            }
            changed += 1;
            saved += bytes.len() - smaller.len();
        }
        Ok(json!({"changed":changed,"saved_bytes":saved}))
    }

    pub fn archive(&self, data_dir: &Path, sensitivity: i64, output: impl Write) -> Result<(), String> {
        let _lock = self.files.lock().unwrap();
        let clips = self.list();
        let mut zip = zip::ZipWriter::new(ArchiveWriter::new(output));
        zip.set_flush_on_finish_file(true);
        let options = archive_options();
        let mut add = |name: &str, data: &[u8]| -> zip::result::ZipResult<()> {
            zip.start_file(name, options)?;
            Ok(zip.write_all(data)?)
        };
        let model = fs::read(self.root.join("wake.bin.tuned"))
            .or_else(|_| fs::read(data_dir.join("wake.bin")))
            .map_err(|e| e.to_string())?;
        add("wake.bin", &model).map_err(|e| e.to_string())?;
        add(
            "wake.json",
            json!({"sensitivity":sensitivity}).to_string().as_bytes(),
        )
        .map_err(|e| e.to_string())?;
        add(
            "finetune.py",
            include_bytes!("../../modules/wake-train/assets/finetune.py"),
        )
        .map_err(|e| e.to_string())?;
        for clip in clips {
            let data = read_clip(&self.path(clip.storage, &clip.kind, &clip.name)?)?;
            add(&format!("{}/{}", clip.kind, clip.name), &data).map_err(|e| e.to_string())?;
        }
        let mut writer = zip.finish().map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())
    }
}

// ZipWriter patches the current entry's header before flushing it. Keep just
// that entry (or the final directory) seekable while sending completed bytes.
struct ArchiveWriter<W> {
    output: W,
    pending: Cursor<Vec<u8>>,
    sent: u64,
}

impl<W> ArchiveWriter<W> {
    fn new(output: W) -> Self {
        Self {
            output,
            pending: Cursor::new(Vec::new()),
            sent: 0,
        }
    }
}

impl<W> Read for ArchiveWriter<W> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.pending.read(bytes)
    }
}

impl<W: Write> Write for ArchiveWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.write_all(self.pending.get_ref())?;
        self.sent += self.pending.get_ref().len() as u64;
        self.pending.get_mut().clear();
        self.pending.set_position(0);
        self.output.flush()
    }
}

impl<W> Seek for ArchiveWriter<W> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let position = match position {
            SeekFrom::Start(offset) => {
                SeekFrom::Start(offset.checked_sub(self.sent).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "cannot seek into a completed ZIP entry")
                })?)
            }
            relative => relative,
        };
        self.pending.seek(position).map(|offset| self.sent + offset)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.sent + self.pending.position())
    }
}

fn valid_name(name: &str) -> bool {
    name.len() <= 112
        && name.ends_with(".wav")
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn read_clip(path: &Path) -> Result<Vec<u8>, String> {
    if !path
        .symlink_metadata()
        .is_ok_and(|m| m.is_file() && m.len() <= MAX_CLIP)
    {
        return Err("Запись не найдена".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(MAX_CLIP + 1).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_CLIP {
        return Err("Запись слишком большая".into());
    }
    if !bytes.starts_with(GZIP) {
        return Ok(bytes);
    }
    let mut wav = Vec::new();
    GzDecoder::new(bytes.as_slice())
        .take(MAX_CLIP + 1)
        .read_to_end(&mut wav)
        .map_err(|e| e.to_string())?;
    if wav.len() as u64 > MAX_CLIP || !wav.starts_with(b"RIFF") {
        return Err("Повреждена сжатая запись".into());
    }
    Ok(wav)
}

fn compress(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(bytes)?;
    encoder.finish()
}

pub fn pcm_wav(pcm: &[i16]) -> Vec<u8> {
    let size = (pcm.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + size as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
    bytes.extend_from_slice(&(RATE as u32).to_le_bytes());
    bytes.extend_from_slice(&((RATE * 2) as u32).to_le_bytes());
    bytes.extend_from_slice(b"\x02\0\x10\0data");
    bytes.extend_from_slice(&size.to_le_bytes());
    bytes.extend(pcm.iter().flat_map(|sample| sample.to_le_bytes()));
    bytes
}

fn archive_options() -> zip::write::SimpleFileOptions {
    zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cloud_offloads_exactly_ten_and_only_deletes_after_confirmation() {
        use axum::{Router, body::Bytes, http::StatusCode, routing::put};
        let root = std::env::temp_dir().join(format!("jarvis-offload-{}", rand_hex(5)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("background-cloud.json"), json!({
            "provider":"nextcloud", "endpoint":format!("http://{}/", listener.local_addr().unwrap()),
            "user":"test", "password":""
        }).to_string()).unwrap();
        let (sent, mut archive) = tokio::sync::mpsc::channel(1);
        let fail = Arc::new(AtomicBool::new(true));
        let state = fail.clone();
        let server = tokio::spawn(async move {
            let app = Router::new().route("/{archive}", put(move |body: Bytes| {
                let fail = fail.clone();
                let sent = sent.clone();
                async move {
                    if fail.load(Ordering::Relaxed) {
                        return StatusCode::SERVICE_UNAVAILABLE;
                    }
                    sent.send(body).await.unwrap();
                    StatusCode::CREATED
                }
            }));
            axum::serve(listener, app).await.unwrap();
        });
        let records = Arc::new(Records::new(root.clone()));
        let wav = pcm_wav(&[241; 16_000]);
        let kind = root.join(CLIPS).join("near");
        fs::create_dir_all(&kind).unwrap();
        for i in 0..11 {
            fs::write(kind.join(format!("{i:02}.wav")), &wav).unwrap();
        }
        fs::write(kind.join("00_r.wav"), &wav).unwrap();
        assert!(records.offload_batch(records.background.short_upload().unwrap()).await.is_err());
        assert_eq!(records.list_dir(CLIPS).len(), 12);
        state.store(false, Ordering::Relaxed);
        assert_eq!(records.offload_batch(records.background.short_upload().unwrap()).await.unwrap(), 10);
        let mut zip = zip::ZipArchive::new(Cursor::new(archive.recv().await.unwrap())).unwrap();
        assert_eq!(zip.len(), 10);
        assert!(zip.by_name("near/00.wav").is_ok());
        assert!(kind.join("10.wav").is_file());
        assert!(kind.join("00_r.wav").is_file());
        server.abort();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn keeps_collecting_when_the_archive_is_above_five_megabytes() {
        let root = std::env::temp_dir().join(format!("jarvis-training-limit-{}", rand_hex(5)));
        let records = Records::new(root.clone());
        let reviewed = root.join(CLIPS).join("pos").join("large_r.wav");
        fs::create_dir_all(reviewed.parent().unwrap()).unwrap();
        fs::File::create(&reviewed).unwrap().set_len(5 << 20).unwrap();

        records.save("near", &[241; 16_000], 0.49, 50, false).unwrap();

        let snapshot = records.snapshot(true, 50, false);
        assert_eq!(snapshot["limit_mb"], 50);
        assert_eq!(snapshot["local"], 2);
        assert!(reviewed.is_file());
        assert_eq!(records.snapshot(true, 51, false)["limit_mb"], 50);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exports_existing_and_new_recordings_with_labels_and_model() {
        let root = std::env::temp_dir().join(format!("jarvis-dataset-{}", rand_hex(5)));
        let records = Records::new(root.clone());
        let old = root.join(OLD_CLIPS).join("near").join("legacy.wav");
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, vec![0; 1 << 20]).unwrap();
        let pcm: Vec<_> = (0..16000).map(|i| (i % 100) as i16).collect();
        records.save("near", &pcm, 0.49, 1, false).unwrap();
        assert!(old.is_file());
        let clip = records.list_dir(CLIPS).pop().unwrap();
        assert!(!clip.reviewed);
        assert!(records.read("near", "../wake.bin").is_err());
        records.move_clip("near", &clip.name, "pos").unwrap();
        let reviewed = records.list_dir(CLIPS).pop().unwrap();
        assert!(reviewed.reviewed);
        assert_eq!(&records.read("pos", &reviewed.name).unwrap()[..4], b"RIFF");
        fs::write(root.join("wake.bin"), b"model").unwrap();
        let mut data = Vec::new();
        records.archive(&root, 8, &mut data).unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(data)).unwrap();
        assert!(zip.by_name("finetune.py").is_ok());
        assert!(zip.by_name("near/legacy.wav").is_ok());
        assert!(zip.by_name(&format!("pos/{}", reviewed.name)).is_ok());
        assert!(zip.by_name("wake.bin").is_ok());
        let raw = pcm_wav(&pcm);
        fs::write(&old, &raw).unwrap();
        assert_eq!(records.compact().unwrap()["changed"], 1);
        assert_eq!(records.compact().unwrap()["changed"], 0);
        assert_eq!(records.read("near", "legacy.wav").unwrap(), raw);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn exports_nearly_fifty_mib_of_recordings_as_one_zip() {
        let root = std::env::temp_dir().join(format!("jarvis-dataset-{}", rand_hex(5)));
        let records = Records::new(root.clone());
        let clips = root.join(CLIPS).join("pos");
        fs::create_dir_all(&clips).unwrap();
        fs::write(root.join("wake.bin"), b"model").unwrap();

        let mut wav = pcm_wav(&vec![0; 11 << 18]);
        let compressed = compress(&wav).unwrap();
        fs::write(clips.join("compressed.wav"), &compressed).unwrap();
        getrandom::fill(&mut wav[44..]).unwrap();
        for index in 0..9 {
            fs::write(clips.join(format!("clip-{index}.wav")), &wav).unwrap();
        }
        let stored = wav.len() * 9 + compressed.len();
        let limit = (MAX_LIMIT_MB as usize) << 20;
        assert!(((49 << 20)..limit).contains(&stored));
        assert!(wav.len() * 10 > limit);

        let path = root.join("export.zip");
        records
            .archive(&root, 8, fs::File::create(&path).unwrap())
            .unwrap();
        assert!(fs::metadata(&path).unwrap().len() > 32 << 20);
        let mut zip = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(zip.len(), 13);
        for name in ["wake.bin", "wake.json", "finetune.py"] {
            assert!(zip.by_name(name).is_ok());
        }
        let mut extracted = Vec::new();
        for index in 0..9 {
            extracted.clear();
            zip.by_name(&format!("pos/clip-{index}.wav"))
                .unwrap()
                .read_to_end(&mut extracted)
                .unwrap();
            assert_eq!(extracted, wav);
        }
        extracted.clear();
        zip.by_name("pos/compressed.wav")
            .unwrap()
            .read_to_end(&mut extracted)
            .unwrap();
        assert_eq!(extracted.len(), wav.len());
        assert_eq!(&extracted[..44], &wav[..44]);
        assert!(extracted[44..].iter().all(|&byte| byte == 0));
        drop(zip);
        fs::remove_dir_all(root).unwrap();
    }
}
