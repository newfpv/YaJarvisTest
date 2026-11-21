//! Главный цикл: микрофон → слово «Джарвис» → разговор с моделью → динамик. Алиса имеет приоритет:
//! позвали её — Джарвис сразу замолкает и закрывает разговор.

use crate::{
    alice,
    audio::{Mic, Player},
    chatgpt, fastcmd,
    journal::Journal,
    led::{Leds, Mode},
    plugins::Host,
    rules,
    session::Session,
    settings::{Profile, Settings, Store},
    tools::Tools,
    wake::Listener,
};
use jarvis_dsp::Wake;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::RecvTimeoutError,
    },
    time::{Duration, Instant},
};

const RATE: usize = 16000;
pub(crate) const INTERRUPTED: &str = "озвучку прервали";
#[path="text.rs"]
mod text;
#[path="voice_preview.rs"]
pub(crate) mod voice_preview;
#[path="wake_reply.rs"]
mod wake_reply;

#[derive(Default)]
struct St {
    live: Option<Arc<dyn Session>>,
    busy: bool,
    holding: bool,   // разговора с моделью ещё нет — звук с момента слова копится в hold
    hold: Vec<i16>,  // уйдёт модели целиком, когда разговор откроется
    aborted: bool,   // «Стоп» или «Алиса», пока разговор ещё не открыт
    ring: VecDeque<i16>,  // последние ~3 с микрофона: само слово и начало фразы
    last_wake: Option<Instant>,
    last_loud: Option<Instant>,
    pending_wake: bool,
    echo: EchoWindow,
    wake_echo: Option<wake_reply::WakeEcho>,
    input_at: Option<Instant>,
}

const ECHO_TAIL: Duration = Duration::from_millis(450);

#[derive(Default)]
struct EchoWindow {
    queued_until: Option<Instant>,
    mute_until: Option<Instant>,
}

impl EchoWindow {
    fn played(&mut self, now: Instant, pcm: &[i16], rate: usize) {
        self.played_with_tail(now, pcm, rate, ECHO_TAIL);
    }

    fn played_with_tail(&mut self, now: Instant, pcm: &[i16], rate: usize, tail: Duration) {
        let start = self.queued_until.map_or(now, |end| end.max(now));
        let end = start + Duration::from_secs_f64(pcm.len() as f64 / rate as f64);
        self.queued_until = Some(end);
        if peak(pcm) > 100.0 {
            self.mute_until = Some(end + tail);
        }
    }

    fn muted(&self, now: Instant) -> bool {
        self.mute_until.is_some_and(|until| until > now)
    }

    fn stopped(&mut self, now: Instant) {
        self.stopped_with_tail(now, ECHO_TAIL);
    }

    fn stopped_with_tail(&mut self, now: Instant, tail: Duration) {
        self.queued_until = None;
        if self.muted(now) {
            self.mute_until = Some(now + tail);
        }
    }
}

pub struct Engine {
    pub store: Store,
    pub host: Arc<Host>,
    pub mic: Arc<Mic>,
    pub player: Arc<Player>,
    pub leds: Arc<Leds>,
    pub journal: Arc<Journal>,
    pub tools: Arc<Tools>,
    pub music: Arc<crate::music::Music>,
    pub listener: Arc<Listener>,
    pub data: PathBuf,
    rt: tokio::runtime::Handle,
    st: Mutex<St>,
    reset_wake: AtomicBool,
    recording_word: AtomicBool,
    training_active: AtomicBool,
    training_until: Mutex<Option<Instant>>,
    speech_epoch: AtomicU64,
    speech_notify: tokio::sync::Notify,
    saying: AtomicBool,
    previews: voice_preview::Previews,
    wake_reply_sequence: AtomicU64,
    cmds: std::sync::OnceLock<crate::cmdspot::CmdSpotter>,
}

fn same(a: &Arc<dyn Session>, b: &Arc<dyn Session>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

pub fn peak(pcm: &[i16]) -> f64 {
    pcm.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0) as f64
}

pub fn loud(pcm: &[i16]) -> bool {
    pcm.iter().any(|&v| !(-600..=600).contains(&v))
}

struct InputActivity {
    background: f32,
}

impl InputActivity {
    fn observe(&mut self, pcm: &[i16], listening: bool) -> bool {
        let mut active = false;
        for frame in pcm.chunks(RATE / 50).filter(|frame| frame.len() >= RATE / 250) {
            let level = frame.iter().map(|v| u32::from(v.unsigned_abs())).sum::<u32>() as f32 / frame.len() as f32;
            let threshold = (self.background * 2.8).max(140.0) as u16;
            let speech = level > (self.background * 2.25).max(50.0)
                && frame.iter().filter(|v| v.unsigned_abs() > threshold).count() >= frame.len() / 8;
            if !speech || !listening {
                // Learn the room's background without absorbing sustained speech into it.
                let step = (level - self.background).clamp(-self.background, self.background.max(20.0));
                self.background += step * 0.05;
            }
            active |= speech;
        }
        active
    }
}

/// Копия звука с усилением db (с ограничением по краям шкалы).
pub fn boost(pcm: &[i16], db: i64) -> Vec<i16> {
    if db <= 0 {
        return pcm.to_vec();
    }
    let k = 10f64.powf(db as f64 / 20.0);
    pcm.iter().map(|&v| (v as f64 * k).clamp(-32767.0, 32767.0) as i16).collect()
}

fn short(e: &str) -> String {
    if e.chars().count() > 160 { format!("{}…", e.chars().take(160).collect::<String>()) } else { e.into() }
}

fn command_settled(schedule: fastcmd::AliceSchedule, since: Duration, quiet_for: Duration) -> bool {
    match schedule {
        fastcmd::AliceSchedule::None => since >= Duration::from_millis(350),
        fastcmd::AliceSchedule::Ready => since >= Duration::from_millis(750)
            && (quiet_for >= Duration::from_millis(550) || since >= Duration::from_millis(2200)),
        fastcmd::AliceSchedule::Pending => since >= Duration::from_millis(2200)
            && quiet_for >= Duration::from_millis(750),
    }
}

fn lang_name(l: &str) -> &'static str {
    match l {
        "ru" => "русском",
        "en" => "английском",
        "uk" => "украинском",
        _ => "",
    }
}

