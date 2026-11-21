use super::{Host, Manifest};
use jarvis_plugin_sdk::process::rpc::{PROTOCOL, Peer};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Stdio,
    sync::{Arc, Weak},
    time::Duration,
};
use tokio::{process::Command, sync::watch};

pub struct Official {
    peer: Peer,
    exited: watch::Receiver<bool>,
}

impl Official {
    pub async fn start(
        manifest: Manifest,
        dir: &Path,
        host: Weak<Host>,
    ) -> Result<Arc<Self>, String> {
        let exe = dir.join(&manifest.main);
        if !exe.symlink_metadata().is_ok_and(|entry| entry.is_file()) {
            return Err("Бинарник плагина отсутствует".into());
        }
        let exe = exe.canonicalize().map_err(|error| error.to_string())?;
        let data = dir.join("data");
        let core = host.upgrade().ok_or("Ядро завершилось")?;
        let core_data = core
            .data_dir
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let info = (core.hooks.info)();
        drop(core);
        tokio::fs::create_dir_all(&data)
            .await
            .map_err(|error| error.to_string())?;
        let mut child = Command::new(exe)
            .current_dir(data)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("Не удалось запустить {}: {error}", manifest.name))?;
        let input = child.stdin.take().ok_or("Нет stdin плагина")?;
        let output = child.stdout.take().ok_or("Нет stdout плагина")?;
        let id = manifest.id.clone();
        let exit_host = host.clone();
        let peer = Peer::open(
            output,
            input,
            Box::new(move |_, method, params| {
                let host = host.clone();
                let manifest = manifest.clone();
                Box::pin(async move {
                    let host = host.upgrade().ok_or("Ядро завершилось")?;
                    if !host.official(&manifest.id) {
                        return Err("Требуется официальный плагин".into());
                    }
                    if matches!(
                        (manifest.id.as_str(), method.as_str()),
                        ("chatgpt", "voice.event")
                            | (
                                "youtube",
                                "music.state" | "music.play" | "music.related_end"
                            )
                    ) {
                        (host.hooks.event)(&manifest, &method, params);
                        return Ok(Value::Null);
                    }
                    (host.hooks.request)(manifest.id, method, params).await
                })
            }),
        );
        let (done, exited) = watch::channel(false);
        let link = peer.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = child.wait() => {}
                _ = link.closed() => { let _ = child.kill().await; }
            }
            link.close();
            done.send_replace(true);
            if let Some(host) = exit_host.upgrade() {
                host.refresh_wake_training();
            }
        });
        let handshake = peer
            .call(
                "init",
                json!({"api":PROTOCOL,"id":id,"info":info,"core_data":core_data}),
                Duration::from_secs(5),
            )
            .await;
        match handshake {
            Ok(reply) if reply["api"] == PROTOCOL => Ok(Arc::new(Self { peer, exited })),
            _ => {
                peer.close();
                Err(handshake
                    .err()
                    .unwrap_or_else(|| "Несовместимый API Rust-плагина".into()))
            }
        }
    }

    pub fn running(&self) -> bool {
        self.peer.running()
    }

    pub async fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.peer.call(method, params, timeout).await
    }

    pub async fn stop(&self) {
        let _ = self
            .request("shutdown", Value::Null, Duration::from_millis(600))
            .await;
        self.peer.close();
        let _ = self.exited.clone().wait_for(|done| *done).await;
    }
}
