mod account;
mod media;
mod player_script;
mod remote;

use account::{Account, COOKIES};
use base64::{Engine, engine::general_purpose::STANDARD};
use jarvis_home_common::{Host, load, save, text};
use remote::{Remote, Stream, Track, valid_id, video_id};
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{Mutex as AsyncMutex, Notify, OnceCell, oneshot},
    task::JoinHandle,
};

const HISTORY: &str = "youtube-progress.json";

#[derive(Clone, Serialize, Deserialize)]
struct Saved {
    track: Track,
    pos: f64,
    dur: f64,
    updated: String,
    complete: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Loading,
    Playing,
    Paused,
}

#[derive(Default)]
struct State {
    phase: Phase,
    generation: u64,
    track: Track,
    pos: f64,
    stream: Option<Stream>,
    queue: VecDeque<Track>,
    related: VecDeque<Track>,
    history: BTreeMap<String, Saved>,
    saved: Option<Instant>,
    proposal: Option<(Instant, VecDeque<Track>)>,
    personal: bool,
    error: String,
    task: Option<JoinHandle<()>>,
}

impl State {
    fn dur(&self) -> f64 {
        self.stream.as_ref().map_or(0.0, |v| v.duration)
    }
    fn proposals(&self) -> bool {
        self.proposal
            .as_ref()
            .is_some_and(|(until, tracks)| Instant::now() < *until && !tracks.is_empty())
    }
    fn current(&self) -> Value {
        let active = self.phase != Phase::Idle;
        let status = match self.phase {
            Phase::Idle if !self.error.is_empty() => self.error.clone(),
            Phase::Idle => "Сейчас ничего не играет".into(),
            Phase::Loading => "Загружаю YouTube".into(),
            Phase::Paused => format!("На паузе «{}»", self.track.title),
            Phase::Playing => format!("Играет «{}»", self.track.title),
        };
        json!({
            "playing":self.phase==Phase::Playing, "active":active, "paused":self.phase==Phase::Paused,
            "proposal":self.proposals(), "personal_search":self.personal, "cur":self.track,
            "pos":(self.pos*10.0).round()/10.0, "status":status
        })
    }
    fn snapshot(&self, radio: bool) -> Value {
        let active = self.phase != Phase::Idle;
        let next: Vec<_> = self
            .queue
            .iter()
            .chain(self.related.iter().filter(|_| radio))
            .take(20)
            .collect();
        let mut history: Vec<_> = self.history.values().collect();
        history.sort_by_key(|entry| std::cmp::Reverse(&entry.updated));
        let history: Vec<_> = history.iter().take(30).map(|entry| &entry.track).collect();
        json!({
            "source":"youtube", "playing":self.phase==Phase::Playing, "paused":self.phase==Phase::Paused,
            "loading":self.phase==Phase::Loading, "track":if active{self.track.clone()}else{Track::default()},
            "pos":if active{self.pos}else{0.0}, "dur":if active{self.dur()}else{0.0},
            "client":self.stream.as_ref().map_or("",|v|v.client), "next":next, "queue":self.queue.len(),
            "radio":radio,"history":history,"error":self.error
        })
    }
}

pub struct Youtube {
    host: Arc<Host>,
    account: Arc<Account>,
    remote: Remote,
    state: Mutex<State>,
    ready: OnceCell<()>,
    request: AsyncMutex<()>,
    wake: Notify,
    closed: AtomicBool,
    gain: AtomicU32,
}

impl Youtube {
    pub fn new(host: Host) -> Self {
        let host = Arc::new(host);
        Self {
            remote: Remote::new(host.clone()),
            account: Arc::new(Account::new(host.clone())),
            host,
            state: Mutex::default(),
            ready: OnceCell::default(),
            request: AsyncMutex::default(),
            wake: Notify::new(),
            closed: AtomicBool::new(false),
            gain: AtomicU32::new(0.8f32.to_bits()),
        }
    }

