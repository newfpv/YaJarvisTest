use percent_encoding::percent_decode_str;
use serde_json::{Value, json};
use std::{collections::BTreeMap, net::IpAddr};

// Do not derive Debug: the UUID and URL query may contain credentials.
pub(crate) struct Config {
    pub host: String,
    pub port: u16,
    pub uuid: [u8; 16],
    pub tls: bool,
    pub insecure: bool,
    pub server_name: String,
    pub websocket: bool,
    pub ws_host: String,
    pub ws_path: String,
    pub name: String,
    pub reality: Option<Reality>,
    pub vision: bool,
    pub flow: String,
    pub fingerprint: Fingerprint,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fingerprint {
    Rustls,
    Chrome133,
    Firefox120,
}

impl Fingerprint {
    fn name(self) -> &'static str {
        match self {
            Self::Rustls => "rustls",
            Self::Chrome133 => "chrome133",
            Self::Firefox120 => "firefox120",
        }
    }
}
pub(crate) struct Reality {
    pub public_key: [u8; 32],
    pub short_id: [u8; 8],
}

impl Config {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let invalid = || "Неверная ссылка VLESS".to_string();
        if raw.len() > 16 * 1024 {
            return Err(invalid());
        }
        let url = url::Url::parse(raw).map_err(|_| invalid())?;
        if url.scheme() != "vless" || url.password().is_some() || !matches!(url.path(), "" | "/") {
            return Err(invalid());
        }
        let host = match url.host().ok_or_else(invalid)? {
            url::Host::Ipv6(ip) => ip.to_string(),
            h => h.to_string(),
        };
        if host.is_empty()
            || host.len() > 253
            || host.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(invalid());
        }
        let port = url
            .port()
            .filter(|p| *p > 0)
            .ok_or("В ссылке VLESS нужен порт 1–65535")?;
        let user = percent_decode_str(url.username())
            .decode_utf8()
            .map_err(|_| invalid())?;
        let uuid_text =
            if user.len() == 36 && [8, 13, 18, 23].iter().all(|i| user.as_bytes()[*i] == b'-') {
                user.replace('-', "")
            } else {
                user.to_string()
            };
        let mut uuid = [0; 16];
        hex::decode_to_slice(&uuid_text, &mut uuid)
            .map_err(|_| "В ссылке VLESS нужен UUID".to_string())?;
        let mut query = BTreeMap::new();
        for (key, value) in url.query_pairs() {
            if query.insert(key.into_owned(), value.into_owned()).is_some() {
                return Err("Повторяющийся параметр VLESS".into());
            }
        }
        let get = |k: &str| query.get(k).map(String::as_str).unwrap_or("");
        if !matches!(get("encryption"), "" | "none") {
            return Err("Rust VPN пока поддерживает только encryption=none".into());
        }
        let websocket = match get("type") {
            "" | "tcp" | "raw" => false,
            "ws" => true,
            _ => return Err("Rust VPN пока поддерживает только TCP и WebSocket".into()),
        };
        let tls = match get("security") {
            "" | "none" => false,
            "tls" => true,
            "reality" => true,
            _ => return Err("Неизвестный режим защиты VLESS".into()),
        };
        let vision = match get("flow") {
            "" => false,
            "xtls-rprx-vision" | "xtls-rprx-vision-udp443" => true,
            _ => return Err("Неизвестный flow VLESS".into()),
        };
        if vision && (!tls || websocket) {
            return Err("Vision требует TCP с TLS или REALITY".into());
        }
        let reality = if get("security") == "reality" {
            use base64::{
                Engine,
                engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
            };
            if websocket {
                return Err("REALITY требует TCP".into());
            }
            if get("sni").is_empty() && get("peer").is_empty() {
                return Err("Для REALITY нужен SNI".into());
            }
            let key = URL_SAFE_NO_PAD
                .decode(get("pbk"))
                .or_else(|_| URL_SAFE.decode(get("pbk")))
                .map_err(|_| "Неверный открытый ключ REALITY")?;
            let public_key: [u8; 32] = key
                .try_into()
                .map_err(|_| "Открытый ключ REALITY должен содержать 32 байта")?;
            if public_key == [0; 32] {
                return Err("Неверный открытый ключ REALITY".into());
            }
            let short = hex::decode(get("sid")).map_err(|_| "Неверный short ID REALITY")?;
            if short.len() > 8 {
                return Err("Short ID REALITY: максимум 8 байт".into());
            }
            let mut short_id = [0; 8];
            short_id[..short.len()].copy_from_slice(&short);
            if !get("pqv").is_empty() {
                return Err("ML-DSA REALITY пока не поддерживается".into());
            }
            Some(Reality {
                public_key,
                short_id,
            })
        } else {
            None
        };
        let fingerprint = match get("fp") {
            "" | "rustls" => Fingerprint::Rustls,
            "chrome" | "chrome133" => Fingerprint::Chrome133,
            "firefox" | "firefox120" => Fingerprint::Firefox120,
            _ if !tls => Fingerprint::Rustls,
            _ => return Err("Этот TLS fingerprint ещё не перенесён в Rust VPN".into()),
        };
        if fingerprint != Fingerprint::Rustls && reality.is_none() {
            return Err(format!("TLS fingerprint {} поддерживается только с REALITY", fingerprint.name()));
        }
        if !matches!(get("headerType"), "" | "none")
            || (!tls && (!get("pbk").is_empty() || !get("sid").is_empty()))
        {
            return Err("Неподдерживаемые параметры защиты или заголовка VLESS".into());
        }
        if !get("serviceName").is_empty() || !get("mode").is_empty() {
            return Err("Этот транспорт VLESS ещё не перенесён в Rust VPN".into());
        }
        // WebSocket needs HTTP/1.1; a raw byte tunnel must not negotiate an outer HTTP/2 session.
        if fingerprint != Fingerprint::Rustls && !matches!(get("alpn"), "" | "h2,http/1.1") {
            return Err(format!("REALITY с TLS fingerprint {} использует ALPN h2,http/1.1", fingerprint.name()));
        }
        if fingerprint == Fingerprint::Rustls && !matches!(get("alpn"), "" | "http/1.1") {
            return Err("Rust VPN пока поддерживает только ALPN http/1.1".into());
        }
        let server_name = [get("sni"), get("peer"), host.as_str()]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap()
            .to_string();
        reality_tls::pki_types::ServerName::try_from(server_name.clone())
            .map_err(|_| "Неверный SNI VLESS")?;
        let insecure = match (get("allowInsecure"), get("insecure")) {
            ("1" | "true", "" | "1" | "true") | ("", "1" | "true") => true,
            ("" | "0" | "false", "" | "0" | "false") => false,
            _ => return Err("Неверный параметр allowInsecure".into()),
        };
        let authority = if host.parse::<IpAddr>().is_ok_and(|ip| ip.is_ipv6()) {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let ws_host = if get("host").is_empty() {
            authority
        } else {
            get("host").to_string()
        };
        if ws_host.is_empty()
            || ws_host.len() > 1024
            || ws_host
                .bytes()
                .any(|c| c <= 32 || c >= 127 || b"/\\@?#".contains(&c))
        {
            return Err("Неверный Host WebSocket".into());
        }
        let ws_path = if get("path").is_empty() {
            "/"
        } else {
            get("path")
        }
        .to_string();
        if !ws_path.starts_with('/')
            || ws_path.len() > 4096
            || ws_path.bytes().any(|c| c <= 32 || c == 127 || c == b'#')
        {
            return Err("Неверный путь WebSocket".into());
        }
        if websocket
            && (ws_path.contains("?ed=") || ws_path.contains("&ed=") || !get("ed").is_empty())
        {
            return Err("WebSocket early data ещё не перенесены в Rust VPN".into());
        }
        let name = percent_decode_str(url.fragment().unwrap_or(""))
            .decode_utf8_lossy()
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect();
        Ok(Self {
            host,
            port,
            uuid,
            tls,
            insecure,
            server_name,
            websocket,
            ws_host,
            ws_path,
            name,
            reality,
            vision,
            flow: get("flow").to_string(),
            fingerprint,
        })
    }

    pub fn info(&self) -> Value {
        json!({"host":self.host,"port":self.port,"name":self.name,
            "network":if self.websocket {"ws"} else {"tcp"},"security":if self.reality.is_some() {"reality"} else if self.tls {"tls"} else {"none"},"flow":self.flow,"fingerprint":if self.tls {self.fingerprint.name()} else {""}})
    }
}
