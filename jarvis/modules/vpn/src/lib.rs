//! Optional in-process VLESS client. No subprocesses, system proxy, TUN or changes to host routing.
//! TCP/WebSocket, TLS, and authenticated REALITY with optional XTLS Vision on TCP.
mod config;
mod chrome;
mod client_hello;
mod firefox;
mod protocol;
mod reality;
#[cfg(test)]
mod tests;
mod tls_io;
mod transport;
mod udp;
mod vision;
mod websocket;

use config::Config;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::copy_bidirectional,
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};

pub struct Vpn {
    apply_lock: tokio::sync::Mutex<()>,
    active: Mutex<Option<Live>>,
    stopped: AtomicBool,
}
struct Live {
    raw: String,
    state: Value,
    task: JoinHandle<()>,
    cancel: tokio::sync::watch::Sender<bool>,
}
impl Drop for Live {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Live {
    async fn close(mut self) {
        self.cancel.send_replace(true);
        let _ = (&mut self.task).await;
    }
}
impl Default for Vpn {
    fn default() -> Self {
        Self::new()
    }
}
impl Vpn {
    pub fn new() -> Self {
        Self {
            apply_lock: tokio::sync::Mutex::new(()),
            active: Mutex::new(None),
            stopped: AtomicBool::new(false),
        }
    }

    /// Validate a replacement without changing the active tunnel or opening network sockets.
    pub fn validate(params: Value) -> Result<Value, String> {
        let raw = params
            .get("link")
            .and_then(Value::as_str)
            .ok_or("VPN: нужен параметр link")?
            .trim();
        if raw.is_empty() {
            return Ok(json!({}));
        }
        Ok(Config::parse(raw)?.info())
    }

    pub async fn request(&self, params: Value) -> Result<Value, String> {
        let raw = params
            .get("link")
            .and_then(Value::as_str)
            .ok_or("VPN: нужен параметр link")?
            .trim();
        let _apply = self.apply_lock.lock().await;
        if self.stopped.load(Ordering::SeqCst) {
            return Err("модуль VPN остановлен".into());
        }
        let old = {
            let mut active = self.active.lock().unwrap();
            if let Some(live) = active.as_ref() {
                if live.raw == raw && !live.task.is_finished() {
                    return Ok(live.state.clone());
                }
            }
            // Even a rejected replacement must retire the old tunnel and all its existing streams.
            active.take()
        };
        if let Some(live) = old {
            live.close().await;
        }
        if raw.is_empty() {
            return Ok(json!({}));
        }
        let config = Arc::new(Config::parse(raw)?);
        let tls = transport::tls_config(&config)?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| "Не открыть локальный порт VPN")?;
        let addr = listener
            .local_addr()
            .map_err(|_| "Не получить локальный порт VPN")?;
        let secret = || -> Result<String, String> {
            let mut bytes = [0; 16];
            getrandom::fill(&mut bytes).map_err(|_| "Не получить случайный пароль VPN")?;
            Ok(hex::encode(bytes))
        };
        let user = secret()?;
        let pass = secret()?;
        let state = json!({"socks":addr.to_string(),"user":user,"pass":pass,"udp":true,"info":config.info()});
        let mut active = self.active.lock().unwrap();
        if self.stopped.load(Ordering::SeqCst) {
            return Err("модуль VPN остановлен".into());
        }
        let (cancel, receive) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(serve(
            listener,
            config,
            tls,
            user.into_bytes(),
            pass.into_bytes(),
            receive,
        ));
        *active = Some(Live {
            raw: raw.to_string(),
            state: state.clone(),
            task,
            cancel,
        });
        Ok(state)
    }

    pub async fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        let live = self.active.lock().unwrap().take();
        if let Some(live) = live {
            live.close().await;
        }
    }
}

async fn serve(
    listener: TcpListener,
    config: Arc<Config>,
    tls: Arc<reality_tls::ClientConfig>,
    user: Vec<u8>,
    pass: Vec<u8>,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) {
    let credentials = Arc::new((user, pass));
    let udp_limit = Arc::new(tokio::sync::Semaphore::new(32));
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = cancel.changed() => break,
            _ = connections.join_next(), if !connections.is_empty() => {},
            accepted = listener.accept() => {
                let Ok((socket, _)) = accepted else { break };
                if connections.len() >= 64 { drop(socket); continue; }
                connections.spawn(connection(socket, config.clone(), tls.clone(), credentials.clone(), udp_limit.clone()));
            }
        }
    }
    drop(listener);
    connections.shutdown().await;
}

async fn connection(
    mut socket: TcpStream,
    config: Arc<Config>,
    tls: Arc<reality_tls::ClientConfig>,
    credentials: Arc<(Vec<u8>, Vec<u8>)>,
    udp_limit: Arc<tokio::sync::Semaphore>,
) {
    let Ok(Ok(request)) = tokio::time::timeout(Duration::from_secs(15),
        protocol::socks_handshake(&mut socket, &credentials.0, &credentials.1)).await else { return; };
    let target = match request {
        protocol::Request::Associate(target) => {
            let _ = udp::associate(socket, target, config, tls, udp_limit).await;
            return;
        }
        protocol::Request::Connect(target) => target,
    };
    let setup = async {
        let upstream = match transport::dial(&config, tls, &target).await {
            Ok(io) => io,
            Err(e) => {
                let _ = protocol::socks_reply(&mut socket, 5).await;
                return Err(e);
            }
        };
        protocol::socks_reply(&mut socket, 0).await?;
        Ok::<_, std::io::Error>(upstream)
    };
    let Ok(Ok(mut upstream)) = tokio::time::timeout(Duration::from_secs(15), setup).await else {
        return;
    };
    let _ = copy_bidirectional(&mut socket, &mut upstream).await;
}