    async fn init(&self) -> Result<(), String> {
        self.ready
            .get_or_try_init(|| async {
                let marker = self.host.data.join(".migrated");
                if !marker.is_file() {
                    for name in [COOKIES, HISTORY] {
                        let path = self.host.data.join(name);
                        if path.exists() {
                            continue;
                        }
                        let raw =
                            (self.host.services)("legacy".into(), json!({"name":name})).await?;
                        if let Some(encoded) = raw.as_str() {
                            save(
                                &path,
                                &STANDARD
                                    .decode(encoded)
                                    .map_err(|_| "Повреждены данные YouTube")?,
                            )?;
                        }
                    }
                    save(&marker, b"rust\n")?;
                }
                let history_path = self.host.data.join(HISTORY);
                let history = if history_path.is_file() {
                    load(&history_path).unwrap_or_else(|e| {
                        log::warn!("YouTube: {e}");
                        Value::Null
                    })
                } else {
                    Value::Null
                };
                self.state.lock().unwrap().history =
                    serde_json::from_value(history).unwrap_or_default();
                Ok(())
            })
            .await
            .map(|_| ())
    }

    pub fn watch(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut last = Value::Null;
            loop {
                let Some(service) = weak.upgrade() else { break };
                if service.closed.load(Ordering::Acquire) {
                    break;
                }
                let (mut state, playing) = {
                    let st = service.state.lock().unwrap();
                    (st.current(), st.phase == Phase::Playing)
                };
                state["pos"] = state["pos"].as_f64().unwrap_or_default().floor().into();
                if state != last {
                    if state["playing"] == true && last["playing"] != true {
                        service.event("music.play", Value::Null).await;
                    }
                    service.event("music.state", state.clone()).await;
                    last = state;
                }
                if playing {
                    tokio::select! {
                        _ = service.wake.notified() => {}
                        _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                    }
                } else {
                    tokio::select! {
                        _ = service.wake.notified() => {}
                        _ = tokio::time::sleep(Duration::from_secs(600)) => {
                            service.remote.evict_idle(Duration::from_secs(600)).await;
                        }
                    }
                }
            }
        });
    }

    pub fn volume(&self, input: &Value) -> Value {
        if let Some(value) = input["v"].as_f64().filter(|v| (0.0..=1.0).contains(v)) {
            self.gain.store((value as f32).to_bits(), Ordering::Relaxed);
        }
        true.into()
    }

    pub async fn stop(&self) {
        self.closed.store(true, Ordering::Release);
        self.wake.notify_one();
        self.account.close();
        self.halt(false).await;
        self.emit_state().await;
        self.host.close();
    }

    async fn event(&self, method: &str, value: Value) {
        let _ = (self.host.services)(method.into(), value).await;
    }

    async fn emit_state(&self) {
        let state = self.state.lock().unwrap().current();
        self.event("music.state", state).await;
    }

    fn save_progress(&self, state: &mut State, complete: bool) {
        if !valid_id(&state.track.id) || state.pos <= 0.0 {
            return;
        }
        state.saved = Some(Instant::now());
        let previous = state.history.get(&state.track.id);
        if previous.is_some_and(|entry| entry.complete)
            && !complete
            && state.pos >= state.dur() - 20.0
        {
            return;
        }
        let entry = Saved {
            track: state.track.clone(),
            pos: state.pos,
            dur: state.dur(),
            updated: jiff::Timestamp::now().to_string(),
            complete,
        };
        state.history.insert(entry.track.id.clone(), entry);
        if state.history.len() > 100 {
            if let Some(id) = state
                .history
                .iter()
                .min_by_key(|(_, item)| &item.updated)
                .map(|(id, _)| id.clone())
            {
                state.history.remove(&id);
            }
        }
        match serde_json::to_vec(&state.history)
            .map_err(|e| e.to_string())
            .and_then(|data| save(&self.host.data.join(HISTORY), &data))
        {
            Ok(()) => {}
            Err(error) => log::warn!("YouTube: сохранить позицию: {error}"),
        }
    }

    async fn halt(&self, pause: bool) -> (bool, u64) {
        let (active, generation, task) = {
            let mut state = self.state.lock().unwrap();
            let active = state.phase != Phase::Idle;
            if active {
                self.save_progress(&mut state, false);
            }
            state.generation = state.generation.wrapping_add(1);
            if pause && state.stream.is_some() && active {
                state.phase = Phase::Paused;
            } else {
                state.phase = Phase::Idle;
                state.stream = None;
                state.track = Track::default();
                state.pos = 0.0;
            }
            if !pause {
                state.proposal = None;
                state.error.clear();
            }
            (active, state.generation, state.task.take())
        };
        self.wake.notify_one();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
        (active, generation)
    }

    async fn radio(&self) -> bool {
        self.host
            .config()
            .await
            .ok()
            .is_some_and(|config| config["radio_off"] != true)
    }

    async fn search(&self, query: &str, personal: bool) -> Result<Vec<Track>, String> {
        let query = query.trim();
        if query.len() > 2048 {
            return Err("Слишком длинный запрос".into());
        }
        if query.is_empty() {
            return Err("Что включить?".into());
        }
        if let Some(id) = video_id(query).or_else(|| valid_id(query).then(|| query.into())) {
            return Ok(vec![Track::from_id(id)]);
        }
        if personal {
            let headers = self.account.headers(true).await?;
            if !headers.is_empty() {
                let results = self.remote.search(query, headers).await?;
                if !results.is_empty() {
                    return Ok(results);
                }
            }
        }
        self.remote.search(query, HeaderMap::new()).await
    }

    async fn resolve(&self, track: Track) -> Result<(Track, Stream), String> {
        let anonymous = self.remote.resolve(&track.id, HeaderMap::new()).await;
        let (mut info, stream) = match anonymous {
            Ok(value) => value,
            Err(e) => {
                let auth = self.account.headers(false).await?;
                if auth.is_empty() {
                    return Err(e);
                }
                self.remote.resolve(&track.id, auth).await?
            }
        };
        if info.title.is_empty() {
            info.title = track.title;
        }
        if info.author.is_empty() {
            info.author = track.author;
        }
        if info.title.is_empty() {
            info.title = info.id.clone();
        }
        Ok((info, stream))
    }

    fn launch(
        self: &Arc<Self>,
        generation: u64,
        tracks: Vec<Track>,
        start: f64,
        stream: Option<Stream>,
    ) -> oneshot::Receiver<Result<Track, String>> {
        let (notify, result) = oneshot::channel();
        let mut state = self.state.lock().unwrap();
        if state.generation != generation || self.closed.load(Ordering::Acquire) {
            let _ = notify.send(Err("Воспроизведение отменено".into()));
            return result;
        }
        state.phase = Phase::Loading;
        state.track = tracks.first().cloned().unwrap_or_default();
        state.stream = stream.clone();
        state.pos = start;
        state.error.clear();
        state.saved = None;
        self.wake.notify_one();
        let service = self.clone();
        state.task = Some(tokio::spawn(async move {
            service.run(generation, tracks, start, stream, notify).await;
        }));
        result
    }

    async fn run(
        self: Arc<Self>,
        generation: u64,
        tracks: Vec<Track>,
        mut start: f64,
        mut stream: Option<Stream>,
        notify: oneshot::Sender<Result<Track, String>>,
    ) {
        let mut notify = Some(notify);
        self.emit_state().await;
        let mut pending: VecDeque<_> = tracks.into();
        let mut last = String::new();
        let mut retries = 0;
        let mut finished_selection = None;
        while let Some(track) = pending.pop_front() {
            let resolved = if let Some(ready) = stream.take() {
                Ok((track.clone(), ready))
            } else {
                self.resolve(track.clone()).await
            };
            let (track, source) = match resolved {
                Ok(value) => value,
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            if source.duration > 0.0 {
                start = start.min((source.duration - 0.1).max(0.0));
            }
            let client = match self.host.client().await {
                Ok(client) => client,
                Err(e) => {
                    last = e;
                    break;
                }
            };
            let radio = self.radio().await;
            {
                let mut state = self.state.lock().unwrap();
                if state.generation != generation {
                    return;
                }
                state.track = track.clone();
                state.stream = Some(source.clone());
                state.pos = start;
                state.phase = Phase::Loading;
                self.wake.notify_one();
                state.personal |= interesting("", &track);
                state.related = if radio && !state.personal {
                    pending.clone()
                } else {
                    VecDeque::new()
                };
            }
            log::debug!("YouTube: аудио {} Opus {}", source.client, source.itag);
            let needs_related = radio && pending.is_empty();
            let mut sent = false;
            // The audio task owns aplay. Dropping it on pause or stop closes the pipe and kills the child.
            let outcome = media::play(
                client,
                source,
                start,
                |pos| {
                    let first = {
                        let mut state = self.state.lock().unwrap();
                        if state.generation != generation {
                            return;
                        }
                        let first = !sent;
                        sent = true;
                        state.phase = Phase::Playing;
                        state.pos = pos;
                        if state
                            .saved
                            .is_none_or(|when| when.elapsed() >= Duration::from_secs(15))
                        {
                            self.save_progress(&mut state, false);
                        }
                        first
                    };
                    if first {
                        service.wake.notify_one();
                        if needs_related {
                            let service = self.clone();
                            let seed = track.id.clone();
                            tokio::spawn(async move {
                                let Ok(items) = service.related(&seed).await else {
                                    return;
                                };
                                let mut state = service.state.lock().unwrap();
                                if state.generation == generation
                                    && state.track.id == seed
                                    && state.phase == Phase::Playing
                                {
                                    state.related = items.into();
                                }
                            });
                        }
                        if let Some(notify) = notify.take() {
                            let _ = notify.send(Ok(track.clone()));
                        }
                    }
                },
                || f32::from_bits(self.gain.load(Ordering::Relaxed)),
            )
            .await;
            match outcome {
                Err(error) => {
                    last = error.to_string();
                    log::warn!("YouTube: воспроизведение {last}");
                    // CDNs sometimes rotate a URL during playback. Resume the same video at the last audible position.
                    if sent && retries < 3 && (last.contains("(403") || last.contains("оборвалось"))
                    {
                        retries += 1;
                        start = self.state.lock().unwrap().pos;
                        pending.push_front(track.clone());
                        continue;
                    }
                    if sent {
                        break;
                    }
                    continue;
                }
                Ok(()) => {
                    last.clear();
                    retries = 0;
                    start = 0.0;
                }
            }
            let radio = self.radio().await;
            let (next, related_end, candidates) = {
                let mut state = self.state.lock().unwrap();
                if state.generation != generation {
                    return;
                }
                if let Some(stream) = state.stream.as_ref().filter(|st| st.duration > 0.0) {
                    state.pos = stream.duration;
                }
                self.save_progress(&mut state, true);
                if let Some(next) = state.queue.pop_front() {
                    (Some(next), false, VecDeque::new())
                } else if state.personal {
                    (None, true, VecDeque::new())
                } else if radio {
                    (
                        pending.pop_front(),
                        false,
                        std::mem::take(&mut state.related),
                    )
                } else {
                    (None, false, VecDeque::new())
                }
            };
            if let Some(next) = next {
                pending.push_front(next);
                continue;
            }
            if related_end {
                finished_selection = Some(track.id.clone());
            } else if radio {
                if !candidates.is_empty() {
                    pending = candidates;
                    continue;
                }
                match self.related(&track.id).await {
                    Ok(related) if !related.is_empty() => {
                        pending = related.into();
                        continue;
                    }
                    _ => {}
                }
            }
            break;
        }
        {
            let mut state = self.state.lock().unwrap();
            if state.generation != generation {
                return;
            }
            state.phase = Phase::Idle;
            state.task.take();
            state.related.clear();
            state.error = last.clone();
        }
        self.wake.notify_one();
        if let Some(notify) = notify {
            let _ = notify.send(Err(if last.is_empty() {
                "YouTube ничего не нашёл".into()
            } else {
                last
            }));
        }
        self.emit_state().await;
        if let Some(id) = finished_selection {
            self.event("music.related_end", id.into()).await;
        }
    }

    async fn related(&self, id: &str) -> Result<Vec<Track>, String> {
        let auth = self.account.headers(true).await.unwrap_or_default();
        let tracks = self.remote.related(id, auth).await?;
        let state = self.state.lock().unwrap();
        Ok(tracks
            .into_iter()
            .filter(|t| !state.history.contains_key(&t.id))
            .take(12)
            .collect())
    }

    async fn launch_query(self: &Arc<Self>, op: &str, args: &Value) -> Result<Value, String> {
        let lock = self.request.lock().await;
        let query = text(&args["query"]);
        let (tracks, start) = match op {
            "play" => {
                let tracks = self.search(&query, false).await?;
                self.state.lock().unwrap().personal = false;
                (tracks, 0.0)
            }
            "play_id" => {
                let id = text(&args["id"]);
                let id = video_id(&id)
                    .or_else(|| valid_id(&id).then_some(id))
                    .ok_or("Неверный ID видео YouTube")?;
                self.state.lock().unwrap().personal = false;
                (vec![Track::from_id(id)], 0.0)
            }
            "next" => {
                let radio = self.radio().await;
                let (queued, seed) = {
                    let mut state = self.state.lock().unwrap();
                    let next = state.queue.pop_front().or_else(|| {
                        if radio {
                            state.related.pop_front()
                        } else {
                            None
                        }
                    });
                    (next, state.track.id.clone())
                };
                let tracks = if let Some(next) = queued {
                    vec![next]
                } else if valid_id(&seed) && radio {
                    self.related(&seed).await?
                } else {
                    return Err("В очереди ничего нет".into());
                };
                (tracks, 0.0)
            }
            "resume_saved" => {
                let query = query.to_lowercase();
                let state = self.state.lock().unwrap();
                let entry = state
                    .history
                    .values()
                    .filter(|entry| {
                        !entry.complete
                            && entry.pos > 0.0
                            && (query.is_empty()
                                || entry.track.title.to_lowercase().contains(&query)
                                || entry.track.author.to_lowercase().contains(&query))
                    })
                    .max_by_key(|entry| &entry.updated)
                    .ok_or("Сохранённая позиция не найдена")?;
                (vec![entry.track.clone()], entry.pos)
            }
            _ => return Err("Неизвестная команда YouTube".into()),
        };
        if tracks.is_empty() {
            return Err("YouTube ничего не нашёл".into());
        }
        let (_, generation) = self.halt(false).await;
        let finished = self.launch(
            generation,
            tracks.into_iter().take(12).collect(),
            start,
            None,
        );
        drop(lock);
        let track = finished.await.map_err(|_| "Воспроизведение отменено")??;
        if op == "resume_saved" {
            Ok(format!("Продолжаю «{}» с сохранённого места.", track.title).into())
        } else {
            Ok(json!(track))
        }
    }

    async fn change_position(self: &Arc<Self>, op: &str, args: &Value) -> Result<Value, String> {
        let _lock = self.request.lock().await;
        let (track, pos, stream, paused) = {
            let state = self.state.lock().unwrap();
            if state.phase == Phase::Idle {
                return Err("Сейчас ничего не играет".into());
            }
            if op == "resume" && state.phase != Phase::Paused {
                return Ok(Value::Null);
            }
            let delta = args["delta"].as_f64().unwrap_or(0.0);
            let position = if args["abs"] == true {
                delta
            } else {
                state.pos + delta
            };
            let max = if state.dur() > 0.0 {
                (state.dur() - 0.1).max(0.0)
            } else {
                f64::MAX
            };
            (
                state.track.clone(),
                position.clamp(0.0, max),
                state.stream.clone(),
                state.phase == Phase::Paused,
            )
        };
        let (_, generation) = self.halt(true).await;
        if paused && op == "seek" {
            let mut state = self.state.lock().unwrap();
            if state.generation == generation {
                state.pos = pos;
                self.save_progress(&mut state, false);
            }
        } else {
            let stream = stream.filter(|st| st.ready.elapsed() < Duration::from_secs(1800));
            drop(self.launch(generation, vec![track], pos, stream));
        }
        Ok(Value::Null)
    }

    async fn proposal(self: &Arc<Self>, op: &str, args: &Value) -> Result<Value, String> {
        let lock = self.request.lock().await;
        if op == "respond" {
            let reply = text(&args["text"]).to_lowercase();
            let words: Vec<_> = reply
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .collect();
            let clean = words.join(" ");
            let answer = {
                let mut state = self.state.lock().unwrap();
                if !state.proposals() || words.len() > 8 || words.is_empty() {
                    return Ok(json!({"ok":false}));
                }
                if ["отмена", "отбой", "забей", "не включай", "не сейчас"]
                    .iter()
                    .any(|w| clean.contains(w))
                {
                    state.proposal = None;
                    return Ok(json!({"ok":true,"answer":"Ладно, оставляю на паузе."}));
                }
                if ["нет", "дальше", "следующий", "другое", "неа"]
                    .iter()
                    .any(|w| words.contains(w))
                    && !["почему нет", "почему бы и нет"].contains(&clean.as_str())
                {
                    let (until, items) = state.proposal.as_mut().unwrap();
                    items.pop_front();
                    *until = Instant::now() + Duration::from_secs(180);
                    let answer = items
                        .front()
                        .map(describe)
                        .unwrap_or_else(|| "Варианты закончились. Назови другую тему.".into());
                    return Ok(json!({"ok":true,"answer":answer}));
                }
                if (words.contains(&"не") && !clean.contains("не против"))
                    || !words.iter().any(|w| {
                        [
                            "да",
                            "давай",
                            "ага",
                            "включай",
                            "погнали",
                            "окей",
                            "согласен",
                            "можно",
                        ]
                        .contains(w)
                    }) && !["не против", "почему нет", "почему бы и нет"]
                        .contains(&clean.as_str())
                {
                    return Ok(json!({"ok":false}));
                }
                state.proposal.take().unwrap().1.pop_front().unwrap()
            };
            let (_, generation) = self.halt(false).await;
            let started = self.launch(generation, vec![answer], 0.0, None);
            drop(lock);
            let track = started.await.map_err(|_| "Воспроизведение отменено")??;
            return Ok(json!({"ok":true,"answer":format!("Включил «{}».",track.title)}));
        }
        let generation = self.state.lock().unwrap().generation;
        let items = if op == "related" {
            let expected = text(&args["expected"]);
            let id = {
                let state = self.state.lock().unwrap();
                if !expected.is_empty() && (state.track.id != expected || state.phase != Phase::Idle) {
                    return Ok(Value::Null);
                }
                state.track.id.clone()
            };
            if !valid_id(&id) {
                return Err("Нет текущего видео YouTube".into());
            }
            self.related(&id).await?
        } else {
            let query = text(&args["query"]);
            let headers = self.account.headers(false).await?;
            let feed = self
                .remote
                .recommendations(headers)
                .await
                .unwrap_or_default();
            let mut candidates = {
                let state = self.state.lock().unwrap();
                feed.into_iter()
                    .filter(|track| {
                        !state.history.contains_key(&track.id) && interesting(&query, track)
                    })
                    .collect::<Vec<_>>()
            };
            if candidates.is_empty() {
                candidates = self.search(&query, true).await?;
            }
            candidates
        };
        let Some(first) = items.first() else {
            return Err("Подходящих видео YouTube нет".into());
        };
        let answer = describe(first);
        if self.state.lock().unwrap().generation != generation {
            return Ok(Value::Null);
        }
        let (_, generation) = self.halt(true).await;
        let mut state = self.state.lock().unwrap();
        if state.generation != generation {
            return Err("Подбор отменён".into());
        }
        state.personal = true;
        state.proposal = Some((Instant::now() + Duration::from_secs(180), items.into()));
        Ok(answer.into())
    }

    pub async fn music(self: &Arc<Self>, input: Value) -> Result<Value, String> {
        self.init().await?;
        let op = input["op"].as_str().unwrap_or_default();
        let result = self.command(op, &input["args"]).await;
        let state = self.state.lock().unwrap().current();
        match result {
            Ok(value) => Ok(json!({"value":value,"state":state})),
            Err(error) => {
                Ok(json!({"error":error,"login":remote::login_required(&error),"state":state}))
            }
        }
    }

    async fn command(self: &Arc<Self>, op: &str, args: &Value) -> Result<Value, String> {
        match op {
            "state" => Ok(Value::Null),
            "snapshot" => {
                let radio = self.radio().await;
                Ok(self.state.lock().unwrap().snapshot(radio))
            }
            "play" | "play_id" | "next" | "resume_saved" => self.launch_query(op, args).await,
            "enqueue" => {
                let _lock = self.request.lock().await;
                let track = self
                    .search(&text(&args["query"]), false)
                    .await?
                    .into_iter()
                    .next()
                    .ok_or("YouTube ничего не нашёл")?;
                let track = if track.title.is_empty() {
                    self.resolve(track).await?.0
                } else {
                    track
                };
                let mut state = self.state.lock().unwrap();
                if state.queue.len() >= 100 {
                    return Err("Очередь заполнена".into());
                }
                state.queue.push_back(track.clone());
                Ok(json!(track))
            }
            "unqueue" => {
                if let Some(index) = args["index"].as_u64().and_then(|v| usize::try_from(v).ok()) {
                    self.state.lock().unwrap().queue.remove(index);
                }
                Ok(Value::Null)
            }
            "stop" | "pause" => Ok(self.halt(op == "pause").await.0.into()),
            "resume" | "seek" => self.change_position(op, args).await,
            "propose" | "related" | "respond" => self.proposal(op, args).await,
            "radio_reset" => {
                let mut state = self.state.lock().unwrap();
                state.queue.clear();
                state.related.clear();
                state.personal = false;
                Ok(Value::Null)
            }
            "warm" => {
                self.remote.warm().await?;
                Ok(Value::Null)
            }
            "cookies_set" => {
                self.account
                    .set_cookies(&self.remote, args["cookies"].as_str().unwrap_or_default())
                    .await?;
                Ok(Value::Null)
            }
            "cookies_clear" => {
                self.account.clear_cookies()?;
                Ok(Value::Null)
            }
            "account" => {
                Ok(json!({"set":self.account.has_cookies(),"account":self.account.state().await?}))
            }
            "account_begin" => self.account.begin().await,
            "account_logout" => {
                self.account.logout().await?;
                Ok(Value::Null)
            }
            _ => Err("Неизвестная команда плеера".into()),
        }
    }
}

