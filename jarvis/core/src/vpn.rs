//! VPN по подключениям: туннель держит плагин «VPN» (локальный SOCKS5 с паролем), ядро только знает,
//! куда ходить. Подключение с галочкой «через VPN» при недоступном туннеле НИКОГДА не идёт напрямую.

use crate::{plugins::Host, settings::Store};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    sync::{Arc, RwLock},
    time::Duration,
};

pub const VPN_MISSING: &str = "VPN теперь отдельный плагин: поставьте «VPN» на странице «Плагины»";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Tunnel {
    pub socks: String,
    pub user: String,
    pub pass: String,
    pub udp: bool,
    pub info: Option<Value>,
}

impl Tunnel {
    fn valid(&self) -> bool {
        self.socks
            .parse::<SocketAddr>()
            .is_ok_and(|a| a.ip().is_loopback() && a.port() > 0)
            && (1..=255).contains(&self.user.len())
            && (1..=255).contains(&self.pass.len())
    }
}

#[derive(Default)]
struct St {
    link: String,
    tun: Option<Tunnel>,
    err: String,
    cleared: bool,
}

pub struct Proxy {
    st: RwLock<St>,
    store: Store,
    refresh_lock: tokio::sync::Mutex<()>,
    notify: tokio::sync::Notify,
}

/// Куда ходить подключению: напрямую, через туннель или никуда (с причиной).
pub enum Route {
    Direct,
    Tunnel(Tunnel),
    Blocked(String),
}

impl Proxy {
    pub fn new(store: Store) -> Arc<Proxy> {
        Arc::new(Proxy {
            st: RwLock::default(),
            store,
            refresh_lock: tokio::sync::Mutex::new(()),
            notify: tokio::sync::Notify::new(),
        })
    }

    pub fn route(&self, svc: &str) -> Route {
        let (use_vpn, link) = self.store.with(|s| {
            (
                svc.is_empty() || crate::web::state::use_vpn(s, svc),
                s.vless.trim().to_string(),
            )
        });
        let st = self.st.read().unwrap();
        if !use_vpn || link.is_empty() {
            return Route::Direct;
        }
        if st.link != link {
            return Route::Blocked("VPN применяет настройки".into());
        }
        match &st.tun {
            Some(t) => Route::Tunnel(t.clone()),
            None => Route::Blocked(if st.err.is_empty() {
                "плагин VPN запускается".into()
            } else {
                st.err.clone()
            }),
        }
    }

    /// Ответ сервиса ядра «network» для плагина: {} — напрямую, {socks,user,pass} — через туннель, {error}.
    pub fn network_reply(&self, svc: &str) -> Value {
        match self.route(svc) {
            Route::Direct => json!({}),
            Route::Tunnel(t) => json!({"socks": t.socks, "user": t.user, "pass": t.pass, "udp":t.udp}),
            Route::Blocked(why) => json!({"error": why}),
        }
    }

    pub fn status(&self) -> (Option<Value>, String) {
        let st = self.st.read().unwrap();
        (st.tun.as_ref().and_then(|t| t.info.clone()), st.err.clone())
    }

    /// Свериться со ссылкой из настроек и плагином VPN.
    pub async fn refresh(&self, host: &Host) {
        let _lock = self.refresh_lock.lock().await;
        self.refresh_inner(host).await;
    }

