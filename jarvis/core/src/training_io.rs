use crate::plugins::Host;
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub const SAMPLES: usize = 16_000 * 16 / 5;
pub const BODY_BYTES: usize = 13 * SAMPLES * 2;

pub async fn call(
    host: &Host,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, String> {
    host.provider("training")
        .map_err(|_| "Включите плагин «Обучение слова»")?
        .request(method, params, timeout)
        .await
}

pub struct Transfer(PathBuf);

impl Transfer {
    pub fn new() -> Result<Self, String> {
        let path =
            std::env::temp_dir().join(format!("jarvis-transfer-{}", crate::settings::rand_hex(12)));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path).map_err(|error| error.to_string())?;
        Ok(Self(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Transfer {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(unix)]
pub struct ArchiveSocket(PathBuf);

#[cfg(unix)]
impl ArchiveSocket {
    pub fn bind() -> std::io::Result<(Self, tokio::net::UnixListener)> {
        use std::os::unix::fs::DirBuilderExt;

        let directory = std::env::temp_dir()
            .join(format!("jarvis-archive-{}", crate::settings::rand_hex(12)));
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let socket = Self(directory.join("zip.sock"));
        let listener = tokio::net::UnixListener::bind(socket.path())?;
        Ok((socket, listener))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

#[cfg(unix)]
impl Drop for ArchiveSocket {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.path());
        if let Some(directory) = self.path().parent() {
            let _ = fs::remove_dir(directory);
        }
    }
}

#[derive(Default)]
pub struct AudioBatch {
    pcm: Vec<i16>,
    hit: bool,
    discard: bool,
    prob: f32,
}

impl AudioBatch {
    pub fn reset(&mut self) {
        self.pcm.clear();
        self.hit = false;
        self.discard = false;
        self.prob = 0.0;
    }

    pub fn push(
        &mut self,
        pcm: &[i16],
        hit: bool,
        discard: bool,
        prob: f32,
        collect: bool,
        limit: i64,
    ) -> Option<Value> {
        self.hit |= hit;
        self.discard |= discard;
        self.prob = self.prob.max(prob);
        self.pcm.extend_from_slice(pcm);
        if self.pcm.len() < 3_200 && !hit {
            return None;
        }
        let pcm = B64.encode(
            self.pcm
                .iter()
                .flat_map(|sample| sample.to_le_bytes())
                .collect::<Vec<_>>(),
        );
        let value = json!({"pcm":pcm,"hit":self.hit,"discard":self.discard,"prob":self.prob,"collect":collect,"limit":limit});
        self.reset();
        Some(value)
    }

    pub fn channel(
        rt: &tokio::runtime::Handle,
        host: Arc<Host>,
    ) -> tokio::sync::mpsc::Sender<Value> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        rt.spawn(async move {
            while let Some(frame) = rx.recv().await {
                let _ = call(&host, "training.observe", frame, Duration::from_secs(2)).await;
            }
        });
        tx
    }
}
