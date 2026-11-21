//! YouTube player script, evaluated without filesystem, process or network bindings.
use jarvis_home_common::bytes;
use regex_lite::Regex;
use reqwest::Client;
use rquickjs::{CatchResultExt, Context, Function, Object, Runtime};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const EJS: &str = include_str!("ejs.js");
const WEB_AGENT: &str = super::remote::WEB_AGENT;
const MAX_HEAP: usize = 28 << 20;
const JS_TIMEOUT: Duration = Duration::from_secs(75);

pub struct Prepared {
    context: Context,
    _runtime: Runtime,
    deadline: Arc<Mutex<Instant>>,
    pub timestamp: u32,
}
impl Prepared {
    pub(crate) fn new(source: String) -> Result<Self, String> {
        let timestamp = Regex::new(r"signatureTimestamp\s*[=:]\s*(\d+)")
            .unwrap()
            .captures(&source)
            .and_then(|m| m[1].parse().ok())
            .ok_or("Нет версии подписи плеера YouTube")?;
        let body_start =
            Regex::new(r"^\s*(var\s+[\w$]+\s*=\s*\{\}\s*;\s*)?\(function\(([\w$]*)\)\{").unwrap();
        let body_end =
            Regex::new(r"\}\)(\.call\(this(,\s*[\w$]+)?\)|\([\w$]*\))\s*;?\s*$").unwrap();
        let head = body_start
            .captures(&source)
            .ok_or("Неизвестный формат плеера YouTube")?;
        let end = body_end
            .find(&source)
            .ok_or("Нет конца плеера YouTube")?
            .start();
        let body = source
            .get(head.get(0).unwrap().end()..end)
            .ok_or("Неверный формат плеера YouTube")?;
        let cuts = boundaries(body.as_bytes());
        let runtime = Runtime::new().map_err(|_| "Не запустить движок плеера YouTube")?;
        runtime.set_memory_limit(MAX_HEAP);
        runtime.set_max_stack_size(6 << 20);
        let deadline = Arc::new(Mutex::new(Instant::now() + JS_TIMEOUT));
        let cutoff = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            Instant::now() >= *cutoff.lock().unwrap()
        })));
        let context = Context::full(&runtime).map_err(|_| "Не создать окружение плеера YouTube")?;
        let chunks = context
            .with(|ctx| {
                ctx.eval::<(), _>(EJS)
                    .catch(&ctx)
                    .map_err(|e| e.to_string())?;
                let begin: Function = ctx
                    .eval("JarvisEjs.begin")
                    .catch(&ctx)
                    .map_err(|e| e.to_string())?;
                let job: Object = begin
                    .call((&head[2], head.get(1).is_some()))
                    .catch(&ctx)
                    .map_err(|e| e.to_string())?;
                let add: Function = job.get("add").map_err(|e| e.to_string())?;
                let finish: Function = job.get("finish").map_err(|e| e.to_string())?;
                let mut start = 0;
                let mut index = 0;
                while index < cuts.len() {
                    let mut part = (index + 31).min(cuts.len() - 1);
                    loop {
                        let end = cuts[part];
                        if add
                            .call::<_, bool>((&body[start..end],))
                            .catch(&ctx)
                            .map_err(|e| format!("parse {start}..{end}: {e}"))?
                        {
                            start = end;
                            index = part + 1;
                            break;
                        }
                        if part == cuts.len() - 1 || end - start > 256 << 10 {
                            return Err("Неизвестный синтаксис плеера".to_string());
                        }
                        part = (part + 8).min(cuts.len() - 1);
                    }
                }
                finish
                    .call::<_, Vec<String>>(())
                    .catch(&ctx)
                    .map_err(|e| format!("prepare retained functions: {e}"))
            })
            .map_err(|e| format!("Не разобрать текущий плеер YouTube: {e}"))?;
        runtime.run_gc();
        for (index, chunk) in chunks.into_iter().enumerate() {
            context
                .with(|ctx| {
                    ctx.eval::<(), _>(chunk)
                        .catch(&ctx)
                        .map_err(|e| e.to_string())
                })
                .map_err(|e| format!("Не запустить плеер YouTube, фрагмент {index}: {e}"))?;
        }
        runtime.set_max_stack_size(512 << 10);
        runtime.run_gc();
        Ok(Self {
            context,
            _runtime: runtime,
            deadline,
            timestamp,
        })
    }
    pub async fn decode(
        self: &Arc<Self>,
        signature: String,
        n: String,
    ) -> Result<(String, String), String> {
        let prepared = self.clone();
        tokio::task::spawn_blocking(move || {
            prepared
                .context
                .with(|ctx| {
                    *prepared.deadline.lock().unwrap() = Instant::now() + Duration::from_secs(2);
                    let solve: Function = ctx.globals().get("__solve")?;
                    let mut output = [String::new(), String::new()];
                    for (i, (kind, value)) in [("sig", signature), ("n", n)].into_iter().enumerate()
                    {
                        if value.is_empty() {
                            continue;
                        }
                        output[i] = solve.call((kind, value))?;
                    }
                    let [signature, n] = output;
                    Ok::<_, rquickjs::Error>((signature, n))
                })
                .map_err(|_| "Не расшифровать ссылку YouTube".into())
        })
        .await
        .map_err(|_| "Движок плеера завершился")?
    }
}