fn interesting(query: &str, track: &Track) -> bool {
    let title = format!("{} {}", track.title, track.author).to_lowercase();
    let query = query.to_lowercase();
    for (want, terms) in [
        (
            &["стендап", "standup"][..],
            &["стендап", "standup", "stand up"][..],
        ),
        (&["апвоут", "upvote"][..], &["апвоут", "upvote"][..]),
        (
            &["музык", "песн", "трек", "рок"][..],
            &[
                "музык",
                "music",
                "альбом",
                "playlist",
                "концерт",
                "official audio",
            ][..],
        ),
    ] {
        if want.iter().any(|word| query.contains(word)) {
            return terms.iter().any(|term| title.contains(term));
        }
    }
    [
        "подкаст",
        "podcast",
        "интервью",
        "лекция",
        "история",
        "аудиокниг",
        "стендап",
        "разбор",
        "апвоут",
    ]
    .iter()
    .any(|word| title.contains(word))
}

fn describe(track: &Track) -> String {
    format!("Нашёл на YouTube «{}». Послушаем?", track.title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "live YouTube probe"]
    async fn public_audio() {
        let host = Arc::new(Host::new(
            Arc::new(|_, _| Box::pin(async { Ok(json!({})) })),
            std::env::temp_dir(),
        ));
        let remote = Remote::new(host.clone());
        remote.warm().await.unwrap();
        let (_, source) = remote
            .resolve("jNQXAC9IVRw", HeaderMap::new())
            .await
            .unwrap();
        let mut bytes = 0;
        assert_eq!(source.client, "TVHTML5");
        eprintln!("Source {} itag {}", source.client, source.itag);
        let mut audio = media::Audio::open(host.client().await.unwrap(), source, 0.0, 1.0)
            .await
            .unwrap();
        while bytes < 96000 {
            let Some((_, pcm)) = audio.next(1.0).await.unwrap() else {
                break;
            };
            bytes += pcm.len();
        }
        assert!(bytes >= 96000);
    }

    #[tokio::test]
    async fn isolated_player_extracts_signature_and_rate_functions() {
        let fixture = r#"(function(){
            function makeUrl(url, key, sig) {
                let params = new Map([[key, sig]]);
                params.set("alr", "yes");
                return new Wrapped(params);
            }
            function Wrapped(params) { this.params = params; }
            Wrapped.prototype.set = function(k,v) { this.params.set(k,v); };
            Wrapped.prototype.get = function(k) { return this.params.get(k); };
            Wrapped.prototype.transform = function() {
                for (const k of ["s","n"]) {
                    const v=this.get(k);
                    if (v) this.set(k,v.split("").reverse().join(""));
                }
            };
            var config={signatureTimestamp:12345};
        }).call(this);"#;
        let prepared = Arc::new(player_script::Prepared::new(fixture.to_owned()).unwrap());
        assert_eq!(prepared.timestamp, 12345);
        assert_eq!(
            prepared.decode("abc".into(), "xyz".into()).await.unwrap(),
            ("cba".into(), "zyx".into())
        );
    }
}
