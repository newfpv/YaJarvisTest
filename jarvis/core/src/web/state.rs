//! GET /api/state — всё, что панели нужно при открытии. Форма ответа та же, что у Go-версии.
//! То, что ещё не перенесено (плагины, музыка, ChatGPT), отдаётся пустым, но в той же форме.

use super::App;
use crate::{hardware, settings::Settings, version};
use axum::{Json, extract::State};
use serde_json::{Map, Value, json};
use std::sync::OnceLock;

fn catalog() -> &'static Map<String, Value> {
    static C: OnceLock<Map<String, Value>> = OnceLock::new();
    C.get_or_init(|| serde_json::from_str(include_str!("../catalog.json")).expect("catalog.json"))
}

pub fn mask(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    if s.len() <= 8 {
        return "задан".into();
    }
    format!("задан, …{}", s.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<String>())
}

fn key_tail(k: &str) -> String {
    if k.len() <= 8 { "…".into() } else { format!("…{}", k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<String>()) }
}

/// Сетевые подключения и «через VPN» по умолчанию (как netServices в Go).
pub const NET_SERVICES: &[(&str, &str, bool)] = &[
    ("chatgpt", "Подписка ChatGPT", true),
    ("openai", "OpenAI", true),
    ("gemini", "Google Gemini", true),
    ("grok", "xAI Grok", true),
    ("elevenlabs", "ElevenLabs", true),
    ("youtube", "YouTube", false),
    ("xiaomi", "Xiaomi Home", false),
    ("smartlife", "Smart Life", false),
    ("gdrive", "Google Диск", false),
    ("yandex", "Яндекс", false),
    ("plugins", "Плагины", false),
];

pub fn use_vpn(s: &Settings, svc: &str) -> bool {
    if let Some(v) = s.vpn.get(svc) {
        return *v;
    }
    if svc == "yandex" {
        return s.proxy_yandex;
    }
    NET_SERVICES.iter().find(|n| n.0 == svc).is_none_or(|n| n.2)
}

/// Разбор ссылки vless://uuid@host:port?type=…&security=…&sni=…#имя — только для показа.
fn vless_view(link: &str) -> Value {
    let mut v = json!({"set": !link.is_empty(), "error": Value::Null});
    let Some(rest) = link.strip_prefix("vless://") else { return v };
    let (rest, name) = rest.split_once('#').unwrap_or((rest, ""));
    let (addr, query) = rest.split_once('?').unwrap_or((rest, ""));
    let hostport = addr.rsplit_once('@').map(|x| x.1).unwrap_or(addr);
    let (host, port) = hostport.rsplit_once(':').unwrap_or((hostport, ""));
    let q = |k: &str| {
        query.split('&').filter_map(|kv| kv.split_once('=')).find(|(a, _)| *a == k).map(|(_, b)| pct(b)).unwrap_or_default()
    };
    let o = v.as_object_mut().unwrap();
    o.insert("name".into(), pct(name).into());
    o.insert("host".into(), host.trim_matches(['[', ']']).into());
    o.insert("port".into(), port.parse::<u16>().map(Value::from).unwrap_or(Value::from(port)));
    o.insert("network".into(), q("type").into());
    o.insert("security".into(), q("security").into());
    o.insert("sni".into(), q("sni").into());
    v
}

fn pct(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                if let Ok(x) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    out.push(x);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into()
}

pub fn memory_state(s: &Settings, host: &crate::plugins::Host) -> Value {
    let has = |p: &str| crate::llm::memory_available(host, s, p);
    let active = crate::llm::memory_candidates(host, s).first().copied();
    let backend = catalog().get("backend_models").cloned().unwrap_or(json!([]));
    json!({
        "off": s.memory.off, "provider": s.memory.provider, "model": s.memory.model,
        "has": {"chatgpt": has("chatgpt"), "gemini": has("gemini"), "openai": has("openai")},
        "models": {"chatgpt": backend, "gemini": [crate::llm::GEMINI_TEXT_MODEL], "openai": backend},
        "active": active, "active_model": active.map(|provider| crate::llm::memory_model(s, provider)),
    })
}

async fn security_checks() -> Value {
    let listening = |port: u16| async move {
        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)),
        )
        .await
        .is_ok_and(|r| r.is_ok())
    };
    let (adb, ssh) = tokio::join!(listening(5555), listening(2222));
    json!([
        {"id": "adb", "title": "ADB по сети (порт 5555)", "open": adb,
         "hint": "Через ADB любой в вашей Wi-Fi сети получает полный доступ к колонке без пароля. Нужен только для установки и обновления — потом его можно закрыть в программе установки."},
        {"id": "ssh", "title": "SSH (порт 2222)", "open": ssh,
         "hint": "Удалённый вход в консоль колонки. Если не пользуетесь — лучше закрыть; если пользуетесь — только по ключу."},
    ])
}

