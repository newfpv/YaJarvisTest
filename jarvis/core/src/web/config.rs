//! Общие настройки панели. Учётные записи и протоколы подключений остаются у модулей.
use super::{App, bad, ok, reply};
use crate::settings::{MAX_TRAIN_LIMIT_MB, Profile, Prompt, Settings};
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::Response,
};
use serde_json::{Value, json};
use std::collections::HashMap;

fn invalid(msg: impl Into<String>) -> std::io::Error {
    std::io::Error::other(msg.into())
}
fn timezone(s: &str) -> Result<(), std::io::Error> {
    jiff::tz::TimeZone::get(s)
        .map(|_| ())
        .map_err(|_| invalid("Неизвестный часовой пояс"))
}
fn color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}
fn led(l: &mut crate::settings::LedSettings) -> Result<(), std::io::Error> {
    l.brightness = l.brightness.clamp(5, 100);
    if [&l.listening, &l.thinking, &l.speaking]
        .iter()
        .any(|c| !color(c))
    {
        return Err(invalid("Цвет подсветки — #rrggbb"));
    }
    Ok(())
}

fn patch_settings(s: &mut Settings, v: &Value) -> Result<(), std::io::Error> {
    let input = v
        .as_object()
        .ok_or_else(|| invalid("Нужен объект настроек"))?;
    const ALLOWED: &[&str] = &[
        "wake_enabled",
        "wake_reply_enabled",
        "wake_check",
        "vpn",
        "wake_sensitivity",
        "mic_boost_db",
        "mic_gain_db",
        "memory",
        "follow_up_s",
        "city",
        "timezone",
        "volume",
        "lan_only",
        "home_via",
        "home_use",
        "speaker_room",
        "dev_mode",
        "language",
        "led",
        "train_mode",
        "train_limit_mb",
        "night",
        "radio",
    ];
    let mut out = serde_json::to_value(&*s)?;
    for (k, v) in input {
        if !ALLOWED.contains(&k.as_str()) {
            return Err(invalid(format!("Неизвестная общая настройка: {k}")));
        }
        if v.is_null() {
            return Err(invalid(format!("Пустое значение: {k}")));
        }
        if k == "radio" {
            out["radio_off"] = (!v
                .as_bool()
                .ok_or_else(|| invalid("radio должен быть true/false"))?)
            .into();
        } else if ["vpn", "home_use"].contains(&k.as_str()) {
            let map = v
                .as_object()
                .ok_or_else(|| invalid("Ожидался объект переключателей"))?;
            if !out[k].is_object() {
                out[k] = json!({});
            }
            for (name, on) in map {
                if !on.is_boolean() {
                    return Err(invalid("Переключатель должен быть true/false"));
                }
                if k == "home_use"
                    && ![
                        "yandex",
                        "alice",
                        "yandexmusic",
                        "homeassistant",
                        "xiaomi",
                        "smartlife",
                    ]
                    .contains(&name.as_str())
                {
                    return Err(invalid("Неизвестный способ управления домом"));
                }
                if k == "vpn" && !super::state::NET_SERVICES.iter().any(|n| n.0 == name) {
                    return Err(invalid("Неизвестное сетевое подключение"));
                }
                out[k][name] = on.clone();
            }
        } else if k == "led" {
            let enabled = v["enabled"]
                .as_bool()
                .ok_or_else(|| invalid("Нужно led.enabled"))?;
            out[k]["enabled"] = enabled.into();
        } else {
            out[k] = v.clone();
        }
    }
    let mut next: Settings = serde_json::from_value(out)?;
    timezone(&next.timezone)?;
    if !["", "ru", "en", "uk"].contains(&next.language.as_str()) {
        return Err(invalid("Неизвестный язык"));
    }
    if !["alice", "jarvis"].contains(&next.home_via.as_str()) {
        return Err(invalid("Неизвестный способ управления домом"));
    }
    next.wake_sensitivity = next.wake_sensitivity.clamp(1, 10);
    next.mic_boost_db = next.mic_boost_db.clamp(0, 18);
    next.mic_gain_db = next.mic_gain_db.clamp(0, 30);
    next.follow_up_s = next.follow_up_s.clamp(0.0, 30.0);
    next.volume = next.volume.clamp(0, 100);
    next.train_limit_mb = next.train_limit_mb.clamp(1, MAX_TRAIN_LIMIT_MB);
    next.city = next.city.trim().into();
    next.speaker_room = next.speaker_room.trim().into();
    for time in [&next.night.from, &next.night.to] {
        if !time.is_empty() && crate::settings::hhmm(time).is_none() {
            return Err(invalid("Время ночного режима — HH:MM"));
        }
    }
    next.night.fill();
    *s = next;
    Ok(())
}
pub async fn settings(State(app): State<App>, Json(v): Json<Value>) -> Response {
    if let Err(e) = app.store.update(|s| patch_settings(s, &v)) {
        return bad(e.to_string());
    }
    let s = app.store.get();
    crate::jlog::set_timezone(&s.timezone);
    app.engine.leds.configure(&s.led_for());
    if v.get("night").is_some() {
        let store = app.store.clone();
        tokio::task::spawn_blocking(move || crate::speakervol::night_cap(&store));
    }
    ok()
}
pub async fn profile(State(app): State<App>, Json(mut p): Json<Profile>) -> Response {
    p.name = p.name.trim().into();
    if p.name.is_empty() || p.name.chars().count() > 40 {
        return bad("Имя — от 1 до 40 символов");
    }
    if !p.timezone.is_empty() && timezone(&p.timezone).is_err() {
        return bad("Неизвестный часовой пояс");
    }
    p.idle_s = if p.idle_s <= 0.0 {
        8.0
    } else {
        p.idle_s.clamp(2.0, 60.0)
    };
    if p.volume != 0 {
        p.volume = p.volume.clamp(10, 100);
    }
    if let Some(l) = &mut p.led {
        if let Err(e) = led(l) {
            return bad(e.to_string());
        }
    }
    p.engine = p.engine.trim().into();
    p.stt_engine = p.stt_engine.trim().into();
    p.llm_engine = p.llm_engine.trim().into();
    p.tts_engine = p.tts_engine.trim().into();
    p.llm_plugin = p.llm_plugin.trim().into();
    p.stt_model = p.stt_model.trim().into();
    p.llm_model = p.llm_model.trim().into();
    p.tts_model = p.tts_model.trim().into();
    for (id, msg) in [
        (&p.stt_engine, "Неверный идентификатор плагина распознавания"),
        (&p.llm_engine, "Неверный идентификатор текстового провайдера"),
        (&p.tts_engine, "Неверный идентификатор плагина озвучки"),
        (&p.llm_plugin, "Неверный идентификатор текстового плагина"),
    ] {
        if !id.is_empty() && !crate::plugins::valid_id(id) {
            return bad(msg);
        }
    }
    for m in [&p.stt_model, &p.llm_model, &p.tts_model] {
        if m.len() > 160 || m.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return bad("Некорректное имя модели");
        }
    }
    if p.mode != "classic" {
        p.mode = "live".into();
    } else {
        if p.llm_engine.is_empty() && !p.llm_plugin.is_empty() {
            p.llm_engine = p.llm_plugin.clone();
        }
        if matches!(p.llm_engine.as_str(), "" | "openai" | "chatgpt" | "gemini" | "grok") {
            p.llm_plugin.clear();
        } else {
            p.llm_plugin = p.llm_engine.clone();
        }
        if p.engine.is_empty() || p.engine == "live" {
            p.engine = "classic".into();
        }
    }
    if !p.is_classic() && p.engine == "chatgpt" {
        let Some(voice) = crate::chatgpt::voice(&p.voice) else { return bad("Этот голос недоступен для подписки ChatGPT") };
        p.voice = voice;
    }
    let result = app.store.update(|s| -> Result<(), std::io::Error> {
        if s.profiles
            .iter()
            .any(|x| x.id != p.id && x.name.to_lowercase() == p.name.to_lowercase())
        {
            return Err(invalid("Ассистент с таким именем уже есть"));
        }
        if let Some(old) = s
            .profiles
            .iter_mut()
            .find(|x| !p.id.is_empty() && x.id == p.id)
        {
            *old = p.clone();
        } else {
            p.id = format!("p{}", crate::settings::rand_hex(4));
            s.profiles.push(p.clone());
        }
        Ok(())
    });
    match result {
        Ok(()) => reply(StatusCode::OK, json!({"ok":true,"id":p.id})),
        Err(e) => bad(e.to_string()),
    }
}
pub async fn delete_profile(
    State(app): State<App>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(id) = q.get("id") else {
        return bad("Нужен id");
    };
    match app.store.update(|s| -> Result<(), std::io::Error> {
        if &s.active == id {
            return Err(invalid("Сначала включите другого ассистента"));
        }
        s.profiles.retain(|p| &p.id != id);
        Ok(())
    }) {
        Ok(()) => ok(),
        Err(e) => bad(e.to_string()),
    }
}
pub async fn active(State(app): State<App>, Json(v): Json<Value>) -> Response {
    let id = v
        .get("id")
        .or_else(|| v.get("ID"))
        .and_then(Value::as_str)
        .unwrap_or("");
    match app.store.update(|s| -> Result<(), std::io::Error> {
        if !s.profiles.iter().any(|p| p.id == id) {
            return Err(invalid("Нет такого ассистента"));
        }
        s.active = id.into();
        Ok(())
    }) {
        Ok(()) => {
            app.engine.leds.configure(&app.store.with(|s| s.led_for()));
            ok()
        }
        Err(e) => bad(e.to_string()),
    }
}
pub async fn order(State(app): State<App>, Json(v): Json<Value>) -> Response {
    let Some(ids) = v
        .get("ids")
        .or_else(|| v.get("IDs"))
        .and_then(Value::as_array)
    else {
        return bad("Нужен список ids");
    };
    match app.store.update(|s| -> Result<(), std::io::Error> {
        s.profiles.sort_by_key(|p| {
            ids.iter()
                .position(|id| id.as_str() == Some(&p.id))
                .unwrap_or(usize::MAX)
        });
        Ok(())
    }) {
        Ok(()) => ok(),
        Err(e) => bad(e.to_string()),
    }
}
pub async fn prompt(State(app): State<App>, Json(mut p): Json<Prompt>) -> Response {
    p.name = p.name.trim().into();
    if p.name.is_empty() || p.text.trim().is_empty() || p.text.len() > 20000 {
        return bad("Нужны название и текст до 20000 байт");
    }
    match app.store.update(|s| -> Result<(), std::io::Error> {
        if let Some(old) = s
            .prompts
            .iter_mut()
            .find(|x| !p.id.is_empty() && x.id == p.id)
        {
            *old = p.clone();
        } else {
            p.id = format!("c{}", crate::settings::rand_hex(4));
            s.prompts.push(p.clone());
        }
        Ok(())
    }) {
        Ok(()) => reply(StatusCode::OK, json!({"ok":true,"id":p.id})),
        Err(e) => bad(e.to_string()),
    }
}
pub async fn delete_prompt(
    State(app): State<App>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let Some(id) = q.get("id") else {
        return bad("Нужен id");
    };
    match app.store.update(|s| -> Result<(), std::io::Error> {
        if s.profiles.iter().any(|p| &p.prompt == id) {
            return Err(invalid("Этот характер использует ассистент"));
        }
        s.prompts.retain(|p| &p.id != id);
        Ok(())
    }) {
        Ok(()) => ok(),
        Err(e) => bad(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patch_preserves_secrets_and_other_switches() {
        let mut s = Settings::default();
        s.timezone = "Europe/Minsk".into();
        s.home_via = "alice".into();
        s.password_hash = "secret".into();
        s.train_limit_mb = 50;
        s.vpn.insert("chatgpt".into(), true);
        patch_settings(
            &mut s,
            &json!({"vpn":{"youtube":true},"volume":300,"radio":false,"wake_reply_enabled":false}),
        )
        .unwrap();
        assert_eq!(s.password_hash, "secret");
        assert_eq!(s.volume, 100);
        assert_eq!(s.train_limit_mb, 50);
        assert!(s.radio_off);
        assert!(!s.wake_reply_enabled);
        assert!(s.wake_enabled);
        assert!(s.vpn["chatgpt"] && s.vpn["youtube"]);
        assert!(patch_settings(&mut s, &json!({"password_hash":"replace"})).is_err());
        assert!(patch_settings(&mut s, &json!({"timezone":"not-a-timezone"})).is_err());
        assert_eq!(s.timezone, "Europe/Minsk");
    }
}