/// Чего не хватает, чтобы говорить выбранным движком ("" — всё на месте).
pub fn key_missing(host: &Host, s: &Settings, engine: &str) -> String {
    let id = match engine {
        "" => return "Установите голосовой плагин и выберите его в настройках ассистента".into(),
        "live" => "openai",
        "eleven_realtime" => "elevenlabs",
        id => id,
    };
    if host.provider(id).is_err() {
        return format!("Установите и включите плагин «{id}» в «Подключениях»");
    }
    match engine {
        "chatgpt" if !chatgpt::signed_in(&s.chatgpt) => "не выполнен вход в ChatGPT — войдите в «Подключениях»".into(),
        "chatgpt" => chatgpt::voice_limit().unwrap_or_default(),
        "live" | "openai" => if s.openai_key.is_empty(){"Добавьте ключ OpenAI в «Подключениях»".into()}else{String::new()},
        "gemini" => if s.gemini_key.is_empty(){"Добавьте ключ Gemini в «Подключениях»".into()}else{String::new()},
        "grok" => if s.grok_key.is_empty(){"Добавьте ключ Grok в «Подключениях»".into()}else{String::new()},
        "elevenlabs"|"eleven_realtime"=>if s.elevenlabs_key.is_empty(){"Добавьте ключ ElevenLabs в «Подключениях»".into()}else{String::new()},
        other => format!("движок «{other}» ещё не перенесён в новое ядро — выберите ассистента на ChatGPT"),
    }
}

pub fn stt_missing(host: &Host, s: &Settings, p: &Profile) -> String {
    if !p.is_classic() {
        return key_missing(host, s, &p.engine);
    }
    match p.effective_stt_engine() {
        "openai" => key_missing(host, s, "openai"),
        "elevenlabs" => key_missing(host, s, "elevenlabs"),
        id => {
            if host.supports_stt(id) {
                String::new()
            } else {
                format!("Установите и включите плагин распознавания «{id}» в «Подключениях»")
            }
        }
    }
}

pub fn tts_missing(host: &Host, s: &Settings, p: &Profile) -> String {
    if !p.is_classic() {
        return key_missing(host, s, &p.engine);
    }
    match p.effective_tts_engine() {
        "openai" => key_missing(host, s, "openai"),
        "elevenlabs" => key_missing(host, s, "elevenlabs"),
        id => {
            if host.supports_tts(id) {
                String::new()
            } else {
                format!("Установите и включите плагин озвучки «{id}» в «Подключениях»")
            }
        }
    }
}

pub fn llm_missing(host: &Host, s: &Settings, p: &Profile) -> String {
    if !p.is_classic() {
        return key_missing(host, s, &p.engine);
    }
    match p.effective_llm_engine() {
        "" => {
            let any = (host.provider("openai").is_ok() && !s.openai_key.is_empty())
                || (host.provider("gemini").is_ok() && !s.gemini_key.is_empty())
                || (host.provider("grok").is_ok() && !s.grok_key.is_empty())
                || (host.provider("chatgpt").is_ok() && chatgpt::signed_in(&s.chatgpt));
            if any {
                String::new()
            } else {
                "Добавьте API-ключ или войдите в ChatGPT в «Подключениях»".into()
            }
        }
        "chatgpt" => {
            if host.provider("chatgpt").is_err() {
                "Установите и включите плагин «chatgpt» в «Подключениях»".into()
            } else if !chatgpt::signed_in(&s.chatgpt) {
                "не выполнен вход в ChatGPT — войдите в «Подключениях»".into()
            } else {
                String::new()
            }
        }
        id @ ("openai" | "live" | "gemini" | "grok") => key_missing(host, s, id),
        id => {
            if host.supports_chat(id) {
                String::new()
            } else {
                format!("Включите текстовый плагин «{id}» в «Подключениях»")
            }
        }
    }
}

pub fn profile_missing(host: &Host, s: &Settings, p: &Profile) -> String {
    if !p.is_classic() {
        return key_missing(host, s, &p.engine);
    }
    for msg in [stt_missing(host, s, p), llm_missing(host, s, p), tts_missing(host, s, p)] {
        if !msg.is_empty() {
            return msg;
        }
    }
    String::new()
}

pub struct WakeRecording(Arc<Engine>);

impl Drop for WakeRecording {
    fn drop(&mut self) {
        self.0.reset_wake.store(true, Ordering::Release);
        self.0.recording_word.store(false, Ordering::Release);
    }
}

impl Engine {
    pub fn recording_word(self: &Arc<Self>) -> WakeRecording {
        self.recording_word.store(true, Ordering::Release);
        WakeRecording(self.clone())
    }

