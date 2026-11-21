//! ChatGPT по подписке: голос (GPT-Live) и «мозг» (Codex) живут в плагине chatgpt, ядро — мост:
//! звук микрофона уходит плагину (voice.feed), голос и расшифровки приходят событиями voice.event.

use crate::{
    journal::Journal,
    plugins::{BoxFut, Host, Runtime},
    session::{Dialog, Session, queue_mic, take_mic_caught_up},
    settings::{BACKEND_MODELS, Profile},
    tools::Tools,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Condvar, LazyLock, Mutex, Weak,
        atomic::{AtomicBool, AtomicI64, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const VOICES: &[&str] = &[
    "cove", "juniper", "maple", "spruce", "ember", "vale", "breeze", "arbor", "sol",
    "aube", "rivage", "bubbie", "haetsal", "hanul", "himari", "rindo", "leher", "neer",
    "miko", "sonna", "rio", "viola", "tinta",
];
const LIVE_MODEL: &str = "GPT-Live по подписке";

/// Пауза перед следующим голосовым звонком после HTTP 429.
pub static LIMITED_UNTIL: AtomicI64 = AtomicI64::new(0);
/// Сколько звука микрофона (16 кГц) уходит плагину за такт 20 мс.
const FEED_MAX: usize = 960;

fn unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn voice_limit() -> Option<String> {
    let remaining = LIMITED_UNTIL.load(Ordering::Relaxed) - unix();
    (remaining > 0).then(|| format!("ChatGPT ограничил голосовые звонки аккаунта. Повторите через {} мин.", (remaining + 59) / 60))
}
fn clip_error(text:&str)->String{text.chars().take(240).collect()}
async fn await_open(future:impl std::future::Future<Output=Result<Value,String>>,closed:&tokio::sync::Notify)->Result<(),String>{
    tokio::pin!(future);
    tokio::select!{
        biased;
        result=&mut future=>result.map(|_|()),
        _=closed.notified()=>tokio::time::timeout(Duration::from_millis(100),&mut future).await.unwrap_or_else(|_|Err("разговор отменён".into())).map(|_|()),
    }
}

pub fn voice(v: &str) -> Option<String> {
    let v = v.trim().to_ascii_lowercase();
    if v.is_empty() { return Some("cove".into()); }
    VOICES.contains(&v.as_str()).then_some(v)
}

pub fn new_id() -> String {
    let mut b = [0u8; 16];
    let _ = getrandom::fill(&mut b);
    let h = hex::encode(b);
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// Вход в ChatGPT есть: refresh и account.
pub fn signed_in(v: &Value) -> bool {
    ["refresh", "account"].iter().all(|k| v.get(k).and_then(Value::as_str).is_some_and(|s| !s.is_empty()))
}

pub type Brain = Arc<dyn Fn(String) -> BoxFut<Result<String, String>> + Send + Sync>;

static SESSIONS: LazyLock<Mutex<HashMap<String, Weak<ChatGptSession>>>> = LazyLock::new(Mutex::default);

pub struct ChatGptSession {
    id: String,
    dialog: Dialog,
    journal: Arc<Journal>,
    memory: Option<Arc<crate::memory::Memory>>,
    rt: tokio::runtime::Handle,
    brain: Option<Brain>,
    pending: Mutex<Vec<i16>>,
    out_tx: Mutex<Option<SyncSender<Vec<i16>>>>,
    out_rx: Mutex<Option<Receiver<Vec<i16>>>>,
    provider: Mutex<Option<Runtime>>,
    started: Mutex<Option<Receiver<Result<(), String>>>>,
    cancelled: AtomicBool,
    wake: tokio::sync::Notify,
    done: (Mutex<bool>, Condvar),
    created:Instant,
    first_audio:AtomicBool,
    last_input: Mutex<Option<Instant>>,
}

pub struct Open {
    pub host: Arc<Host>,
    pub voice: String,
    pub thread: Value,
    pub instructions: String,
    pub brain: Option<Brain>,
    pub journal: Arc<Journal>,
    pub memory: Option<Arc<crate::memory::Memory>>,
}

impl ChatGptSession {
    pub fn open(rt: &tokio::runtime::Handle, o: Open) -> Arc<ChatGptSession> {
        let (tx, rx) = sync_channel(400);
        let (stx, srx) = sync_channel(1);
        let s = Arc::new(ChatGptSession {
            id: new_id(),
            dialog: Dialog::new(o.journal.clone(), "GPT-Live (subscription)", LIVE_MODEL),
            journal: o.journal,
            memory: o.memory,
            rt: rt.clone(),
            brain: o.brain,
            pending: Mutex::default(),
            out_tx: Mutex::new(Some(tx)),
            out_rx: Mutex::new(Some(rx)),
            provider: Mutex::default(),
            started: Mutex::new(Some(srx)),
            cancelled: AtomicBool::new(false),
            wake: tokio::sync::Notify::new(),
            done: (Mutex::new(false), Condvar::new()),
            created:Instant::now(),first_audio:AtomicBool::new(false),last_input:Mutex::default(),
        });
        let me = s.clone();
        rt.spawn(async move { me.run(o.host, o.voice, o.thread, o.instructions, stx).await });
        s
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.wake.notify_one();
    }

    async fn run(self: Arc<Self>, host: Arc<Host>, voice: String, thread: Value, instructions: String, started: SyncSender<Result<(), String>>) {
        let n = match host.provider("chatgpt") {
            Ok(n) => n,
            Err(e) => {
                let _ = started.send(Err(e));
                return self.finish();
            }
        };
        drop(host);
        *self.provider.lock().unwrap() = Some(n.clone());
        SESSIONS.lock().unwrap().insert(self.id.clone(), Arc::downgrade(&self));
        let t0 = Instant::now();
        let open = async {
            let left = Duration::from_secs(40).saturating_sub(t0.elapsed()).max(Duration::from_secs(1));
            let response = n.request("voice.open", json!({"id": self.id, "profile": {"voice": voice, "thread": thread}, "instructions": instructions}), left).await?;
            if let Some(model) = response["requested_model"].as_str() {
                log::info!("ChatGPT: запрошена модель {model} (фактическую подтверждает событие сессии)");
            }
            Ok(response)
        };
        // `closed` may arrive immediately before the actual connection error.
        let res = await_open(open,&self.wake).await;
        let ok = res.is_ok();
        if ok{log::info!("ChatGPT: разговор готов за {:.2} с, в очереди микрофона {:.2} с",self.created.elapsed().as_secs_f64(),self.pending.lock().unwrap().len() as f64/16000.0);}
        let _ = started.send(res);
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while ok && !self.cancelled.load(Ordering::SeqCst) {
            tokio::select! {
                _ = self.wake.notified() => continue,
                _ = tick.tick() => {}
            }
            // До 60 мс за такт, чтобы накопленный звук быстро дошёл до плагина.
            // Длинные тихие паузы сокращаем до отправки. Темп RTP задаёт плагин.
            let pcm = take_mic_caught_up(&mut self.pending.lock().unwrap(), FEED_MAX);
            if pcm.is_empty() {
                continue;
            }
            let bytes: Vec<u8> = pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
            if n.notify("voice.feed", json!({"id": self.id, "pcm": B64.encode(bytes)})).await.is_err() {
                break;
            }
        }
        self.out_tx.lock().unwrap().take(); // канал голоса закрывается — движок поймёт, что разговор оборвался
        SESSIONS.lock().unwrap().remove(&self.id);
        let _ = n.request("voice.close", json!({"id": self.id}), Duration::from_secs(1)).await;
        self.finish();
    }

    fn finish(&self) {
        self.out_tx.lock().unwrap().take();
        *self.done.0.lock().unwrap() = true;
        self.done.1.notify_all();
    }

    /// Разговор до этого — для задачи бэкенду (без общей памяти: последние полчаса журнала).
    fn with_history(&self, task: &str) -> String {
        if let Some(m) = self.memory.as_ref().filter(|m| m.enabled()) {
            // общий чат: весь недавний разговор, а не только последние полчаса
            let mem = m.transcript(4000);
            if mem.is_empty() {
                return task.into();
            }
            return format!(
                "Разговор до этого (для контекста; что было раньше — ищи инструментом recall_conversation):\n{mem}\n\nЗадача от голосового ассистента — выполни её с учётом разговора выше: {task}"
            );
        }
        let now = crate::journal::now_s();
        let mut lines = vec![];
        for t in self.journal.items(16).iter().rev() {
            if now - t.ts > 1800.0 {
                continue;
            }
            if !t.text.is_empty() {
                lines.push(format!("Человек: {}", t.text));
            }
            if !t.answer.is_empty() {
                lines.push(format!("Джарвис: {}", t.answer));
            }
        }
        if lines.is_empty() {
            return task.into();
        }
        let from = lines.len().saturating_sub(20);
        format!("Разговор до этого (для контекста):\n{}\n\nЗадача от голосового ассистента — выполни её с учётом разговора выше: {task}", lines[from..].join("\n"))
    }
}

impl Session for ChatGptSession {
    fn feed(&self, pcm: &[i16]) {
        if !self.first_audio.load(Ordering::Relaxed)
            && pcm.chunks(320).any(|frame| frame.iter().filter(|v| v.unsigned_abs() > 220).count() >= 24)
        {
            *self.last_input.lock().unwrap() = Some(Instant::now());
        }
        queue_mic(&mut self.pending.lock().unwrap(), pcm);
    }
    fn take_audio(&self) -> Option<Receiver<Vec<i16>>> {
        self.out_rx.lock().unwrap().take()
    }
    fn rate(&self) -> usize {
        jarvis_dsp::audio::OUT_RATE
    }
    fn out_boost(&self) -> f64 {
        4.5
    }
    fn ready(&self, timeout: Duration) -> Result<(), String> {
        let Some(rx) = self.started.lock().unwrap().take() else { return Ok(()) };
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(_) => Err("ChatGPT timeout".into()),
        }
    }
    fn say(&self, text: &str) -> Result<(), String> {
        let n = self.provider.lock().unwrap().clone().ok_or("ChatGPT not ready")?;
        let p = json!({"id": self.id, "text": text});
        self.rt.block_on(async move { n.request("voice.say", p, Duration::from_secs(20)).await }).map(|_| ())
    }
    fn close(&self) {
        self.cancel();
        let g = self.done.0.lock().unwrap();
        let _ = self.done.1.wait_timeout_while(g, Duration::from_secs(3), |d| !*d);
    }
    fn dialog(&self) -> &Dialog {
        &self.dialog
    }
}

/// Событие голосовой сессии от плагина chatgpt.
pub fn module_voice_event(p: Value) {
    let id = p.get("id").or(p.get("ID")).and_then(Value::as_str).unwrap_or("");
    let kind = p.get("kind").or(p.get("Kind")).and_then(Value::as_str).unwrap_or("");
    let value = p.get("value").or(p.get("Value")).cloned().unwrap_or(Value::Null);
    let Some(s) = SESSIONS.lock().unwrap().get(id).and_then(Weak::upgrade) else { return };
    if s.cancelled.load(Ordering::SeqCst) || s.provider.lock().unwrap().is_none() {
        return;
    }
    let text = value.as_str().unwrap_or("");
    match kind {
        "user" => s.dialog.on_user(text),
        "bot" => s.dialog.on_bot(text),
        "heard" => {
            s.dialog.heard_voice();
            if !s.first_audio.swap(true,Ordering::Relaxed) {
                log::info!("ChatGPT: первый голос через {:.2} с от открытия сессии",s.created.elapsed().as_secs_f64());
                if let Some(input) = *s.last_input.lock().unwrap() {
                    log::info!("ChatGPT: от последнего речевого кадра до первого голоса {:.2} с",input.elapsed().as_secs_f64());
                }
            }
        },
        "model" => {
            s.dialog.set_model(text);
            log::info!("ChatGPT: сервер сообщил модель {}", clip_error(text));
        },
        "working" => s.dialog.working(value.as_i64().unwrap_or(0) as i32),
        "limit" => {
            LIMITED_UNTIL.store(unix() + value.as_i64().unwrap_or(60).clamp(1, 86400), Ordering::Relaxed);
        }
        "error" => {log::info!("ChatGPT: ошибка сессии: {}",clip_error(text));s.dialog.finish_turn(text);},
        "closed" => s.cancel(),
        "audio" => {
            let Ok(data) = B64.decode(text) else { return };
            if data.len() % 2 != 0 {
                return;
            }
            let pcm = data.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
            if let Some(tx) = s.out_tx.lock().unwrap().as_ref() {
                let _ = tx.try_send(pcm);
            }
        }
        _ => {}
    }
}

/// Плагин chatgpt просит бэкенд выполнить задачу голосовой модели (voice.brain).
pub async fn brain_request(params: Value) -> Result<Value, String> {
    let id = params.get("id").or(params.get("ID")).and_then(Value::as_str).unwrap_or("");
    let task = params.get("task").or(params.get("Task")).and_then(Value::as_str).unwrap_or("");
    let s = SESSIONS.lock().unwrap().get(id).and_then(Weak::upgrade).ok_or("session closed")?;
    let brain = s.brain.clone().ok_or("brain unavailable")?;
    let task = s.with_history(task);
    drop(s);
    let t0 = Instant::now();
    let r = match tokio::time::timeout(Duration::from_secs(45), brain(task)).await {
        Ok(r) => r.map(Value::String),
        Err(_) => Err("бэкенд не ответил за 45 с".into()),
    };
    match &r {
        Ok(_) => log::info!("бэкенд ответил за {:.2} с", t0.elapsed().as_secs_f64()),
        Err(e) => log::info!("бэкенд: {e} ({:.2} с)", t0.elapsed().as_secs_f64()),
    }
    r
}

/// Прогреть сетевое соединение без звонка и без передачи звука.
pub async fn warm_loop(host: Arc<Host>, store: crate::settings::Store) {
    tokio::time::sleep(Duration::from_secs(15)).await; // плагины ещё запускаются
    loop {
        if !store.is_night()
            && store.with(|s| s.active_profile_ref().engine == "chatgpt" && signed_in(&s.chatgpt))
            && let Ok(n) = host.provider("chatgpt")
        {
            let _ = n.request("voice.warm", Value::Null, Duration::from_secs(5)).await;
        }
        tokio::time::sleep(Duration::from_secs(180)).await;
    }
}

/// Уровни рассуждения, которые принимает модель (Sol 6.1 и Astra не знают "none").
pub fn effort(model: &str, effort: &str) -> String {
    let e = effort.trim().to_lowercase();
    let allowed: &[&str] = match model.trim().to_lowercase().as_str() {
        "gpt-6.2-sol" | "gpt-6.2-astra" => &["low", "medium", "high", "xhigh", "max"],
        "gpt-6.1-sol" | "gpt-6-astra" => &["low", "medium", "high"],
        _ => &["none", "low", "medium", "high"],
    };
    if allowed.contains(&e.as_str()) { e } else { "low".into() }
}

/// Текстовый запрос к модели подписки через плагин (Codex) с инструментами колонки.
/// defs — инструменты (список готовит вызывающий: заодно с контекстом, параллельно).
pub async fn ask_codex(host: &Host, auth: Value, p: &Profile, instructions: &str, text: &str, tools: Option<&Tools>, defs: Vec<Value>) -> Result<String, String> {
    ask_codex_with_history(host, auth, p, instructions, &[], text, tools, defs).await
}

pub async fn ask_codex_with_history(host: &Host, auth: Value, p: &Profile, instructions: &str, history: &[crate::llm::Turn], text: &str, tools: Option<&Tools>, mut defs: Vec<Value>) -> Result<String, String> {
    let model = if p.effective_llm_model().is_empty() { BACKEND_MODELS[0].to_string() } else { p.effective_llm_model().to_string() };
    let effort = effort(&model, &p.effort);
    if p.web_search {
        defs.push(json!({"type": "web_search"}));
    }
    let mut input: Vec<Value> = history.iter().map(|turn| {
        let part_type = if turn.role == "assistant" { "output_text" } else { "input_text" };
        json!({"type": "message", "role": turn.role, "content": [{"type": part_type, "text": turn.content}]})
    }).collect();
    input.push(json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": text}]}));
    let session = new_id();
    for _ in 0..5 {
        let n = host.provider("chatgpt")?;
        let body = json!({"model": model, "instructions": instructions, "input": input, "tools": defs, "tool_choice": "auto",
            "parallel_tool_calls": false, "reasoning": {"effort": effort}, "store": false, "stream": true, "include": []});
        let r = n.request("codex.request", json!({"auth": auth, "session": session, "body": body}), Duration::from_secs(60)).await?;
        let answer = r.get("answer").or(r.get("Answer")).and_then(Value::as_str).unwrap_or("").trim().to_string();
        let calls = r.get("calls").or(r.get("Calls")).and_then(Value::as_array).cloned().unwrap_or_default();
        if calls.is_empty() {
            return if answer.is_empty() { Err("модель ничего не ответила".into()) } else { Ok(answer) };
        }
        let Some(tools) = tools else { return Err("модель попросила инструмент, которого нет".into()) };
        if calls.len()>32{return Err("Слишком много вызовов инструментов".into());}
        let full_output=r["output"].as_array();if let Some(output)=full_output{input.extend(output.clone());}
        for c in calls {
            let name = c.get("name").and_then(Value::as_str).unwrap_or("");
            if !defs.iter().any(|d|d["name"]==name){return Err("Модель вызвала необъявленный инструмент".into());}
            let args = c.get("arguments").and_then(Value::as_str).unwrap_or("");
            let result = tokio::time::timeout(Duration::from_secs(15), tools.call(name, args)).await.unwrap_or_else(|_| "Не успел: инструмент не ответил за 15 с.".into());
            log::info!("подписка: {name}({args}) → {result}");
            if full_output.is_none(){input.push(json!({"type": "function_call", "name": c["name"], "arguments": c["arguments"], "call_id": c["call_id"]}));}
            input.push(json!({"type": "function_call_output", "call_id": c["call_id"], "output": result}));
        }
    }
    Err("модель не закончила за 5 шагов".into())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn provider_close_does_not_mask_connection_failure(){
        let closed=tokio::sync::Notify::new();
        let request=async{closed.notify_one();tokio::time::sleep(std::time::Duration::from_millis(5)).await;Err("ChatGPT HTTP 403".to_string())};
        assert_eq!(super::await_open(request,&closed).await.unwrap_err(),"ChatGPT HTTP 403");
        let cancelled=tokio::sync::Notify::new();cancelled.notify_one();
        assert!(super::await_open(std::future::pending(),&cancelled).await.unwrap_err().contains("отменён"));
    }
    #[test]
    fn efforts_and_ids() {
        assert_eq!(super::effort("gpt-6.1-sol", "none"), "low");
        assert_eq!(super::effort("gpt-6-luna", " High "), "high");
        let id = super::new_id();
        assert_eq!(id.len(), 36);
        assert_eq!(super::voice("Ember").as_deref(), Some("ember"));
        assert_eq!(super::voice(" AUBE ").as_deref(), Some("aube"));
        assert_eq!(super::voice("").as_deref(), Some("cove"));
        assert_eq!(super::voice("porto"), None);
        assert_eq!(super::voice("selva"), None);
    }

    #[test]
    fn subscription_voice_catalog_matches_validation() {
        let catalog: serde_json::Value = serde_json::from_str(include_str!("catalog.json")).unwrap();
        let groups = catalog["chatgpt_voice_groups"].as_array().unwrap();
        assert_eq!(groups.iter().map(|group| group["voices"].as_array().unwrap().len()).collect::<Vec<_>>(), [9, 14]);
        let choices: Vec<_> = groups.iter().flat_map(|group| group["voices"].as_array().unwrap())
            .map(|voice| voice["ID"].as_str().unwrap()).collect();
        assert_eq!(choices, super::VOICES);
    }
}