    /// Проверка принадлежит провайдеру; неподдержанная ссылка не заменяет рабочую.
    pub async fn set_link(&self, host: &Host, link: &str) -> Result<(), String> {
        let _lock = self.refresh_lock.lock().await;
        let link = link.trim();
        if !link.is_empty() {
            vpn_provider(host)?
                .request("vpn.validate", json!({"link":link}), Duration::from_secs(5))
                .await?;
        }
        self.store
            .update(|s| -> Result<(), std::io::Error> {
                s.vless = link.to_string();
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        if link.is_empty() {
            self.st.write().unwrap().cleared = false;
        }
        self.refresh_inner(host).await;
        self.notify.notify_one();
        Ok(())
    }

    async fn refresh_inner(&self, host: &Host) {
        let link = self.store.with(|s| s.vless.trim().to_string());
        if link.is_empty()
            && self.st.read().unwrap().cleared
        {
            return;
        }
        let (tun, err, cleared) = if link.is_empty() {
            // Clearing the link also closes the module's listener and active streams.
            if let Ok(provider) = vpn_provider(host) {
                let _ = provider
                    .request("vpn", json!({"link":""}), Duration::from_secs(5))
                    .await;
            }
            (None, String::new(), true)
        } else {
            match vpn_provider(host) {
                Err(why) => (None, why, false),
                Ok(n) => match n
                    .request("vpn", json!({"link": link}), Duration::from_secs(5))
                    .await
                {
                    Ok(v) => match serde_json::from_value::<Tunnel>(v) {
                        Ok(t) if t.valid() => (Some(t), String::new(), false),
                        _ => (
                            None,
                            "плагин VPN вернул неправильный локальный SOCKS".into(),
                            false,
                        ),
                    },
                    Err(e) => (None, e, false),
                },
            }
        };
        let mut st = self.st.write().unwrap();
        st.link = link;
        st.tun = tun;
        st.err = err;
        st.cleared = cleared;
    }

    pub async fn run(self: Arc<Self>, host: Arc<Host>) {
        loop {
            self.refresh(&host).await;
            let delay = {
                let st = self.st.read().unwrap();
                if st.link.is_empty() {
                    Duration::from_secs(60)
                } else if st.tun.is_some() {
                    Duration::from_secs(30)
                } else {
                    Duration::from_secs(3)
                }
            };
            tokio::select! {
                _ = self.notify.notified() => {}
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }
}

/// Включённый плагин с provides "vpn"; официальный «vpn» побеждает.
fn vpn_provider(host: &Host) -> Result<crate::plugins::Runtime, String> {
    let g = host.inner.lock().unwrap();
    let dedicated = g.plugins.contains_key("vpn") || g.state.get("vpn").is_some_and(|s| s.removed);
    let mut chosen: Option<&crate::plugins::Plugin> = None;
    let mut disabled = false;
    for p in g.plugins.values().filter(|p| p.manifest.provides("vpn")) {
        if dedicated && p.manifest.id != "vpn" {
            continue;
        }
        let st = g.state.get(&p.manifest.id);
        let on = st.is_some_and(|s| s.enabled && !s.removed);
        if !on {
            disabled |= st.is_some_and(|s| !s.removed);
            continue;
        }
        if chosen.is_none_or(|c| {
            p.manifest.id == "vpn" || (c.manifest.id != "vpn" && p.manifest.id < c.manifest.id)
        }) {
            chosen = Some(p);
        }
    }
    let Some(p) = chosen else {
        return Err(if disabled {
            "Плагин VPN выключен — включите его на странице «Плагины»".into()
        } else {
            VPN_MISSING.into()
        });
    };
    p.rt.as_ref()
        .filter(|runtime| runtime.is_service())
        .cloned()
        .ok_or_else(|| "плагин VPN запускается".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "vpn")]
    #[tokio::test]
    async fn rust_module_route_settings_and_plugin_shutdown() {
        use tokio::io::AsyncReadExt;
        tokio::time::timeout(Duration::from_secs(5), async {
            let dir =
                std::env::temp_dir().join(format!("jarvis-vpn-{}", crate::settings::rand_hex(6)));
            std::fs::create_dir_all(&dir).unwrap();
            let store = Store::open(dir.join("settings.json")).unwrap();
            let host = Host::new(dir.clone(), crate::plugins::Hooks::default());
            host.install_module("vpn").await.unwrap();
            let proxy = Proxy::new(store.clone());
            let link = "vless://00112233-4455-6677-8899-aabbccddeeff@127.0.0.1:443";
            proxy.set_link(&host, link).await.unwrap();
            let Route::Tunnel(tunnel) = proxy.route("openai") else {
                panic!("Rust tunnel must be selected");
            };
            let mut pending = tokio::net::TcpStream::connect(&tunnel.socks).await.unwrap();
            assert!(
                proxy
                    .set_link(&host, &format!("{link}?security=reality"))
                    .await
                    .unwrap_err()
                    .contains("REALITY")
            );
            assert_eq!(store.with(|s| s.vless.clone()), link);
            let Route::Tunnel(same) = proxy.route("openai") else {
                panic!()
            };
            assert_eq!(same, tunnel);
            proxy.set_link(&host, "").await.unwrap();
            assert!(matches!(proxy.route("openai"), Route::Direct));
            assert!(matches!(pending.read_u8().await, Err(_)));
            assert!(tokio::net::TcpStream::connect(&tunnel.socks).await.is_err());
            proxy.set_link(&host, link).await.unwrap();
            let Route::Tunnel(tunnel) = proxy.route("openai") else {
                panic!()
            };
            let mut pending = tokio::net::TcpStream::connect(&tunnel.socks).await.unwrap();
            host.set_enabled("vpn", false).await.unwrap();
            assert!(pending.read_u8().await.is_err());
            assert!(tokio::net::TcpStream::connect(&tunnel.socks).await.is_err());
            proxy.refresh(&host).await;
            assert!(matches!(proxy.route("openai"), Route::Blocked(_)));
            assert!(proxy.set_link(&host, link).await.is_err());
            host.uninstall("vpn").await.unwrap();
            assert!(vpn_provider(&host).is_err());
            proxy.set_link(&host, "").await.unwrap();
            assert_eq!(dir.parent(), Some(std::env::temp_dir().as_path()));
            assert!(
                dir.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("jarvis-vpn-")
            );
            std::fs::remove_dir_all(dir).unwrap();
        })
        .await
        .unwrap();
    }
    #[test]
    fn changed_or_starting_vpn_never_uses_direct_or_old_route() {
        let dir = std::env::temp_dir().join(format!("jarvis-vpn-{}", crate::settings::rand_hex(6)));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(dir.join("settings.json")).unwrap();
        store
            .update(|s| -> Result<(), std::io::Error> {
                s.vless = "vless://first".into();
                s.vpn.insert("yandex".into(), true);
                Ok(())
            })
            .unwrap();
        let proxy = Proxy::new(store.clone());
        assert!(matches!(proxy.route("yandex"), Route::Blocked(_)));
        {
            let mut st = proxy.st.write().unwrap();
            st.link = "vless://first".into();
            st.tun = Some(Tunnel {
                socks: "127.0.0.1:1080".into(),
                user: "u".into(),
                pass: "p".into(),
                udp: true,
                info: None,
            });
        }
        assert!(matches!(proxy.route("yandex"), Route::Tunnel(_)));
        store
            .update(|s| -> Result<(), std::io::Error> {
                s.vless = "vless://second".into();
                Ok(())
            })
            .unwrap();
        assert!(matches!(proxy.route("yandex"), Route::Blocked(_)));
        store
            .update(|s| -> Result<(), std::io::Error> {
                s.vpn.insert("yandex".into(), false);
                Ok(())
            })
            .unwrap();
        assert!(matches!(proxy.route("yandex"), Route::Direct));
        store
            .update(|s| -> Result<(), std::io::Error> {
                s.vpn.insert("yandex".into(), true);
                s.vless.clear();
                Ok(())
            })
            .unwrap();
        assert!(matches!(proxy.route("yandex"), Route::Direct));
        assert_eq!(dir.parent(), Some(std::env::temp_dir().as_path()));
        assert!(
            dir.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("jarvis-vpn-")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