pub async fn state(State(app): State<App>) -> Json<Value> {
    let (xiaomi,smartlife)=tokio::join!(
        super::home_vendor::state(&app,"xiaomi"),
        super::home_vendor::state(&app,"smartlife")
    );
    let chatgpt_login=match app.plugins.provider("chatgpt"){
        Ok(provider)=>provider.request("login.status",Value::Null,std::time::Duration::from_secs(1)).await.unwrap_or(Value::Null),
        Err(_)=>Value::Null,
    };
    let s = app.store.get();
    let model = hardware::station_model();
    let now = jiff::Timestamp::now().as_second();
    let eleven: Vec<Value> = s
        .eleven_keys
        .iter()
        .map(|k| {
            let mut m = json!({"tail": key_tail(&k.key), "active": k.key == s.elevenlabs_key});
            if k.until != 0 && now < k.until {
                m["until"] = k.until.into();
                m["why"] = k.why.clone().into();
            }
            m
        })
        .collect();
    let gemini_models = catalog().get("gemini_models").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
    let gemini: Value = if s.gemini_keys.is_empty() {
        Value::Null
    } else {
        s.gemini_keys.iter().map(|k| json!({"tail": key_tail(k), "spent": 0, "models": gemini_models})).collect()
    };
    let vpn: Map<String, Value> = NET_SERVICES.iter().map(|n| (n.0.to_string(), use_vpn(&s, n.0).into())).collect();
    let mut night = s.night.clone();
    night.fill();
    let ha = &s.home_assistant;
    let mut vless = vless_view(&s.vless);
    let (info, verr) = app.proxy.status();
    if !verr.is_empty() {
        vless["error"] = verr.into();
    }
    if let Some(Value::Object(i)) = info {
        for k in ["name", "host", "port", "network", "security", "sni"] {
            if let Some(v) = i.get(k) {
                vless[k] = v.clone();
            }
        }
    }
    let str_of = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let wake_training = app.plugins.wake_training();
    let training_records = wake_training;
    let wake_word = if wake_training { app.engine.listener.phrase() } else { crate::wake_profiles::DEFAULT_PHRASE.into() };
    let mut out = json!({
        "version": version(),
        "home_plugin": match app.plugins.provider("homeassistant") {
            Ok(_) => json!({"available": true}),
            Err(e) => json!({"available": false, "error": e}),
        },
        "profiles": s.profiles, "active": s.active, "prompts": s.prompts, "live_voices": s.live_voices,
        "keys": {"openai": mask(&s.openai_key), "gemini": mask(&s.gemini_key), "grok": mask(&s.grok_key),
                 "elevenlabs": mask(&s.elevenlabs_key), "yandex": mask(&s.yandex_token)},
        "eleven_keys": eleven, "gemini_keys": gemini, "vpn": vpn,
        "chatgpt": {"signed_in": crate::settings::has_all(&s.chatgpt, &["refresh", "account"]),
                    "plan": str_of(&s.chatgpt, "plan"), "email": str_of(&s.chatgpt, "email"),
                    "login": chatgpt_login, "chat": super::memory::chat_state(&app), "memory": memory_state(&s,&app.plugins), "limited_until": crate::chatgpt::LIMITED_UNTIL.load(std::sync::atomic::Ordering::Relaxed)},
        "vless": vless,
        "youtube": super::music::youtube_state(&app).await,
        "home_assistant": {"url": str_of(ha, "url"), "configured": crate::settings::has_all(ha, &["url", "token"]),
                           "rooms": ha.get("rooms").cloned().unwrap_or(Value::Null)},
        "xiaomi": xiaomi,
        "smartlife": smartlife,
        "rooms": Value::Null,
        "update_available": app.updater.available(),
        "speaker": {"model": model.name, "model_id": model.id, "mic": app.engine.mic.online(), "mic_why": "", "leds": app.engine.leds.available(),
                    "wake_model": app.engine.listener.ready.load(std::sync::atomic::Ordering::Relaxed), "talking": app.engine.talking(), "uptime_s": app.started.elapsed().as_secs(), "volume": tokio::task::spawn_blocking(super::vol).await.unwrap_or(-1)},
        "settings": {"wake_enabled": s.wake_enabled, "wake_reply_enabled": s.wake_reply_enabled, "wake_word": wake_word, "wake_training": wake_training, "training_records": training_records, "wake_profile": if wake_training { &s.wake_profile } else { "" }, "wake_sensitivity": s.wake_sensitivity, "mic_boost_db": s.mic_boost_db,
                     "mic_gain_db": s.mic_gain_db, "wake_check": s.wake_check, "follow_up_s": s.follow_up_s, "city": s.city,
                     "timezone": s.timezone, "volume": s.volume, "lan_only": s.lan_only, "home_via": s.home_via,
                     "home_use": s.home_use_state(), "speaker_room": s.speaker_room, "led": s.led, "train_mode": s.train_mode,
                     "language": s.language, "night": night, "night_now": s.is_night(), "radio": !s.radio_off,
                     "onboarded": s.onboarded, "plugins_warned": s.plugins_warned},
        "plugins": app.plugins.summary(),
        "providers": app.plugins.service_providers(),
        "text_plugins": app.plugins.text_plugins(),
        "stt_plugins": app.plugins.stt_plugins(),
        "tts_plugins": app.plugins.tts_plugins(),
        "repo": crate::DEFAULT_REPO,
        "security": security_checks().await,
    });
    let o = out.as_object_mut().unwrap();
    for (k, v) in catalog() {
        o.insert(k.clone(), v.clone());
    }
    Json(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_like_go() {
        assert_eq!(mask(""), "");
        assert_eq!(mask("short"), "задан");
        assert_eq!(mask("key-1234567890abcd"), "задан, …abcd");
    }

    #[test]
    fn parses_vless_for_display() {
        let v = vless_view("vless://id@example.org:443?type=tcp&security=reality&sni=a.b#%D0%94%D0%BE%D0%BC");
        assert_eq!(v["host"], "example.org");
        assert_eq!(v["port"], 443);
        assert_eq!(v["security"], "reality");
        assert_eq!(v["name"], "Дом");
    }
}
