//! Звук колонки: микрофон — обработанный канал vqe_0 (эхо- и шумоподавление Яндекса) через наш
//! vqe_tap.so в maind; динамик — aplay в тот же ALSA-микшер, что и у Алисы.

use jarvis_dsp::audio::{Limiter, OUT_RATE, Upsampler, gain_pcm};
use std::{
    collections::HashMap,
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TrySendError, sync_channel},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const MIC_RATE: u32 = 16000;

pub type Frame = Arc<[i16]>;

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Микрофон: датаграммы PCM s16le 16 кГц от vqe_tap.so, раздаются подписчикам.
pub struct Mic {
    path: PathBuf,
    subs: Mutex<HashMap<u64, SyncSender<Frame>>>,
    next: AtomicU64,
    seen: AtomicU64,
    /// Общее усиление (дБ) для всех подписчиков: слово, ИИ, записи для обучения.
    pub gain_db: Box<dyn Fn() -> i64 + Send + Sync>,
}

impl Mic {
    pub fn new(path: PathBuf, gain_db: Box<dyn Fn() -> i64 + Send + Sync>) -> Arc<Mic> {
        Arc::new(Mic { path, subs: Mutex::default(), next: AtomicU64::new(0), seen: AtomicU64::new(0), gain_db })
    }

    /// Подписаться: кадры по ~10 мс; медленный подписчик теряет кадры, а не тормозит остальных.
    pub fn subscribe(&self, buf: usize) -> (u64, Receiver<Frame>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = sync_channel(buf);
        self.subs.lock().unwrap().insert(id, tx);
        (id, rx)
    }

    pub fn unsubscribe(&self, id: u64) {
        self.subs.lock().unwrap().remove(&id);
    }

    /// Микрофон на связи: кадр был за последние 3 с.
    pub fn online(&self) -> bool {
        now_ms().saturating_sub(self.seen.load(Ordering::Relaxed)) < 3000
    }

    /// Раздать кадр (из сокета или для тестов).
    pub fn publish(&self, mut pcm: Vec<i16>) {
        gain_pcm(&mut pcm, (self.gain_db)());
        let f: Frame = pcm.into();
        self.seen.store(now_ms(), Ordering::Relaxed);
        let mut subs = self.subs.lock().unwrap();
        subs.retain(|_, tx| !matches!(tx.try_send(f.clone()), Err(TrySendError::Disconnected(_))));
    }

    /// Слушать сокет (в своём потоке): упал — повтор через 3 с.
    pub fn run(self: Arc<Self>) {
        thread::Builder::new()
            .name("mic".into())
            .stack_size(128 * 1024)
            .spawn(move || {
                loop {
                    if let Err(e) = self.serve() {
                        log::info!("микрофон: {e} — повтор через 3 с");
                    }
                    thread::sleep(Duration::from_secs(3));
                }
            })
            .expect("поток микрофона");
    }

    #[cfg(unix)]
    fn serve(&self) -> std::io::Result<()> {
        use std::os::unix::{fs::PermissionsExt, net::UnixDatagram};
        if let Some(d) = self.path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::remove_file(&self.path);
        let sock = UnixDatagram::bind(&self.path)?;
        // maind пишет от root, но на других прошивках может быть другой пользователь
        let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o666));
        let mut buf = [0u8; 4096];
        loop {
            let n = sock.recv(&mut buf)?;
            if n < 2 {
                continue;
            }
            let pcm = buf[..n / 2 * 2].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
            self.publish(pcm);
        }
    }

    #[cfg(not(unix))]
    fn serve(&self) -> std::io::Result<()> {
        Err(std::io::Error::other("микрофон есть только на колонке"))
    }
}

/// Усиление голоса Джарвиса при громкости 100% (голос моделей приходит тихим, около −22 дБ):
/// ×6,5 с ограничителем даёт речь около −8 дБ — чуть громче Алисы.
const PLAY_GAIN: f64 = 6.5;

struct Playing {
    child: Child,
    queue: Option<SyncSender<Vec<u8>>>,
    pump: Option<thread::JoinHandle<()>>,
    lim: Limiter,
    up: Option<Upsampler>,
    boost: f64,
    shaped: Vec<i16>,
    up_buf: Vec<i16>,
}

/// Плеер: один ответ = один процесс aplay. Стоп — SIGINT: aplay сам корректно отцепляется от dmix.
pub struct Player {
    cur: Mutex<Option<Playing>>,
    aplay: String,
    /// 0..100, читается на каждом куске.
    pub volume: Box<dyn Fn() -> i64 + Send + Sync>,
    /// Поправка к громкости голоса от громкости самой колонки (1 — без поправки).
    pub trim: Box<dyn Fn() -> f64 + Send + Sync>,
}

impl Player {
    pub fn new(volume: Box<dyn Fn() -> i64 + Send + Sync>, trim: Box<dyn Fn() -> f64 + Send + Sync>) -> Arc<Player> {
        let aplay = if std::path::Path::new("/usr/bin/aplay").exists() { "/usr/bin/aplay" } else { "aplay" };
        Arc::new(Player { cur: Mutex::default(), aplay: aplay.into(), volume, trim })
    }