#[derive(Default)]
pub struct Scripts(tokio::sync::Mutex<ScriptCache>);
#[derive(Default)]
struct ScriptCache {
    current: Option<(Instant, Instant, String, Arc<Prepared>)>,
    failed: Option<(Instant, String)>,
}
impl Scripts {
    pub fn cached(&self) -> bool {
        self.0.try_lock().is_ok_and(|cache| cache.current.is_some())
    }
    pub async fn evict_if_idle(&self, idle: Duration) {
        let mut state = self.0.lock().await;
        if state
            .current
            .as_ref()
            .is_some_and(|(_, used, _, _)| used.elapsed() >= idle)
        {
            state.current.take();
        }
    }
    pub async fn get(&self, client: &Client) -> Result<Arc<Prepared>, String> {
        let mut state = self.0.lock().await;
        if let Some((at, used, _, prepared)) = state.current.as_mut()
            && at.elapsed() < Duration::from_secs(1800)
        {
            *used = Instant::now();
            return Ok(prepared.clone());
        }
        if let Some((at, error)) = &state.failed
            && at.elapsed() < Duration::from_secs(120)
        {
            return Err(error.clone());
        }
        let iframe = fetch(client, "https://www.youtube.com/iframe_api", 1 << 20).await?;
        let id = Regex::new(r"player\\?/([0-9A-Za-z_-]{8})\\?/")
            .unwrap()
            .captures(&iframe)
            .map(|capture| capture[1].to_owned())
            .ok_or("Не найдена версия плеера YouTube")?;
        if let Some((at, used, old, prepared)) = state.current.as_mut()
            && *old == id
        {
            let now = Instant::now();
            *at = now;
            *used = now;
            return Ok(prepared.clone());
        }
        state.current.take();
        let url = format!("https://www.youtube.com/s/player/{id}/player_ias.vflset/en_US/base.js");
        let source = fetch(client, &url, 8 << 20).await?;
        let started = Instant::now();
        let (send, result) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("youtube-script".into())
            .stack_size(8 << 20)
            .spawn(move || {
                let _ = send.send(Prepared::new(source));
            })
            .map_err(|_| "Не запустить движок плеера")?;
        let prepared = match result
            .await
            .map_err(|_| "Движок плеера завершился".to_owned())
            .and_then(|result| result)
        {
            Ok(prepared) => Arc::new(prepared),
            Err(error) => {
                log::debug!("YouTube: {error}");
                #[cfg(test)]
                eprintln!("Player parser: {error}");
                let message = "Не разобрать текущий плеер YouTube".to_owned();
                state.failed = Some((Instant::now(), message.clone()));
                return Err(message);
            }
        };
        log::info!(
            "YouTube: плеер {id} готов за {:.2} с",
            started.elapsed().as_secs_f64()
        );
        let now = Instant::now();
        state.current = Some((now, now, id, prepared.clone()));
        state.failed = None;
        Ok(prepared)
    }
}
async fn fetch(client: &Client, url: &str, limit: usize) -> Result<String, String> {
    let resp = client
        .get(url)
        .header("User-Agent", WEB_AGENT)
        .send()
        .await
        .map_err(|_| "Не скачать плеер YouTube")?;
    String::from_utf8(bytes(resp, limit).await?)
        .map_err(|_| "Неверная кодировка плеера YouTube".into())
}

fn word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$')
}

// Pick safe boundaries for the minified wrapper. Each slice still goes through the JS parser.
fn boundaries(bytes: &[u8]) -> Vec<usize> {
    let mut cuts = Vec::new();
    let mut depth = 0i32;
    let mut prev = 0;
    let mut word = &bytes[..0];
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        i += 1;
        match byte {
            b if b.is_ascii_whitespace() => continue,
            b'/' if bytes.get(i) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if bytes.get(i) == Some(&b'*') => {
                i = bytes[i + 1..]
                    .windows(2)
                    .position(|part| part == b"*/")
                    .map_or(bytes.len(), |n| i + n + 3);
                continue;
            }
            b'\'' | b'"' | 0x60 => {
                while let Some(&next) = bytes.get(i) {
                    i += 1;
                    if next == byte {
                        break;
                    }
                    if next == b'\\' {
                        i += 1;
                    }
                }
            }
            b'/' if prev == 0
                || b"(,=:[!&|?{};+-*%<>~^".contains(&prev)
                || matches!(
                    word,
                    b"return"
                        | b"typeof"
                        | b"case"
                        | b"do"
                        | b"else"
                        | b"in"
                        | b"of"
                        | b"new"
                        | b"delete"
                        | b"void"
                        | b"throw"
                        | b"instanceof"
                        | b"yield"
                        | b"await"
                ) =>
            {
                let mut in_class = false;
                while let Some(&next) = bytes.get(i) {
                    i += 1;
                    match next {
                        b'\\' => i += 1,
                        b'[' => in_class = true,
                        b']' => in_class = false,
                        b'/' if !in_class => break,
                        b'\n' => break,
                        _ => {}
                    }
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b';' if depth == 0 => {
                let tail = &bytes[i..];
                let k = tail
                    .iter()
                    .position(|b| !b.is_ascii_whitespace())
                    .unwrap_or(tail.len());
                let tail = &tail[k..];
                if ![&b"else"[..], b"catch", b"finally", b"while"]
                    .iter()
                    .any(|w| {
                        tail.starts_with(w) && !tail.get(w.len()).is_some_and(|b| word_byte(*b))
                    })
                {
                    cuts.push(i);
                }
            }
            b if word_byte(b) => {
                let start = i - 1;
                while bytes.get(i).is_some_and(|b| word_byte(*b)) {
                    i += 1;
                }
                word = &bytes[start..i];
                prev = bytes[i - 1];
                continue;
            }
            _ => {}
        }
        prev = byte;
        word = &bytes[..0];
    }
    if cuts.last() != Some(&bytes.len()) {
        cuts.push(bytes.len());
    }
    cuts
}