    pub fn set_background_training(&self, minutes: Option<u32>) {
        self.training_active.store(minutes.is_some(), Ordering::Release);
        *self.training_until.lock().unwrap() = minutes.map(|mins| Instant::now() + Duration::from_secs(u64::from(mins) * 60 + 1));
    }
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rt: tokio::runtime::Handle,
        store: Store,
        host: Arc<Host>,
        mic: Arc<Mic>,
        player: Arc<Player>,
        leds: Arc<Leds>,
        journal: Arc<Journal>,
        tools: Arc<Tools>,
        listener: Arc<Listener>,
        data: PathBuf,
        music: Arc<crate::music::Music>,
    ) -> Arc<Engine> {
        Arc::new(Engine { store, host, mic, player, leds, journal, tools, music, listener, data, rt, st: Mutex::default(), reset_wake: AtomicBool::new(false),
            recording_word: AtomicBool::new(false), training_active: AtomicBool::new(false), training_until: Mutex::new(None),
            speech_epoch: AtomicU64::new(0), speech_notify: tokio::sync::Notify::new(), saying: AtomicBool::new(false), previews: Default::default(),
            wake_reply_sequence: AtomicU64::new(0), cmds: Default::default() })
    }

    /// Слушать микрофон в своём потоке: слово, звук в разговор, перебивание.
    pub fn run(self: &Arc<Self>, mut det: Box<dyn Wake>) {
        self.start_wake_reply_cache();
        let w = Arc::downgrade(self);
        if let Some(c) = crate::cmdspot::CmdSpotter::new(
            &self.data,
            Arc::new(move |p: String| {
                if let Some(e) = w.upgrade() {
                    e.on_command(&p);
                }
            }),
        ) {
            let _ = self.cmds.set(c);
        }
        let e = self.clone();
        let training = crate::training_io::AudioBatch::channel(&self.rt, self.host.clone());
        std::thread::Builder::new()
            .name("engine".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                let (_, rx) = e.mic.subscribe(256);
                let mut batch = crate::training_io::AudioBatch::default();
                let mut activity = InputActivity { background: 24.0 };
                for raw in rx {
                    let training_enabled = e.host.wake_training();
                    if let Some(replacement)=e.listener.take_reload(&e.data,&e.store,training_enabled){det=replacement;}
                    let (live, busy, muted_input, input) = {
                        let mut st = e.st.lock().unwrap();
                        if st.ring.capacity() < 3 * RATE + RATE / 25 {
                            st.ring.reserve(3 * RATE + RATE / 25);
                        }
                        if raw.len() >= 3 * RATE {
                            st.ring.clear();
                            st.ring.extend(&raw[raw.len() - 3 * RATE..]);
                        } else {
                            let overflow = (st.ring.len() + raw.len()).saturating_sub(3 * RATE);
                            if overflow > 0 {
                                st.ring.drain(..overflow);
                            }
                            st.ring.extend(&raw[..]);
                        }
                        if loud(&raw) {
                            st.last_loud = Some(Instant::now());
                        }
                        if st.holding && st.live.is_none() && st.hold.len() < 20 * RATE {
                            st.hold.extend_from_slice(&raw);
                        }
                        let now = Instant::now();
                        if st.wake_echo.as_ref().is_some_and(|echo| echo.expired(now)) {
                            st.wake_echo = None;
                        }
                        let input = st.wake_echo.as_ref().map_or_else(
                            || std::borrow::Cow::Borrowed(raw.as_ref()),
                            |echo| echo.filter(&raw, now),
                        );
                        (st.live.clone(), st.busy, st.echo.muted(now), input)
                    };
                    if !muted_input && activity.observe(&input, busy) {
                        let now = Instant::now();
                        e.st.lock().unwrap().input_at = Some(now);
                        if let Some(session) = &live {
                            session.dialog().heard_input(now);
                        }
                    }
                    if busy || live.is_some() {
                        e.leds.hear(peak(&raw) / 9000.0);
                    }
                    if let Some(c) = e.cmds.get() {
                        if busy || live.is_some() || e.music_playing() {
                            c.feed(&raw);
                        } else {
                            c.idle();
                        }
                    }
                    if e.reset_wake.swap(false, Ordering::Relaxed) {
                        det.reset();
                    }
                    let enabled = e.store.wake_enabled() && !e.recording_word.load(Ordering::Acquire);
                    let boost_db = e.store.mic_boost_db();
                    let ready = e.listener.ready.load(Ordering::Relaxed);
                    let hit = ready && enabled && det.feed(&raw); // поток признаков непрерывный, и во время разговора
                    let prob = det.last_prob();
                    e.listener.last_prob.store(prob.to_bits(), Ordering::Relaxed);
                    if training_enabled {
                        let collect = e.store.train_mode() && e.listener.is_default_phrase();
                        let background = if e.training_active.load(Ordering::Relaxed) {
                            let mut until = e.training_until.lock().unwrap();
                            match *until {
                                Some(t) if t > Instant::now() => true,
                                _ => {
                                    *until = None;
                                    e.training_active.store(false, Ordering::Release);
                                    false
                                }
                            }
                        } else {
                            false
                        };
                        if collect || background {
                            let limit = e.store.train_limit_mb();
                            let discard = busy || live.is_some() || e.saying.load(Ordering::Relaxed) || e.player.playing() || !ready || !enabled;
                            if let Some(frame) = batch.push(&raw, hit, discard, prob, collect, limit) {
                                let _ = training.try_send(frame);
                            }
                        } else {
                            batch.reset();
                        }
                    } else {
                        batch.reset();
                    }
                    if let Some(live) = live {
                        if muted_input {
                            // Preserve the input clock for the provider's turn detector.
                            static ZERO: [i16; 320] = [0; 320];
                            if raw.len() <= ZERO.len() {
                                live.feed(&ZERO[..raw.len()]);
                            } else {
                                live.feed(&vec![0; raw.len()]);
                            }
                        } else if boost_db > 0 {
                            live.feed(&boost(&input, boost_db));
                        } else {
                            live.feed(&input);
                        }
                        // «Джарвис» во время разговора — как у Алисы: сразу замолкает и слушает
                        let recent = e.st.lock().unwrap().last_wake.is_some_and(|t| t.elapsed() < Duration::from_millis(2500));
                        if hit && !recent {
                            e.interrupt(&live);
                        }
                        continue;
                    }
                    if busy {
                        continue;
                    }
                    if hit {
                        let p = det.wake_prob();
                        det.reset(); // хвост этого же слова не должен сработать ещё раз
                        e.trigger("слово", p);
                    }
                }
            })
            .expect("поток движка");
        self.watch_alice();
    }

    /// Системное сообщение колонки: сразу локальный сигнал (без сети и ключей — подтверждение есть всегда),
    /// затем голосом. Озвучка голосом ассистента ещё не перенесена — пока говорит штатная Алиса.
    pub fn system_notice(&self, chime: &[i16], text: &str, alice_text: &str) {
        if self.player.start(RATE).is_ok() {
            let _ = self.player.write(&vec![0; RATE / 4]); // тракт просыпается
            let _ = self.player.write(chime);
            self.player.finish(Duration::from_secs(4));
        }
        log::info!("системное сообщение: {text}");
        if !alice_text.is_empty()
            && let Err(e) = alice::text(alice_text, false)
        {
            log::info!("системное сообщение: Алиса недоступна: {e}");
        }
    }

    /// Состояние из событий музыкального модуля: без запросов на каждом кадре микрофона.
    fn music_playing(&self) -> bool {
        self.music.playing()
    }

    /// Встроенный распознаватель услышал команду, пока Джарвис занят.
    fn on_command(&self, phrase: &str) {
        let w = phrase.to_lowercase();
        let has = |s: &str| w.contains(s);
        if !self.talking() {
            if !self.music_playing() { return; }
            if has("стоп") || has("пауза") {
                let op = if has("пауза") { "pause" } else { "stop" };
                let _ = self.rt.block_on(self.music.transport(&self.host, op, serde_json::Value::Null));
                return;
            }
            if has("громче") || has("тише") {
                let level = crate::speakervol::speaker_volume().unwrap_or(5) + if has("тише") { -1 } else { 1 };
                let _ = crate::speakervol::set_alice_volume(level.clamp(0,10));
            }
            return;
        }
        if fastcmd::is_stop(phrase) || has("хватит") || has("стоп") {
            log::info!("Джарвис: «{phrase}» — разговор закрыт, музыка не тронута (встроенный распознаватель)");
            self.end_chat(&format!("сказали «{phrase}»"));
            self.journal.note("стоп", phrase, "");
        } else if has("пауза") {
            log::info!("Джарвис: «{phrase}» — стоп (встроенный распознаватель)");
            self.stop(&format!("сказали «{phrase}»"));
            self.journal.note("стоп", phrase, "");
        } else if has("громче") || has("тише") {
            let step = if has("тише") { -10 } else { 10 };
            let mut v = 0;
            // громкость голоса активного ассистента
            let _ = self.store.update(|s| -> Result<(), std::io::Error> {
                let id = s.active_profile().id.clone();
                if let Some(p) = s.profiles.iter_mut().find(|p| p.id == id) {
                    p.volume = (p.volume + step).clamp(10, 100);
                    v = p.volume;
                }
                Ok(())
            });
            log::info!("Джарвис: «{phrase}» — громкость {v}% (встроенный распознаватель)");
        } else {
            log::info!("Джарвис: распознаватель услышал «{phrase}» — не моя команда");
        }
    }

    /// Идёт ли разговор (или он открывается).
    pub fn talking(&self) -> bool {
        let st = self.st.lock().unwrap();
        st.busy || st.live.is_some() || self.saying.load(Ordering::Relaxed)
    }

    /// Начать разговор (по слову, по кнопке в настройках).
    pub fn trigger(self: &Arc<Self>, source: &str, prob: f32) {
        if source == "слово" && self.recording_word.load(Ordering::Acquire) { return; }
        let clip = {
            let mut st = self.st.lock().unwrap();
            if st.busy || st.last_wake.is_some_and(|t| t.elapsed() < Duration::from_millis(1500)) {
                return;
            }
            st.busy = true;
            st.echo = EchoWindow::default();
            st.wake_echo = None;
            st.last_wake = Some(Instant::now());
            st.aborted = false;
            st.hold.clear();
            st.holding = true;
            st.ring.make_contiguous().to_vec()
        };
        let by_word = source == "слово";
        // как у Алисы: услышал слово — сразу кольцо и разговор; проверка имени расшифровкой — по галочке
        let sure = !by_word || prob >= 0.9 || !self.store.with(|s| s.wake_check);
        if sure {
            self.leds.set(Mode::Listening);
        }
        self.reset_wake.store(true, Ordering::Relaxed);
        if by_word {
            log::info!("Джарвис: активация ({source}, уверенность {prob:.2})");
        } else {
            log::info!("Джарвис: активация ({source})");
        }
        let e = self.clone();
        std::thread::Builder::new()
            .name("converse".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                struct ConverseGuard<'a> {
                    engine: &'a Engine,
                    completed: bool,
                }
                impl Drop for ConverseGuard<'_> {
                    fn drop(&mut self) {
                        if !self.completed {
                            self.engine.player.stop(true);
                            if let Some(l) = self.engine.st.lock().unwrap_or_else(|p| p.into_inner()).live.take() {
                                l.close();
                            }
                            let mut st = self.engine.st.lock().unwrap_or_else(|p| p.into_inner());
                            st.pending_wake = false;
                            st.busy = false;
                            st.live = None;
                            st.aborted = false;
                            st.hold.clear();
                            st.holding = false;
                            st.echo = EchoWindow::default();
                            st.wake_echo = None;
                            self.engine.reset_wake.store(true, Ordering::Relaxed);
                            self.engine.leds.set(Mode::Off);
                        }
                    }
                }
                let mut guard = ConverseGuard { engine: &e, completed: false };
                let preroll = if by_word { clip.clone() } else { vec![] };
                e.converse(by_word, sure, preroll, clip);
                guard.completed = true;
                let queued = {
                    let mut st = e.st.lock().unwrap();
                    let q = st.pending_wake;
                    st.pending_wake = false;
                    st.busy = false;
                    st.live = None;
                    st.aborted = false;
                    st.hold.clear();
                    st.holding = false;
                    st.echo = EchoWindow::default();
                    st.wake_echo = None;
                    q
                };
                e.reset_wake.store(true, Ordering::Relaxed);
                e.leds.set(Mode::Off);
                if queued {
                    e.trigger("слово", 1.0);
                }
            })
            .expect("поток разговора");
    }

    fn instructions(&self, s: &Settings, p: &Profile) -> String {
        let phrase = self.listener.phrase();
        let mut text = s.prompt_text(&p.prompt);
        if p.prompt == "jarvis" || p.prompt == "bro" {
            text = text.replacen("Ты — Джарвис", &format!("Ты — «{phrase}»"), 1);
        }
        let persona = text.clone();
        let n = lang_name(&s.language);
        if !n.is_empty() {
            text += &format!(
                "\n\nХозяин всегда говорит на {n} языке. Понимай его речь как {n}, даже если она звучит неразборчиво или похоже на другой язык, и отвечай только на {n}."
            );
        }
        if !s.city.is_empty() {
            text += &format!("\n\nХозяин живёт в городе {} — погода, время и новости по умолчанию для него.", s.city);
        }
        text += "\n\nЕсли тебя просят замолчать или закончить разговор, не отвечай вслух и не вызывай инструменты. Останавливай музыку только по явной просьбе про музыку.\n\nКогда для ответа нужен инструмент (поиск в интернете, погода, умный дом, музыка, сложная задача), \
можно ОДИН раз за ответ коротко сказать вслух живую фразу — «Секунду, гляну», «Сейчас поищу» — и сразу вызывать инструмент. \
Если вызываешь несколько инструментов подряд или повторяешь вызов — больше никаких таких фраз: молча дождись результата и скажи итог. \
На простые вопросы и быстрые команды (включить свет, громкость) отвечай сразу, без таких фраз.";
        if s.is_night() {
            text += "\n\nСейчас ночь, в доме могут спать: отвечай коротко и спокойно, без длинных списков.";
        }
        if p.engine == "gemini" {
            text += "\n\nЕсли человек только позвал тебя или пока думает и тянет «эээ», «ааа», дождись продолжения. Отвечай, когда он закончил мысль.";
            if p.web_search {
                text += " Используй Google Search, если просят поискать или нужны актуальные сведения. Если поиск ничего не нашёл, скажи об этом.";
            }
        }
        match &self.tools.memory {
            // общая память разговора: у всех ассистентов, переживает перезапуск
            Some(m) => text += &crate::memory::instructions(m, p.engine == "chatgpt"),
            None => {
                let h = self.journal.history();
                if !h.is_empty() {
                    let ctx: Vec<_> = h.iter().map(|t| serde_json::json!({"user": t.text, "jarvis": t.answer})).collect();
                    text += &format!("\n\nПредыдущие реплики разговора: {}", serde_json::Value::Array(ctx));
                }
            }
        }
        if p.engine == "chatgpt" && self.tools.memory.is_some() {
            // напоминание после истории: в длинном чате модель иначе сползает в обычный стиль ChatGPT
            let persona = persona.trim();
            let persona = if persona.chars().count() > 2500 { format!("{}…", persona.chars().take(2500).collect::<String>()) } else { persona.to_owned() };
            text += &format!(
                "\n\nГлавное, не забывай: ты — голосовой ассистент «{phrase}» внутри умной колонки Яндекс Станция у хозяина дома, \
а не ChatGPT в приложении. Говоришь вслух, коротко, своим характером. Что делать на колонке (музыка, YouTube, свет, \
поиск, погода, таймеры, будильники, напоминания) — передаёшь бэкенду, он делает: таймеры и будильники ставит Алиса \
на этой же колонке, так что не говори, что не умеешь. Твой характер:\n{persona}"
            );
        }
        text + rules::SPEAKER_CONTENT_RULES + rules::SPEAKER_WAIT_RULES + &rules::identity(&phrase)
    }

    /// Кто думает и управляет домом, когда голосовая модель передаёт задачу: модель подписки ChatGPT.
    fn brain(&self, p: &Profile) -> chatgpt::Brain {
        let (host, tools, p) = (self.host.clone(), self.tools.clone(), p.clone());
        Arc::new(move |task: String| {
            let (host, tools, p) = (host.clone(), tools.clone(), p.clone());
            Box::pin(async move {
                // вход и подготовка дома параллельно; инструменты зависят от загруженного каталога
                let t0 = Instant::now();
                let ((ctx, defs), auth) = tokio::join!(tools.prepare(), async {
                    host.provider("chatgpt")?.request("auth.token", serde_json::Value::Null, Duration::from_secs(20)).await
                });
                log::info!("бэкенд: контекст и инструменты за {:.2} с", t0.elapsed().as_secs_f64());
                let instr = format!("{}{ctx}", rules::BACKEND_RULES);
                chatgpt::ask_codex(&host, auth?, &p, &instr, &task, Some(&tools), defs).await
            })
        })
    }

    fn chat_thread(&self) -> String {
        if let Some(m) = &self.tools.memory {
            return m.thread();
        }
        std::fs::read(self.data.join("chatgpt-chat.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v.get("id").and_then(|x| x.as_str()).map(String::from))
            .unwrap_or_default()
    }

    /// Открыть разговор с моделью тем движком, который выбран у ассистента.
    fn new_session(&self, s: &Settings, p: &Profile) -> Arc<dyn Session> {
        if p.is_classic(){return crate::classic_voice::Classic::open(&self.rt,self.host.clone(),self.tools.clone(),self.journal.clone(),s.clone(),p.clone(),self.instructions(s,p),self.listener.phrase());}
        if ["","live","openai","gemini","grok","eleven_realtime"].contains(&p.engine.as_str()) {
            return crate::cloud_voice::Voice::open(&self.rt,self.host.clone(),Some(self.tools.clone()),self.journal.clone(),s.clone(),p.clone(),self.instructions(s,p));
        }
        chatgpt::ChatGptSession::open(
            &self.rt,
            chatgpt::Open {
                host: self.host.clone(),
                voice: p.voice.clone(),
                thread: self.chat_thread().into(),
                instructions: self.instructions(s, p),
                brain: Some(self.brain(p)),
                journal: self.journal.clone(),
                memory: self.tools.memory.clone(),
            },
        )
    }

    /// Выбранная текстовая модель памяти или очередь подключённых провайдеров в режиме автовыбора.
    pub fn summarizer(&self) -> crate::memory::Summarizer {
        let (host, store, tools) = (self.host.clone(), self.store.clone(), self.tools.clone());
        Arc::new(move |instructions: String, text: String| {
            let (host, store, tools) = (host.clone(), store.clone(), tools.clone());
            Box::pin(async move {
                let s = store.get();
                let mut error = crate::memory::NO_SUMMARIZER.to_string();
                for provider in crate::llm::memory_candidates(&host, &s) {
                    let profile = Profile {
                        engine: provider.into(), backend_model: crate::llm::memory_model(&s, provider).into(),
                        effort: "low".into(), web_search: false, ..Default::default()
                    };
                    match crate::llm::ask(&host, &s, &profile, &instructions, &text, &tools, vec![]).await {
                        Ok(summary) => return Ok(summary),
                        Err(why) => error = why,
                    }
                }
                Err(error)
            })
        })
    }

    fn start_voice(&self, sess: &dyn Session) -> Result<(), String> {
        self.player.start(sess.rate()).map_err(|e| e.to_string())?;
        self.player.set_boost(sess.out_boost());
        Ok(())
    }

    fn earcon(&self, ok: bool) {
        if self.player.start(RATE).is_ok() {
            let _ = self.player.write(&fastcmd::earcon(ok));
            self.player.finish(Duration::from_secs(2));
        }
    }

    /// Один разговор от слова (или кнопки) до тишины.
    fn converse(self: &Arc<Self>, by_word: bool, sure: bool, preroll: Vec<i16>, clip: Vec<i16>) {
        let s = self.store.get();
        let p = s.active_profile();
        let missing = profile_missing(&self.host, &s, &p);
        if !missing.is_empty() {
            self.journal.fail(&missing);
            return;
        }
        {
            let t = self.tools.clone();
            self.rt.spawn(async move { t.prewarm().await });
        }
        let idle = if p.idle_s < 2.0 { 8.0 } else { p.idle_s };
        let mut sess: Option<Arc<dyn Session>> = None;
        let mut out = None;
        let mut speaking = false;
        let mut checked = 0i64;
        let mut checked_text = String::new();
        let opening_at = Instant::now();
        let mut ready_at = Instant::now();
        let mut last_voice: Option<Instant> = None;
        let _ = (sure, &clip);

        // открыть разговор с моделью и отдать ему весь звук с момента слова
        let open = |sess: &mut Option<Arc<dyn Session>>, out: &mut Option<std::sync::mpsc::Receiver<Vec<i16>>>, ready_at: &mut Instant| -> bool {
            if by_word {
                self.leds.set(Mode::Thinking); // соединяюсь с моделью — бегущий огонёк
            }
            let x = self.new_session(&s, &p);
            let reply = if by_word && sure && s.wake_reply_enabled { self.cached_wake_reply(&p) } else { None };
            if reply.is_some() {
                // Network setup is already in progress while giving a joined phrase a head start.
                std::thread::sleep(wake_reply::FOLLOW_ON_GAP);
            }
            let reply = {
                let mut st = self.st.lock().unwrap();
                let reply = reply.filter(|_| !st.aborted && st.input_at.is_none_or(|at| at < opening_at));
                // The wake word has already been detected. Avoid replaying seconds of
                // old audio into the subscription call before the first real question.
                let before = if matches!(p.engine.as_str(), "chatgpt" | "gemini") {
                    preroll.len().saturating_sub(RATE * 4 / 5)
                } else { 0 };
                if reply.is_none() {
                    let mut first = preroll[before..].to_vec();
                    first.extend_from_slice(&st.hold);
                    x.feed(&boost(&first, s.mic_boost_db));
                }
                st.hold.clear();
                st.holding = false;
                st.live = Some(x.clone());
                reply
            };
            *sess = Some(x.clone());
            if let Some(sample) = reply {
                self.play_wake_reply(&sample, &p.engine, opening_at);
            }
            if self.st.lock().unwrap().aborted {
                return false;
            }
            if let Err(e) = x.ready(Duration::from_secs(12)) {
                log::info!("разговор не открылся: {e}");
                x.dialog().finish_turn(&format!("разговор не открылся: {}", short(&e)));
                return false;
            }
            if let Err(e) = self.start_voice(&*x) {
                x.dialog().finish_turn(&format!("динамик: {e}"));
                return false;
            }
            *out = x.take_audio();
            *ready_at = Instant::now();
            if p.engine == "gemini" {
                log::info!("Gemini: разговор готов за {:.2} с", opening_at.elapsed().as_secs_f64());
            }
            x.dialog().touch();
            self.leds.set(Mode::Listening);
            true
        };
        let close = |sess: &Option<Arc<dyn Session>>| {
            if let Some(x) = sess {
                x.close();
            }
        };
        if !open(&mut sess, &mut out, &mut ready_at) {
            close(&sess);
            return;
        }
        let x = sess.clone().unwrap();
        let out = out.unwrap();
        let mut tick = Instant::now();
        loop {
            match out.recv_timeout(Duration::from_millis(20)) {
                Err(RecvTimeoutError::Disconnected) => {
                    self.player.stop(true);
                    x.dialog().finish_turn("");
                    return close(&sess);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Ok(pcm) => {
                    if !x.dialog().muted() {
                        if !self.player.playing() && loud(&pcm) {
                            let _ = self.start_voice(&*x); // после перебивания плеер был остановлен
                        }
                        if self.player.write(&pcm).is_ok() && p.engine == "gemini" {
                            self.st.lock().unwrap().echo.played(Instant::now(), &pcm, x.rate());
                        }
                        if loud(&pcm) {
                            if p.engine == "gemini" && last_voice.is_none() {
                                log::info!("Gemini: первый голос через {:.2} с от открытия разговора", opening_at.elapsed().as_secs_f64());
                            }
                            last_voice = Some(Instant::now());
                        }
                        self.leds.level(peak(&pcm) / 9000.0);
                        if loud(&pcm) != speaking {
                            speaking = !speaking;
                            self.leds.set(if speaking { Mode::Speaking } else { Mode::Listening });
                        }
                    }
                }
            }
            if x.interrupted() {
                while out.try_recv().is_ok(){}
                self.player.stop(true);
                self.st.lock().unwrap().echo.stopped(Instant::now());
                speaking = false;
                self.leds.set(Mode::Listening);
            }
            if tick.elapsed() < Duration::from_millis(100) {
                continue;
            }
            tick = Instant::now();
            let stopped = {
                let st = self.st.lock().unwrap();
                st.aborted || !st.live.as_ref().is_some_and(|l| same(l, &x))
            };
            if stopped {
                self.player.stop(true);
                x.dialog().finish_turn("");
                return close(&sess);
            }
            // разговор открыт сразу по слову, а человек так ничего и не сказал — слово сработало зря
            if by_word && checked == 0 && x.dialog().user_turns() == 0
                && ready_at.elapsed() > Duration::from_secs(5)
                && !last_voice.is_some_and(|t| t > ready_at)
                && !x.dialog().heard_input_since(opening_at)
            {
                log::info!("Джарвис: ложное срабатывание (никто ничего не сказал) — молчу");
                self.player.stop(true);
                x.dialog().annotate("ложное срабатывание", "");
                x.dialog().finish_turn("");
                return close(&sess);
            }
            let (id, text, since) = x.dialog().utterance();
            if id != 0 && (id != checked || text != checked_text) && !x.handles_commands() {
                let command = fastcmd::command_for_phrase(&text, &self.listener.phrase());
                let question = fastcmd::clarification(&command);
                let schedule = if question.is_some() { fastcmd::AliceSchedule::Pending } else { fastcmd::alice_schedule(&command) };
                let quiet_for = self.st.lock().unwrap().last_loud.map(|time| time.elapsed()).unwrap_or(Duration::MAX);
                if command_settled(schedule, since, quiet_for) {
                    checked = id;
                    checked_text.clone_from(&text);
                    if let Some(question) = question {
                        x.dialog().annotate("уточнение", question);
                        x.dialog().finish_turn("");
                        x.dialog().unmute();
                        if let Err(error) = x.say(question) {
                            log::info!("уточнение команды: {error}");
                        }
                    } else if self.handle(&x, &text, "модель") {
                        return close(&sess);
                    }
                }
            }
            if x.dialog().idle(idle) {
                log::info!("разговор: тишина {idle:.0} с — окончен");
                self.player.finish(Duration::from_secs(3));
                x.dialog().finish_turn("");
                return close(&sess);
            }
        }
    }

    /// Законченная фраза человека: «стоп», команды плагинов, Алисе и умному дому — сами, без модели.
    /// true — разговор закончен.
    fn handle(&self, x: &Arc<dyn Session>, text: &str, from: &str) -> bool {
        let cmd = fastcmd::command_for_phrase(text, &self.listener.phrase());
        let quiet = || {
            x.dialog().mute();
            self.player.stop(true);
        };
        let note = |route: &str, answer: &str| {
            x.dialog().annotate(route, answer);
            if x.dialog().turn_text().is_empty() {
                x.dialog().set_text(text);
            }
            x.dialog().finish_turn("");
        };
        if fastcmd::is_stop(&cmd) {
            quiet();
            log::info!("Джарвис: «{text}» — разговор закрыт, музыка не тронута ({from})");
            note("стоп", "");
            return true;
        }
        if fastcmd::youtube_request(&cmd) {
            return false;
        }
        let rt = &self.rt;
        if let Some(answer) = rt.block_on(self.music.control(&self.host, &cmd)) {
            quiet();
            note("музыка", &answer);
            (self.say_hook())(answer);
            return true;
        }
        if let Some(answer) = rt.block_on(async { tokio::time::timeout(Duration::from_secs(20), self.host.command(&cmd)).await.ok().flatten() }) {
            quiet();
            log::info!("Джарвис: «{text}» → плагин ({from})");
            note("плагин", &answer);
            if !answer.trim().is_empty() {
                (self.say_hook())(answer);
            } else {
                self.earcon(true);
            }
            return true;
        }
        if let Some(answer) = rt.block_on(self.tools.ha_vacuum(&cmd)) {
            quiet();
            note("Home Assistant", &answer);
            (self.say_hook())(answer);
            return true;
        }
        let t0 = Instant::now();
        if let Some(answer) = rt.block_on(self.tools.smartlife_fast(&cmd)) {
            quiet();
            log::info!("Джарвис: «{text}» → Smart Life ({:.2} с, {from})", t0.elapsed().as_secs_f64());
            note("Smart Life", &answer);
            self.earcon(answer == "Готово.");
            return true;
        }
        // таймеры, будильники, напоминания, музыка — умеет только штатная Алиса; команды умного дома
        // отдаём ей, если так выбрано в настройках
        let schedule = fastcmd::alice_schedule(&cmd);
        let mut to_alice = fastcmd::alice_skill_for(&cmd, self.store.with(|s| s.use_home("yandexmusic")));
        if to_alice.is_none() && self.store.with(|s| s.use_home("alice")) {
            to_alice = rt.block_on(self.tools.home_command(&cmd));
        }
        if let Some(a) = to_alice {
            quiet();
            match alice::text(&a, false) {
                Ok(()) => {
                    log::info!("Джарвис: «{a}» → Алисе ({from}, {:.2} с)", t0.elapsed().as_secs_f64());
                    note("умный дом → Алиса", &format!("передал Алисе: «{a}»"));
                    return true;
                }
                Err(e) if schedule == fastcmd::AliceSchedule::Ready => {
                    note("уточнение", &format!("Не удалось передать Алисе: {e}"));
                    x.dialog().unmute();
                    let _ = x.say("Не удалось передать команду Алисе.");
                    return false;
                }
                Err(e) => log::info!("Алиса: {e} — выполняю сам"),
            }
        }
        if self.store.with(|s| s.use_home("yandex"))
            && let Some(answer) = rt.block_on(self.tools.fast(&cmd))
        {
            quiet();
            log::info!("Джарвис: быстрая команда «{text}» → {answer} (Яндекс {:.2} с, {from})", t0.elapsed().as_secs_f64());
            note("умный дом (быстро)", &answer);
            self.earcon(answer == "Готово.");
            return true;
        }
        false
    }

    /// Сказать вслух просьбу плагина. Пока идёт разговор — не перебиваем его.
    pub fn say_text(&self, text: &str) {
        if self.talking() {
            log::info!("плагин просит сказать «{}» — идёт разговор, пропускаю", crate::plugins::clip(text, 120));
            return;
        }
        (self.say_hook())(text.to_string());
    }

    pub async fn offer_next(self: Arc<Self>, id: String) {
        if self.talking() || self.music.playing() { return; }
        let epoch = self.speech_epoch.load(Ordering::SeqCst);
        let result = self.music.call(&self.host, "related", serde_json::json!({"expected":id})).await;
        let Ok(serde_json::Value::String(answer)) = result else { return };
        if answer.is_empty() || self.talking() || self.music.playing()
            || self.speech_epoch.load(Ordering::SeqCst) != epoch { return; }
        self.journal.note("следующий ролик", "ролик закончился", &answer);
        let _ = tokio::task::spawn_blocking(move || self.say_text(&answer)).await;
    }

    /// Озвучка короткого ответа (плагины, команды, музыка); не вышло — звуком «готово».
    fn say_hook(&self) -> impl Fn(String) + '_ {
        move |answer: String| {
            log::info!("Джарвис: ответ «{}»", crate::plugins::clip(&answer, 200));
            if let Err(e) = self.say_aloud(&answer, Duration::from_secs(25)) {
                log::info!("озвучка: {e}");
                if e != INTERRUPTED {
                    self.earcon(true);
                }
            }
        }
    }

    /// Произнести готовый текст голосом активного ассистента: отдельный короткий звонок, где модель
    /// только читает текст. Блокирует до конца фразы (не дольше limit). Вызывать не из потоков tokio.
    pub fn say_aloud(&self, text: &str, limit: Duration) -> Result<(), String> {
        if self.saying.compare_exchange(false, true, Ordering::SeqCst, Ordering::Relaxed).is_err() {
            return Err(INTERRUPTED.into());
        }
        struct Speaking<'a>(&'a AtomicBool);
        impl Drop for Speaking<'_> { fn drop(&mut self) { self.0.store(false, Ordering::SeqCst); } }
        let _speaking = Speaking(&self.saying);
        let epoch = self.speech_epoch.load(Ordering::SeqCst);
        let s = self.store.get();
        let p = s.active_profile();
        let missing = tts_missing(&self.host, &s, &p);
        if !missing.is_empty() {
            return Err(missing);
        }
        let t0 = Instant::now();
        let sess: Arc<dyn Session> = if p.is_classic() {
            crate::classic_voice::Classic::open(
                &self.rt,
                self.host.clone(),
                self.tools.clone(),
                Arc::new(Journal::default()),
                s.clone(),
                p.clone(),
                "Ты озвучиваешь готовый текст. Произнеси ровно то, что тебе дали, ничего не добавляя, и замолчи.".into(),
                self.listener.phrase(),
            )
        } else {
            chatgpt::ChatGptSession::open(
                &self.rt,
                chatgpt::Open {
                    host: self.host.clone(),
                    voice: p.voice.clone(),
                    thread: self.chat_thread().into(),
                    instructions: "Ты озвучиваешь готовый текст. Произнеси ровно то, что тебе дали, ничего не добавляя, и замолчи.".into(),
                    brain: None,
                    journal: Arc::new(Journal::default()),
                    memory: None,
                },
            )
        };
        let res = self.play_said(&*sess, text, limit, t0, epoch);
        sess.close();
        res
    }

    fn play_said(&self, sess: &dyn Session, text: &str, limit: Duration, t0: Instant, epoch: u64) -> Result<(), String> {
        sess.ready(Duration::from_secs(10).min(limit))?;
        if self.speech_epoch.load(Ordering::SeqCst) != epoch { return Err(INTERRUPTED.into()); }
        sess.say(text)?;
        let out = sess.take_audio().ok_or("нет звука")?;
        let (mut started, mut heard) = (false, false);
        let mut quiet_since = Instant::now();
        while quiet_since.elapsed() < Duration::from_millis(1200) || !heard {
            if self.speech_epoch.load(Ordering::SeqCst) != epoch {
                // «Стоп» из настроек или позвали Алису
                if started {
                    self.player.stop(true);
                }
                return Err(INTERRUPTED.into());
            }
            if t0.elapsed() > limit {
                if started {
                    self.player.stop(true);
                }
                return Err("голос не успел прозвучать".into());
            }
            match out.recv_timeout(Duration::from_millis(100)) {
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    if started {
                        self.player.finish(Duration::from_secs(3));
                    }
                    return if heard { Ok(()) } else { Err("голос не прозвучал".into()) };
                }
                Ok(pcm) => {
                    if !started {
                        // плеер — только с первым звуком, а не пока модель думает
                        self.start_voice(sess)?;
                        self.leds.set(Mode::Speaking);
                        let _ = self.player.write(&vec![0; sess.rate() / 4]);
                        started = true;
                    }
                    if self.player.write(&pcm).is_err() && !self.player.playing() {
                        return Err(INTERRUPTED.into()); // плеер забрали — нас перебили
                    }
                    self.leds.level(peak(&pcm) / 9000.0);
                    if loud(&pcm) {
                        heard = true;
                        quiet_since = Instant::now();
                    }
                }
            }
        }
        self.player.finish(Duration::from_secs(3));
        Ok(())
    }

    /// «Джарвис» посреди ответа: замолчать сразу, слушать новую фразу.
    fn interrupt(&self, sess: &Arc<dyn Session>) {
        log::info!("Джарвис: перебили словом — замолкаю");
        sess.dialog().mute();
        self.player.stop(true);
        self.st.lock().unwrap().echo.stopped(Instant::now());
        self.leds.set(Mode::Listening);
    }

    /// Прервать разговор и музыку (Алиса, кнопка «Стоп» в настройках).
    pub fn stop(&self, reason: &str) {
        self.end_chat(reason);
        self.rt.block_on(self.music.stop_all(&self.host));
    }

    /// Закончить только разговор: музыка, если играет, играет дальше. Блокирует до 3 с.
    pub fn end_chat(&self, reason: &str) {
        self.speech_epoch.fetch_add(1, Ordering::SeqCst);
        self.speech_notify.notify_waiters();
        let sess = {
            let mut st = self.st.lock().unwrap();
            let sess = st.live.take();
            if st.busy || sess.is_some() {
                st.aborted = true;
            }
            if sess.is_none() && st.busy {
                log::info!("Джарвис: остановлен до начала разговора ({reason})");
            }
            sess
        };
        if let Some(sess) = sess {
            log::info!("Джарвис: разговор прерван ({reason})");
            self.player.stop(true);
            sess.close();
        }
    }

    /// Журнал maind: штатный споттер «Алиса» — Джарвис уступает.
    fn watch_alice(self: &Arc<Self>) {
        let w1 = Arc::downgrade(self);
        let w2 = Arc::downgrade(self);
        alice::follow_logs(vec![
            (alice::MAIND_LOG, Box::new(move |line| {
                let Some(e) = w1.upgrade() else { return };
                if line.contains("onPhraseSpotted") && line.contains("alice_capability") && e.talking() {
                    e.end_chat("позвали Алису");
                    let _ = e.rt.block_on(e.music.call(&e.host, "stop", serde_json::Value::Null));
                }
                if line.contains("Toggle play/pause") {
                    e.rt.block_on(e.music.pause_button(&e.host));
                }
                let (started, id) = e.music.alice.lock().unwrap().main_log(line);
                if started || id.is_some() {
                    e.rt.clone().spawn(async move {
                        if started { e.music.alice_started(&e.host).await; }
                        if let Some(id) = id && let Some(track) = crate::alice_music::lookup(&id).await {
                            e.music.alice.lock().unwrap().set_track(&id, track);
                        }
                    });
                }
            })),
            ("/data/quasar/daemons_logs/mediad.log", Box::new(move |line| {
                let Some(e) = w2.upgrade() else { return };
                let started = e.music.alice.lock().unwrap().media_log(line);
                if started { e.rt.block_on(e.music.alice_started(&e.host)); }
            })),
        ]);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn waits_for_the_duration_after_a_transcript_pause() {
        use super::{Duration, command_settled, fastcmd::AliceSchedule::{Pending, Ready}};
        let pause = Duration::from_millis;
        assert!(!command_settled(Pending, pause(400), pause(400)));
        assert!(!command_settled(Pending, pause(1800), pause(1100)));
        assert!(!command_settled(Ready, pause(750), pause(100)));
        assert!(command_settled(Ready, pause(800), pause(650)));
        assert!(command_settled(Pending, pause(2300), pause(800)));
    }

    #[test]
    fn helpers() {
        assert!(super::loud(&[0, 700]) && !super::loud(&[600, -600]));
        assert_eq!(super::boost(&[1000], 6)[0], 1995);
        assert_eq!(super::peak(&[-32768, 5]), 32768.0);
    }

    #[test]
    fn input_activity_detects_quiet_speech_and_ignores_ambient_noise_and_spikes() {
        let mut detector = super::InputActivity { background: 24.0 };
        let background: Vec<i16> = (0..320).map(|i| if i % 2 == 0 { 36 } else { -36 }).collect();
        for _ in 0..100 {
            assert!(!detector.observe(&background, false));
        }
        let mut spike = background.clone();
        spike[80] = i16::MAX;
        assert!(!detector.observe(&spike, true));
        let quiet_speech: Vec<i16> = (0..320).map(|i| {
            ((i as f32 * std::f32::consts::TAU * 180.0 / 16_000.0).sin() * 180.0) as i16
        }).collect();
        assert!(!super::loud(&quiet_speech));
        for _ in 0..500 {
            assert!(detector.observe(&quiet_speech, true));
        }
        assert!(!detector.observe(&background, true));
    }

    #[test]
    fn echo_guard_covers_queued_audio_and_reopens_after_explicit_interrupt() {
        let now = std::time::Instant::now();
        let ms = std::time::Duration::from_millis;
        let mut echo = super::EchoWindow::default();
        assert!(!echo.muted(now));
        let half_second = vec![1500; 12_000];
        echo.played(now, &half_second, 24_000);
        echo.played(now + ms(10), &half_second, 24_000);
        assert!(echo.muted(now + ms(1200)));
        assert!(!echo.muted(now + ms(1451)));
        echo.stopped(now + ms(300));
        assert!(echo.muted(now + ms(749)));
        assert!(!echo.muted(now + ms(751)));
    }

    #[cfg(feature = "lua")]
    #[tokio::test]
    async fn profile_missing_checks_classic_stages_independently() {
        let dir = std::env::temp_dir().join(format!("jarvis-pm-{}", crate::settings::rand_hex(6)));
        for (id, provides, src) in [
            ("stt-p", "stt", "return {transcribe=function() return {text=''} end}"),
            ("llm-p", "llm", "return {chat=function() return 'ok' end}"),
            ("tts-p", "tts", "return {speak=function() return {audio='',rate=24000} end}"),
        ] {
            let p = dir.join("plugins").join(id);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("plugin.json"), serde_json::json!({"id":id,"name":id,"version":"1","kind":"lua","category":"assistant","main":"main.lua","provides":[provides]}).to_string()).unwrap();
            std::fs::write(p.join("main.lua"), src).unwrap();
        }
        let host = super::Host::new(dir.clone(), crate::plugins::Hooks::default());
        host.load().await;
        let s = super::Settings::default();
        let mut p = super::Profile {
            mode: "classic".into(),
            engine: "classic".into(),
            stt_engine: "stt-p".into(),
            llm_engine: "llm-p".into(),
            tts_engine: "tts-p".into(),
            ..Default::default()
        };
        assert_eq!(super::profile_missing(&host, &s, &p), "");
        p.tts_engine = "missing-tts".into();
        assert!(super::profile_missing(&host, &s, &p).contains("missing-tts"));
        p.tts_engine = "tts-p".into();
        p.llm_engine = "missing-llm".into();
        assert!(super::profile_missing(&host, &s, &p).contains("missing-llm"));
        p.llm_engine = "llm-p".into();
        p.stt_engine = "missing-stt".into();
        assert!(super::profile_missing(&host, &s, &p).contains("missing-stt"));
        host.stop_all().await;
        std::fs::remove_dir_all(dir).unwrap();
    }
}