    pub fn playing(&self) -> bool {
        self.cur.lock().unwrap().is_some()
    }

    /// Открыть плеер; rate — частота PCM (моно s16le).
    pub fn start(&self, rate: usize) -> std::io::Result<()> {
        self.stop(true);
        let (up, out) = if rate > 0 && rate < OUT_RATE && OUT_RATE % rate == 0 { (Some(Upsampler::new(OUT_RATE / rate)), OUT_RATE) } else { (None, rate) };
        let mut child = Command::new(&self.aplay)
            .args(["-q", "-D", "default", "-t", "raw", "-f", "S16_LE", "-r", &out.to_string(), "-c", "1"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().unwrap();
        let (tx, rx) = sync_channel::<Vec<u8>>(512);
        let pump = thread::Builder::new().name("aplay-pump".into()).stack_size(128 * 1024).spawn(move || pump(stdin, rx))?;
        *self.cur.lock().unwrap() = Some(Playing { child, queue: Some(tx), pump: Some(pump), lim: Limiter::new(rate), up, boost: 1.0, shaped: Vec::new(), up_buf: Vec::new() });
        Ok(())
    }

    /// Доп. усиление голоса конкретного движка (ChatGPT по подписке приходит тише).
    pub fn set_boost(&self, b: f64) {
        if let Some(p) = self.cur.lock().unwrap().as_mut() {
            p.boost = b;
        }
    }

    /// Кусок PCM; не блокирует (если aplay завис, ответ теряется, а не встаёт всё).
    pub fn write(&self, pcm: &[i16]) -> Result<(), &'static str> {
        let mut g = (self.volume)() as f64 / 50.0 * PLAY_GAIN; // 50% — прежний максимум, 100% — вдвое громче
        g *= (self.trim)();
        let mut cur = self.cur.lock().unwrap();
        let p = cur.as_mut().ok_or("плеер не запущен")?;
        if p.boost > 0.0 {
            g *= p.boost;
        }
        let g = g as f32;
        p.shaped.clear();
        p.shaped.reserve(pcm.len());
        for &s in pcm {
            p.shaped.push(p.lim.next_f32(s as f32 * g));
        }
        let src: &[i16] = if let Some(up) = p.up.as_mut() {
            up.run_into(&p.shaped, &mut p.up_buf);
            &p.up_buf
        } else {
            &p.shaped
        };
        let mut b = Vec::with_capacity(src.len() * 2);
        for &s in src {
            b.extend_from_slice(&s.to_le_bytes());
        }
        let q = p.queue.as_ref().ok_or("плеер закрыт")?;
        q.try_send(b).map_err(|_| "плеер не успевает")
    }

    /// Доиграть то, что уже отдано, и закрыть.
    pub fn finish(&self, timeout: Duration) {
        let Some(mut p) = self.cur.lock().unwrap().take() else { return };
        p.queue.take(); // закрыть очередь: насос допишет и закроет stdin
        let t0 = Instant::now();
        if let Some(h) = p.pump.take() {
            while !h.is_finished() && t0.elapsed() < timeout {
                thread::sleep(Duration::from_millis(20));
            }
        }
        wait_child(&mut p.child, timeout.saturating_sub(t0.elapsed()).max(Duration::from_millis(200)));
    }

    /// Оборвать сразу (now) или дать доиграть буфер ALSA.
    pub fn stop(&self, now: bool) {
        let Some(mut p) = self.cur.lock().unwrap().take() else { return };
        p.queue.take();
        if now {
            interrupt(&p.child);
        }
        wait_child(&mut p.child, Duration::from_secs(2));
    }
}

fn pump(mut stdin: ChildStdin, rx: Receiver<Vec<u8>>) {
    use std::io::Write;
    for b in rx {
        if stdin.write_all(&b).is_err() {
            return; // плеер умер — остаток выбрасываем (rx закроется вместе с потоком)
        }
    }
}

fn interrupt(child: &Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGINT);
    }
    #[cfg(not(unix))]
    let _ = child;
}

fn wait_child(child: &mut Child, timeout: Duration) {
    let t0 = Instant::now();
    loop {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        if t0.elapsed() > timeout {
            interrupt(child);
            thread::sleep(Duration::from_secs(1));
            if !matches!(child.try_wait(), Ok(Some(_))) {
                let _ = child.kill();
                let _ = child.wait();
            }
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Отправить WAV (16 кГц моно) в сокет микрофона кусками по 10 мс в реальном времени — для проверки на колонке.
#[cfg(unix)]
pub fn send_wav(sock: &std::path::Path, wav: &std::path::Path) -> std::io::Result<()> {
    let raw = std::fs::read(wav)?;
    let pcm = if raw.len() > 44 && &raw[..4] == b"RIFF" { &raw[44..] } else { &raw[..] };
    let s = std::os::unix::net::UnixDatagram::unbound()?;
    let t0 = Instant::now();
    for (i, c) in pcm.chunks(320).enumerate() {
        let due = t0 + Duration::from_millis(10 * i as u64);
        if let Some(d) = due.checked_duration_since(Instant::now()) {
            thread::sleep(d);
        }
        s.send_to(c, sock)?;
    }
    Ok(())
}
