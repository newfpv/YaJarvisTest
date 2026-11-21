//! Общая память Джарвиса: один разговор на все звонки и всех ассистентов, на годы.
//!   - Архив: каждая реплика с датой дописывается в chatgpt-chat-archive.jsonl (последние ~5000).
//!   - Досье «о хозяине»: имя, близкие, вкусы, привычки — копится всё время и переживает «Начать новый чат».
//!   - Конспект: договорённости и хронология недавних разговоров. Когда реплик вне конспекта набирается много,
//!     в паузе разговора текстовая модель дописывает старые реплики в досье и конспект.
//!   - Летопись: перед тем как старые реплики уходят из архива, модель сжимает их в запись месяца.
//!   - В начале звонка модель получает досье, конспект и последние реплики дословно, со временем.
//!   - recall_conversation ищет по архиву и летописи, edit_memory забывает и исправляет.
//!
//! «Начать новый чат» забывает конспект и недавние реплики (досье, летопись и архив остаются), «Стереть память» — всё.
//! Файлы те же, что у Go-версии: chatgpt-chat.json и chatgpt-chat-archive.jsonl.

use crate::plugins::BoxFut;
use jiff::{Timestamp, Zoned, civil::Date};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    io::{BufRead as _, Write as _},
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

/// В начале звонка: досье, конспект и последние реплики — не больше стольких букв.
pub const KEEP_CHARS: usize = 10000;
const COMPACT_AT: usize = 9000;
const COMPACT_KEEP: usize = 4500;
const COMPACT_MAX: usize = 24000;
const PROFILE_MAX: usize = 2000;
const SUMMARY_MAX: usize = 2500;
const CHRONICLE_MAX: usize = 1500;
const COMPACT_IDLE: Duration = Duration::from_secs(45);
const RECALL_MAX: usize = 4000;
const ARCHIVE_HARD: usize = 8000;
const ARCHIVE_MAX: usize = 5000;
const RECENT_CAP: usize = 256;
const FORGET_QUIET: i64 = 600;
const TEXT_MAX: usize = 20000;

pub const NO_SUMMARIZER: &str = "конспект не делается: память выключена или нет ни подписки ChatGPT, ни ключей Gemini и OpenAI";
pub const BUSY: &str = "память уже обновляется — подождите минуту";

fn is_zero(n: &i64) -> bool {
    *n == 0
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ChatTurn {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub n: i64,
    #[serde(default)]
    pub ts: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub user: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bot: String,
}

impl ChatTurn {
    fn size(&self) -> usize {
        runes(&self.user) + runes(&self.bot)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChronMonth {
    pub month: String,
    pub text: String,
    pub turns: i64,
}

/// Что лежит в chatgpt-chat.json.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    id: String,
    since: i64,
    #[serde(skip_serializing_if = "is_zero")]
    from: i64,
    #[serde(skip_serializing_if = "is_zero")]
    next: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    profile: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    summary: String,
    #[serde(skip_serializing_if = "is_zero")]
    summary_upto: i64,
    #[serde(skip_serializing_if = "is_zero")]
    summary_at: i64,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "crate::settings::nd")]
    chronicle: Vec<ChronMonth>,
    /// формат до 0.9.67: при загрузке переезжают в архив
    #[serde(skip_serializing, deserialize_with = "crate::settings::nd")]
    turns: Vec<ChatTurn>,
}

/// Текстовая модель без инструментов: (instructions, text) → ответ.
pub type Summarizer = Arc<dyn Fn(String, String) -> BoxFut<Result<String, String>> + Send + Sync>;

#[derive(Default)]
struct St {
    d: Saved,
    recent: Vec<ChatTurn>,
    archive_len: usize,
    archive_first_ts: Option<i64>,
    chat_turns: usize,
    compacting: bool,
    running: bool,
    again: bool,
    folding: bool,
    timer: u64,
    fail_until: i64,
    profile_rev: u64,
    epoch: u64,
    edits: Vec<(String, String)>,
    forgets: Vec<(Vec<String>, i64)>,
}

pub struct Memory {
    path: PathBuf,
    st: Mutex<St>,
    enabled: Box<dyn Fn() -> bool + Send + Sync>,
    clock: Box<dyn Fn() -> Zoned + Send + Sync>,
    summarize: Mutex<Option<Summarizer>>,
    rt: Option<tokio::runtime::Handle>,
}

fn runes(s: &str) -> usize {
    s.chars().count()
}

pub fn clip(s: &str, n: usize) -> String {
    if runes(s) > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s.to_string() }
}

fn or<'a>(s: &'a str, def: &'a str) -> &'a str {
    if s.trim().is_empty() { def } else { s }
}

/// Последние реплики, всего не больше chars букв (но хотя бы одна).
fn tail(turns: &[ChatTurn], chars: usize) -> &[ChatTurn] {
    let (mut n, mut i) = (0, turns.len());
    while i > 0 {
        n += turns[i - 1].size();
        if n > chars && i < turns.len() {
            break;
        }
        i -= 1;
    }
    &turns[i..]
}

fn size(turns: &[ChatTurn]) -> usize {
    turns.iter().map(ChatTurn::size).sum()
}

const MONTHS_GEN: [&str; 12] = ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"];
const MONTHS: [&str; 12] = ["январь", "февраль", "март", "апрель", "май", "июнь", "июль", "август", "сентябрь", "октябрь", "ноябрь", "декабрь"];
const WEEKDAYS: [&str; 7] = ["воскресенье", "понедельник", "вторник", "среда", "четверг", "пятница", "суббота"];

/// «2026-11» → «ноябрь 2026».
pub fn month_label(m: &str) -> String {
    let mut it = m.splitn(2, '-');
    match (it.next().and_then(|y| y.parse::<i32>().ok()), it.next().and_then(|x| x.parse::<usize>().ok())) {
        (Some(y), Some(mo)) if (1..=12).contains(&mo) && m.len() == 7 => format!("{} {y}", MONTHS[mo - 1]),
        _ => m.to_string(),
    }
}

/// «вчера, пятница 14 ноября»; relative=false — без «сегодня/вчера» (для памяти, она живёт долго).
fn day_label(d: Date, now: &Zoned, relative: bool) -> String {
    let mut s = format!("{} {} {}", WEEKDAYS[d.weekday().to_sunday_zero_offset() as usize], d.day(), MONTHS_GEN[d.month() as usize - 1]);
    if d.year() != now.year() {
        s += &format!(" {}", d.year());
    }
    if relative {
        let days = d.until(now.date()).map(|x| x.get_days()).unwrap_or(-1);
        match days {
            0 => s = format!("сегодня, {s}"),
            1 => s = format!("вчера, {s}"),
            2 => s = format!("позавчера, {s}"),
            _ => {}
        }
    }
    s
}

fn at(ts: i64, now: &Zoned) -> Zoned {
    Timestamp::from_second(ts).unwrap_or(Timestamp::UNIX_EPOCH).to_zoned(now.time_zone().clone())
}

/// Реплики по дням: «— вчера, пятница 14 ноября —», «18:05 Человек: …», «18:05 Джарвис: …».
fn transcript(turns: &[ChatTurn], now: &Zoned, relative: bool) -> String {
    let mut b = String::new();
    let (mut day, mut prev) = (String::new(), 0i64);
    for t in turns {
        let tm = at(t.ts, now);
        let d = day_label(tm.date(), now, relative);
        if d != day {
            b += &format!("— {d} —\n");
            day = d;
        } else if prev != 0 && t.n > prev + 1 {
            b += "…\n";
        }
        prev = t.n;
        let hm = tm.strftime("%H:%M").to_string();
        if !t.user.is_empty() {
            b += &format!("{hm} Человек: {}\n", t.user);
        }
        if !t.bot.is_empty() {
            b += &format!("{hm} Джарвис: {}\n", t.bot);
        }
    }
    b.trim_end_matches('\n').to_string()
}

const MEMORY_RULES: &str = "Ты ведёшь долгую память голосового ассистента Джарвиса в умной колонке хозяина — на годы: через год Джарвис должен знать хозяина как близкий друг. \
«Человек» — хозяин (или кто-то в доме), «Джарвис» — ассистент; в квадратных скобках — что Джарвис сделал на колонке. \
По-русски, коротко, только факты из реплик — ничего не выдумывай. Всегда пиши, кто что сказал или сделал: «хозяин рассказал…», \
«хозяин просил…», «Джарвис посоветовал…». Даты — «14.11» (и время, если важно). Пропускай мелочи: громкость, свет, «спасибо», ошибки и повторы команд. \
Факты с пометкой «хозяин сам попросил запомнить» важнее всего и заменяют противоречащие им. Без вступлений и пояснений.\n\n";

static COMPACT_INSTRUCTIONS: LazyLock<String> = LazyLock::new(|| {
    MEMORY_RULES.to_string()
        + "Тебе дают досье на хозяина, прежний конспект и новые реплики по дням. Верни ЦЕЛИКОМ обновлённые досье и конспект — они заменят прежние — \
строго в трёх разделах, каждый заголовок на отдельной строке:\nО ХОЗЯИНЕ:\nДОГОВОРЁННОСТИ:\nХРОНОЛОГИЯ:\n\
- О ХОЗЯИНЕ — досье: имя, близкие и питомцы, работа и учёба, вкусы (музыка, еда, фильмы, игры), привычки и распорядок, \
здоровье (если сам рассказал), дом и устройства, планы и мечты, как с ним лучше говорить. Перенеси всё из прежнего досье и добавь новое. \
Досье — СПИСКОМ: каждый факт на отдельной строке, начиная с «- », одна мысль на строку; сплошным абзацем не пиши. \
Не длиннее 2000 знаков: когда места мало, объединяй и обобщай, но устойчивое не теряй.\n\
- ДОГОВОРЁННОСТИ — обещания, планы, напоминания с датами; прошедшее и выполненное убирай.\n\
- ХРОНОЛОГИЯ — по дням: о чём говорили и что решили. Свежие дни подробнее, старше недели — строка на день, старше месяца — строка на неделю. \
Не длиннее 2000 знаков."
});

static CHRONICLE_INSTRUCTIONS: LazyLock<String> = LazyLock::new(|| {
    MEMORY_RULES.to_string()
        + "Тебе дают реплики за часть месяца (и прежнюю запись летописи за этот месяц, если она есть). Эти реплики уходят из архива навсегда, \
поэтому сохрани всё ценное. Верни запись летописи за месяц ЦЕЛИКОМ: о чём говорили, что хозяин рассказал о себе и близких, \
что решили и о чём договорились, что Джарвис советовал и делал — по датам. Не длиннее 1500 знаков."
});

const EDIT_INSTRUCTIONS: &str = "Ты правишь долгую память голосового ассистента Джарвиса по просьбе хозяина. Тебе дают, что забыть и что запомнить, \
и текущую память. Убери ВСЁ, что относится к забываемому (и прямые упоминания, и выводы из него), \
впиши то, что надо запомнить, с пометкой «хозяин сам попросил запомнить», остальное оставь как было, слово в слово. \
Верни память ЦЕЛИКОМ в том же виде, без пояснений.";

/// Ответ модели на три раздела: досье (О ХОЗЯИНЕ) и конспект (остальное). ok=false — разделов нет.
pub fn sections(out: &str) -> (String, String, bool) {
    let cut = |l: &str, set: &str| l.trim().trim_matches(|c| set.contains(c)).to_string();
    let head = |l: &str| {
        let u = cut(l, "#*: ").to_uppercase();
        if u.starts_with("О ХОЗЯИНЕ") {
            "p"
        } else if u.starts_with("ДОГОВОР") || u.starts_with("ХРОНОЛОГ") {
            "s"
        } else {
            ""
        }
    };
    let (mut p, mut s) = (vec![], vec![]);
    let (mut cur, mut ok) = ("", false);
    for l in out.trim().split('\n') {
        let h = head(l);
        if !h.is_empty() && runes(l.trim()) < 40 {
            cur = h;
            ok |= h == "p";
            if h == "s" {
                s.push(cut(l, "#* "));
            }
            continue;
        }
        match cur {
            "p" => p.push(l.to_string()),
            "s" => s.push(l.to_string()),
            _ => {}
        }
    }
    if !ok {
        return (String::new(), out.trim().to_string(), false);
    }
    (p.join("\n").trim().to_string(), s.join("\n").trim().to_string(), true)
}

/// Досье списком «- факт» по строке; сплошной абзац режется по предложениям.
pub fn profile_lines(s: &str) -> String {
    let mut out = vec![];
    for l in s.split('\n') {
        let l = l.trim().trim_start_matches(['-', '•', '*']).trim();
        if l.is_empty() {
            continue;
        }
        let r: Vec<char> = l.chars().collect();
        let mut start = 0;
        let mut i = 0;
        while i + 2 < r.len() {
            if matches!(r[i], '.' | '!' | '?') && r[i + 1] == ' ' && (r[i + 2].is_uppercase() || r[i + 2] == '«') {
                out.push(format!("- {}", r[start..=i].iter().collect::<String>().trim()));
                start = i + 2;
            }
            i += 1;
        }
        out.push(format!("- {}", r[start..].iter().collect::<String>().trim()));
    }
    out.join("\n")
}

/// Убрать строки, где есть все слова.
fn drop_lines(text: &str, terms: &[String]) -> String {
    if terms.is_empty() {
        return text.to_string();
    }
    text.split('\n').filter(|l| score_text(l, terms) < terms.len()).collect::<Vec<_>>().join("\n")
}

// ---- «вспомни»

static RECALL_STOP: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    "что как про мне мной меня тебе тебя это этот эта эти этом был была было были какой какая какое какие когда где там тут \
все всё весь вся его её она они оно так уже ещё или для при над под без ним ней них кто чем чём нибудь либо тоже свой своя мой моя мои твой твоя твои \
говорил говорила говорили сказал сказала сказали советовал советовала спрашивал спрашивала рассказывал рассказывала рассказал рассказала \
вспомни вспомнить помнишь напомни разговор разговоре разговаривали обсуждали говорили беседа забудь забыть запомни запомнить \
вчера сегодня позавчера утром днём днем вечером ночью неделе недавно прошлой прошлый раньше тогда давно \
джарвис хозяин хозяина хозяину человек нас нам наш наша наши ваш день"
        .split_whitespace()
        .collect()
});

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect()
}

/// Корни слов запроса: «пиццу» и «пицца» → «пицц».
pub fn recall_terms(q: &str) -> Vec<String> {
    let mut out = vec![];
    let mut seen = HashSet::new();
    for w in words(&q.to_lowercase()) {
        let r: Vec<char> = w.replace('ё', "е").chars().collect();
        if r.len() < 3 || RECALL_STOP.contains(w.as_str()) {
            continue;
        }
        let r = if r.len() > 4 { &r[..6.min(4.max(r.len() - 2))] } else { &r[..] };
        let s: String = r.iter().collect();
        if seen.insert(s.clone()) {
            out.push(s);
        }
    }
    out
}

/// Сколько корней из terms есть в тексте (как начало слова).
fn score_text(text: &str, terms: &[String]) -> usize {
    let ws = words(&text.to_lowercase().replace('ё', "е"));
    terms.iter().filter(|t| ws.iter().any(|w| w.starts_with(t.as_str()))).count()
}

fn score(t: &ChatTurn, terms: &[String]) -> usize {
    score_text(&format!("{} {}", t.user, t.bot), terms)
}

/// День из параметра date («2026-11-15») или из слов «сегодня/вчера/позавчера» в запросе.
fn recall_day(date: &str, q: &str, now: &Zoned) -> Option<Date> {
    if let Ok(d) = date.trim().parse::<Date>() {
        return Some(d);
    }
    let q = format!("{q} {date}").to_lowercase();
    let back = |n: i64| now.date().checked_sub(jiff::Span::new().days(n)).ok();
    if q.contains("позавчера") {
        back(2)
    } else if q.contains("вчера") {
        back(1)
    } else if q.contains("сегодня") {
        Some(now.date())
    } else {
        None
    }
}

/// Ответ без модели стоит запомнить: ошибки не запоминаем, иначе модель потом повторяет их.
pub fn worthy(answer: &str) -> bool {
    let a = answer.to_lowercase();
    ![
        "недоступ", "не подключ", "ошибк", "не удалось", "не получилось", "не отвеча", "не нашл",
        // модель назвала себя не Джарвисом — в память не берём, иначе следующий разговор повторит
        "я chatgpt", "я — chatgpt", "я gpt", "я gemini", "я — gemini", "я grok", "я языковая модель", "я большая языковая модель", "я — языковая модель",
    ]
    .iter()
    .any(|b| a.contains(b))
}

/// Реплика архива для панели.
#[derive(Serialize)]
pub struct PanelTurn {
    pub n: i64,
    pub ts: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub user: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub bot: String,
    pub day: String,
}

fn memory_text(s: &str) -> Result<String, String> {
    let s = s.replace("\r\n", "\n").trim().to_string();
    if runes(&s) > TEXT_MAX {
        return Err("слишком длинно: память должна быть короткой, иначе Джарвис будет путаться".into());
    }
    Ok(s)
}

impl St {
    fn from_n(&self, n: i64) -> &[ChatTurn] {
        let i = self.recent.partition_point(|t| t.n < n);
        &self.recent[i..]
    }

    /// Реплики текущего чата, ещё не попавшие в конспект.
    fn pending(&self) -> &[ChatTurn] {
        self.from_n(self.d.from.max(self.d.summary_upto + 1))
    }

    fn push_recent(&mut self, t: ChatTurn) {
        self.recent.push(t);
        if self.recent.len() > RECENT_CAP {
            let drop = self.recent.len() - RECENT_CAP;
            self.recent.drain(..drop);
        }
    }

    fn set_month(&mut self, month: &str, text: String, turns: i64) {
        if let Some(m) = self.d.chronicle.iter_mut().find(|m| m.month == month) {
            m.text = text;
            m.turns += turns;
            return;
        }
        self.d.chronicle.push(ChronMonth { month: month.into(), text, turns });
        self.d.chronicle.sort_by(|a, b| a.month.cmp(&b.month));
    }

    fn reset(&mut self) {
        self.timer += 1;
        self.d.id = crate::chatgpt::new_id();
        self.d.since = Timestamp::now().as_second();
        self.d.from = self.d.next;
        self.chat_turns = 0;
        self.d.summary.clear();
        self.d.summary_upto = 0;
    }
}

fn stream_archive<F: FnMut(ChatTurn) -> bool>(path: &std::path::Path, mut f: F) {
    let Ok(file) = std::fs::File::open(path) else { return };
    let mut rd = std::io::BufReader::with_capacity(8192, file);
    let mut line = Vec::with_capacity(512);
    loop {
        line.clear();
        match rd.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if let Ok(t) = serde_json::from_slice::<ChatTurn>(&line)
                    && (!t.user.is_empty() || !t.bot.is_empty())
                    && !f(t)
                {
                    break;
                }
            }
        }
    }
}

impl Memory {
    pub fn load(path: PathBuf, enabled: Box<dyn Fn() -> bool + Send + Sync>, clock: Box<dyn Fn() -> Zoned + Send + Sync>) -> Arc<Memory> {
        let mut st = St { d: std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default(), ..Default::default() };
        let m = Memory { path, st: Mutex::default(), enabled, clock, summarize: Mutex::default(), rt: tokio::runtime::Handle::try_current().ok() };
        let mut dirty = false;
        st.d.next = st.d.next.max(1);
        st.d.from = st.d.from.max(1);
        if st.d.id.is_empty() {
            st.d.id = crate::chatgpt::new_id();
            st.d.since = Timestamp::now().as_second();
            dirty = true;
        }
        let mut recent_q: VecDeque<ChatTurn> = VecDeque::with_capacity(RECENT_CAP);
        let mut sorted_valid = true;
        let mut prev_n = 0i64;
        stream_archive(&m.archive_path(), |t| {
            if t.n == 0 || t.n < prev_n {
                sorted_valid = false;
            }
            prev_n = prev_n.max(t.n);
            if t.n >= st.d.next {
                st.d.next = t.n + 1;
            }
            if st.archive_first_ts.is_none() {
                st.archive_first_ts = Some(t.ts);
            }
            if t.n >= st.d.from {
                st.chat_turns += 1;
            }
            st.archive_len += 1;
            if recent_q.len() == RECENT_CAP {
                recent_q.pop_front();
            }
            recent_q.push_back(t);
            true
        });
        st.recent = recent_q.into_iter().collect();

        let mut legacy = Vec::new();
        if !st.d.turns.is_empty() && st.archive_len == 0 {
            legacy = std::mem::take(&mut st.d.turns);
            dirty = true;
        } else if !st.d.turns.is_empty() {
            st.d.turns.clear();
            dirty = true;
        }

        if !legacy.is_empty() || !sorted_valid {
            let mut all = if !legacy.is_empty() {
                legacy
            } else {
                let mut v = Vec::with_capacity(st.archive_len);
                stream_archive(&m.archive_path(), |t| {
                    v.push(t);
                    true
                });
                v.sort_by_key(|t| t.n);
                v
            };
            st.d.next = 1;
            for t in all.iter_mut() {
                if t.n == 0 {
                    t.n = st.d.next;
                }
                if t.n >= st.d.next {
                    st.d.next = t.n + 1;
                }
            }
            if all.len() > ARCHIVE_HARD {
                let from = all.len() - ARCHIVE_MAX;
                all.drain(..from);
            }
            m.write_archive_slice(&all);
            m.sync_stats(&mut st, &all);
        } else if st.archive_len > ARCHIVE_HARD {
            let drop_n = st.archive_len - ARCHIVE_MAX;
            m.filter_rewrite_archive(&mut st, drop_n, |_| true);
        }
        if dirty {
            m.save(&st);
        }
        *m.st.lock().unwrap() = st;
        Arc::new(m)
    }

    fn now(&self) -> Zoned {
        (self.clock)()
    }

    /// Галочка «Запоминать разговоры».
    pub fn enabled(&self) -> bool {
        (self.enabled)()
    }

    /// Чем сжимать старые реплики (None — не сжимать).
    pub fn set_summarizer(self: &Arc<Self>, f: Option<Summarizer>) {
        *self.summarize.lock().unwrap() = f;
        let mut st = self.st.lock().unwrap();
        self.schedule(&mut st);
    }

    fn archive_path(&self) -> PathBuf {
        let stem = self.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        self.path.with_file_name(format!("{stem}-archive.jsonl"))
    }

    fn write_archive_slice(&self, turns: &[ChatTurn]) {
        let p = self.archive_path();
        let tmp = p.with_extension("jsonl.tmp");
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        if let Ok(file) = o.open(&tmp) {
            let mut wr = std::io::BufWriter::with_capacity(8192, file);
            for t in turns {
                let _ = serde_json::to_writer(&mut wr, t);
                let _ = wr.write_all(b"\n");
            }
            if wr.flush().is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }

    fn sync_stats(&self, st: &mut St, turns: &[ChatTurn]) {
        st.archive_len = turns.len();
        st.archive_first_ts = turns.first().map(|t| t.ts);
        st.chat_turns = turns.iter().filter(|t| t.n >= st.d.from).count();
        let start = turns.len().saturating_sub(RECENT_CAP);
        st.recent = turns[start..].to_vec();
    }

    fn filter_rewrite_archive<F: FnMut(&ChatTurn) -> bool>(
        &self,
        st: &mut St,
        skip_first: usize,
        mut keep: F,
    ) -> usize {
        let p = self.archive_path();
        let tmp = p.with_extension("jsonl.tmp");
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        let Ok(file) = o.open(&tmp) else { return 0 };
        let mut wr = std::io::BufWriter::with_capacity(8192, file);
        let mut seen = 0usize;
        let mut kept = 0usize;
        let mut removed = 0usize;
        let mut first_ts = None;
        let mut chat_turns = 0usize;
        let mut recent_q = VecDeque::with_capacity(RECENT_CAP);
        stream_archive(&p, |t| {
            let drop = seen < skip_first || !keep(&t);
            seen += 1;
            if drop {
                removed += 1;
                return true;
            }
            let _ = serde_json::to_writer(&mut wr, &t);
            let _ = wr.write_all(b"\n");
            kept += 1;
            if first_ts.is_none() {
                first_ts = Some(t.ts);
            }
            if t.n >= st.d.from {
                chat_turns += 1;
            }
            if recent_q.len() == RECENT_CAP {
                recent_q.pop_front();
            }
            recent_q.push_back(t);
            true
        });
        if wr.flush().is_ok() && std::fs::rename(&tmp, &p).is_ok() {
            st.archive_len = kept;
            st.archive_first_ts = first_ts;
            st.chat_turns = chat_turns;
            st.recent = recent_q.into_iter().collect();
        } else {
            let _ = std::fs::remove_file(&tmp);
            return 0;
        }
        removed
    }

    fn append_archive(&self, st: &mut St, t: &ChatTurn) {
        let mut o = std::fs::OpenOptions::new();
        o.append(true).create(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        if let Ok(mut f) = o.open(self.archive_path()) {
            let mut line = serde_json::to_vec(t).unwrap_or_default();
            line.push(b'\n');
            let _ = f.write_all(&line);
        }
        st.archive_len += 1;
        if st.archive_first_ts.is_none() {
            st.archive_first_ts = Some(t.ts);
        }
        if t.n >= st.d.from {
            st.chat_turns += 1;
        }
        st.push_recent(t.clone());
        if st.archive_len > ARCHIVE_HARD {
            // сжимать в летопись нечем (нет модели) — старое выкидываем, раз в несколько тысяч реплик
            let drop = st.archive_len - ARCHIVE_MAX;
            self.filter_rewrite_archive(st, drop, |_| true);
            log::info!("память: архив переполнен, а сжать в летопись нечем — {drop} старых реплик удалено");
        }
    }

    fn save(&self, st: &St) {
        let Ok(b) = serde_json::to_vec(&st.d) else { return };
        let tmp = self.path.with_extension("json.tmp");
        if write_private(&tmp, &b).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }

    /// Номер беседы для ChatGPT (один на все звонки).
    pub fn thread(&self) -> String {
        self.st.lock().unwrap().d.id.clone()
    }

    /// Реплика закончилась: запомнить вопрос и ответ.
    pub fn add(self: &Arc<Self>, user: &str, bot: &str) {
        let (user, bot) = (user.trim(), bot.trim());
        if (user.is_empty() && bot.is_empty()) || !self.enabled() {
            return;
        }
        let now = self.now().timestamp().as_second();
        let mut st = self.st.lock().unwrap();
        let t = ChatTurn { n: st.d.next, ts: now, user: user.into(), bot: bot.into() };
        // «забудь, что меня зовут Сергей» — ни сама просьба, ни ответ на неё в память не идут
        st.forgets.retain(|(_, until)| now <= *until);
        if st.forgets.iter().any(|(terms, _)| score(&t, terms) == terms.len()) {
            return;
        }
        st.d.next += 1;
        self.append_archive(&mut st, &t);
        self.save(&st);
        self.schedule(&mut st);
    }

    /// Что Джарвис сделал сам (включил видео, выполнил команду): модель узнает это в следующем звонке.
    pub fn add_fact(self: &Arc<Self>, fact: &str) {
        let fact = fact.trim();
        if !fact.is_empty() {
            self.add("", &format!("[{fact}]"));
        }
    }

    /// Последние реплики текущего чата дословно, по дням и со временем.
    pub fn transcript(&self, chars: usize) -> String {
        if !self.enabled() {
            return String::new();
        }
        let now = self.now();
        let st = self.st.lock().unwrap();
        transcript(tail(st.from_n(st.d.from), chars), &now, true)
    }

    /// Память для начала звонка: досье, конспект и последние реплики дословно, всего около chars букв.
    pub fn context(&self, chars: usize) -> String {
        if !self.enabled() {
            return String::new();
        }
        let now = self.now();
        let st = self.st.lock().unwrap();
        let mut parts = vec![];
        if !st.d.profile.is_empty() {
            parts.push(format!("Что ты знаешь о хозяине (досье, копится всё время):\n{}", st.d.profile));
        }
        if !st.d.summary.is_empty() {
            parts.push(format!("Конспект недавних разговоров (когда, кто что сказал и сделал):\n{}", st.d.summary));
        }
        let budget = chars.saturating_sub(runes(&st.d.profile) + runes(&st.d.summary)).max(chars / 3);
        let recent = tail(st.pending(), budget);
        if !recent.is_empty() {
            parts.push(format!("Последние реплики дословно (время — у хозяина):\n{}", transcript(recent, &now, true)));
        }
        if let (Some(a), Some(b)) = (st.d.chronicle.first(), st.d.chronicle.last()) {
            parts.push(format!(
                "Разговоры раньше этого сжаты в летопись: {} — {} (подробности — инструментом recall_conversation).",
                month_label(&a.month),
                month_label(&b.month)
            ));
        }
        parts.join("\n\n")
    }

    /// Есть ли что вспоминать инструментом.
    pub fn has_archive(&self) -> bool {
        if !self.enabled() {
            return false;
        }
        let st = self.st.lock().unwrap();
        st.archive_len > 0 || !st.d.chronicle.is_empty()
    }

    // ---- обслуживание: правки, конспект, летопись

    fn schedule(self: &Arc<Self>, st: &mut St) {
        let has = self.summarize.lock().unwrap().is_some();
        if !has || st.compacting || self.now().timestamp().as_second() < st.fail_until || !self.enabled() {
            return;
        }
        if st.archive_len > ARCHIVE_MAX + 200 {
            st.folding = true;
        }
        if size(st.pending()) < COMPACT_AT && !(st.folding && st.archive_len > ARCHIVE_MAX) {
            st.folding = false;
            return;
        }
        // разговор ещё идёт — ждём паузу: каждая новая реплика откладывает сжатие
        st.timer += 1;
        let (g, me) = (st.timer, self.clone());
        if let Some(rt) = &self.rt {
            rt.spawn(async move {
                tokio::time::sleep(COMPACT_IDLE).await;
                if me.st.lock().unwrap().timer == g {
                    me.maintain().await;
                }
            });
        }
    }

    fn fail(&self, st: &mut St, what: &str, err: &str) {
        st.fail_until = self.now().timestamp().as_second() + 600;
        if err != NO_SUMMARIZER {
            log::info!("память: не получилось {what}: {}", clip(err, 160));
        }
    }

    /// По очереди: правки «забудь/запомни», конспект, летопись. Один проход за раз.
    pub async fn maintain(self: &Arc<Self>) {
        {
            let mut st = self.st.lock().unwrap();
            if st.running {
                st.again = true;
                return;
            }
            st.running = true;
        }
        loop {
            self.apply_edits().await;
            let _ = self.compact(COMPACT_KEEP, COMPACT_AT).await;
            let _ = self.fold().await;
            let mut st = self.st.lock().unwrap();
            if !st.again {
                st.running = false;
                return;
            }
            st.again = false;
        }
    }

    async fn ask(&self, instructions: &str, text: String) -> Result<String, String> {
        let f = self.summarize.lock().unwrap().clone().ok_or(NO_SUMMARIZER)?;
        match tokio::time::timeout(Duration::from_secs(120), f(instructions.to_string(), text)).await {
            Ok(r) => r,
            Err(_) => Err("модель не ответила за 2 минуты".into()),
        }
    }

    /// «Обновить досье сейчас»: сжать всё, кроме последней реплики, не дожидаясь длинного разговора.
    pub async fn compact_now(self: &Arc<Self>) -> Result<usize, String> {
        if !self.enabled() {
            return Err("память выключена".into());
        }
        if self.summarize.lock().unwrap().is_none() {
            return Err(NO_SUMMARIZER.into());
        }
        if self.st.lock().unwrap().compacting {
            return Err(BUSY.into());
        }
        self.compact(0, 1).await
    }

    /// Дописать старые реплики (кроме последних keep букв) в досье и конспект.
    async fn compact(self: &Arc<Self>, keep: usize, at_size: usize) -> Result<usize, String> {
        let now = self.now();
        let (old, id, epoch, rev, text) = {
            let mut st = self.st.lock().unwrap();
            if self.summarize.lock().unwrap().is_none() || st.compacting {
                return Ok(0);
            }
            let pending = st.pending();
            let mut old = &pending[..pending.len() - tail(pending, keep).len()];
            let mut n = 0;
            for (i, t) in old.iter().enumerate() {
                n += t.size();
                if n > COMPACT_MAX && i > 0 {
                    old = &old[..i];
                    break;
                }
            }
            if old.is_empty() || size(pending) < at_size {
                return Ok(0);
            }
            let old = old.to_vec();
            let text = format!(
                "Досье на хозяина (прежнее):\n{}\n\nПрежний конспект:\n{}\n\nНовые реплики (добавь их в досье и конспект):\n{}",
                or(&st.d.profile, "пока пусто"),
                or(&st.d.summary, "пока пусто"),
                transcript(&old, &now, false)
            );
            st.compacting = true;
            (old, st.d.id.clone(), st.epoch, st.profile_rev, text)
        };
        let out = self.ask(&COMPACT_INSTRUCTIONS, text).await.and_then(|o| if o.trim().is_empty() { Err("модель вернула пустой ответ".into()) } else { Ok(o) });
        let mut st = self.st.lock().unwrap();
        st.compacting = false;
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                self.fail(&mut st, "сжать разговор в конспект", &e);
                return Err(e);
            }
        };
        if st.d.id != id || st.epoch != epoch {
            return Ok(0); // пока сжимали, начали новый чат или стёрли память
        }
        let (profile, summary, ok) = sections(&out);
        if ok && st.profile_rev == rev {
            st.d.profile = clip(&profile_lines(&profile), PROFILE_MAX * 3 / 2);
        }
        st.d.summary = clip(&summary, SUMMARY_MAX * 3 / 2);
        st.d.summary_upto = old.last().map_or(0, |t| t.n);
        st.d.summary_at = now.timestamp().as_second();
        self.save(&st);
        log::info!("память: {} реплик дописано в досье и конспект (досье {} букв, конспект {})", old.len(), runes(&st.d.profile), runes(&st.d.summary));
        self.schedule(&mut st);
        Ok(old.len())
    }

    /// Архив переполнен: самые старые реплики (одного месяца) сжать в летопись и убрать из архива.
    async fn fold(self: &Arc<Self>) -> Result<(), String> {
        let now = self.now();
        let (batch, month, epoch, text) = {
            let mut st = self.st.lock().unwrap();
            if self.summarize.lock().unwrap().is_none() || st.compacting || !st.folding || st.archive_len <= ARCHIVE_MAX {
                st.folding = st.folding && st.archive_len > ARCHIVE_MAX;
                return Ok(());
            }
            let over_len = st.archive_len - ARCHIVE_MAX;
            let month_of = |t: &ChatTurn| at(t.ts, &now).strftime("%Y-%m").to_string();
            let mut month = String::new();
            let mut batch = vec![];
            let mut n = 0;
            let mut seen = 0;
            let (from_n, summary_upto) = (st.d.from, st.d.summary_upto);
            stream_archive(&self.archive_path(), |t| {
                if seen >= over_len {
                    return false;
                }
                seen += 1;
                if t.n >= from_n && t.n > summary_upto {
                    return false; // ещё не в конспекте текущего чата — пусть сначала туда
                }
                let tm = month_of(&t);
                if month.is_empty() {
                    month = tm;
                } else if tm != month {
                    return false;
                }
                n += t.size();
                if n > COMPACT_MAX && !batch.is_empty() {
                    return false;
                }
                batch.push(t);
                true
            });
            if batch.is_empty() {
                return Ok(());
            }
            let prev = st.d.chronicle.iter().find(|m| m.month == month).map(|m| m.text.clone()).unwrap_or_default();
            let text = format!(
                "Месяц: {}\n\nПрежняя запись летописи за этот месяц:\n{}\n\nРеплики (добавь их в запись):\n{}",
                month_label(&month),
                or(&prev, "пока нет"),
                transcript(&batch, &now, false)
            );
            st.compacting = true;
            (batch, month, st.epoch, text)
        };
        let out = self.ask(&CHRONICLE_INSTRUCTIONS, text).await.map(|o| o.trim().to_string());
        let out = out.and_then(|o| if o.is_empty() { Err("модель вернула пустую запись".to_string()) } else { Ok(o) });
        let mut st = self.st.lock().unwrap();
        st.compacting = false;
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                self.fail(&mut st, "сжать старые реплики в летопись", &e);
                return Err(e);
            }
        };
        if st.epoch != epoch {
            return Ok(());
        }
        st.set_month(&month, clip(&out, CHRONICLE_MAX * 3 / 2), batch.len() as i64);
        let last = batch.last().map_or(0, |t| t.n);
        self.filter_rewrite_archive(&mut st, 0, |t| t.n > last);
        self.save(&st);
        log::info!("память: {} старых реплик за {} ушли в летопись ({} букв)", batch.len(), month_label(&month), runes(&out));
        self.schedule(&mut st);
        Ok(())
    }

    // ---- «забудь», «запомни», «меня зовут не Сергей, а Павел»

    /// Инструмент edit_memory. Сразу: реплики про забываемое уходят из архива, новое — в досье.
    /// В фоне модель переписывает досье, конспект и летопись без забытого.
    pub fn edit(self: &Arc<Self>, forget: &str, remember: &str, words: &[String]) -> String {
        let (forget, remember) = (forget.trim(), remember.trim());
        if !self.enabled() {
            return "Память выключена в настройках — запоминать и забывать нечего. Так и скажи.".into();
        }
        if forget.is_empty() && remember.is_empty() {
            return "Скажи, что забыть или что запомнить.".into();
        }
        let now = self.now();
        let mut out = vec![];
        {
            let mut st = self.st.lock().unwrap();
            if !remember.is_empty() {
                st.d.profile = format!("{}\n- {remember} (хозяин сам попросил запомнить, {})", st.d.profile, now.strftime("%d.%m")).trim().to_string();
                st.profile_rev += 1;
                out.push(format!("Запомнил: «{remember}»."));
            }
            if !forget.is_empty() {
                let mut terms: Vec<String> = words.iter().flat_map(|w| recall_terms(w)).collect();
                if terms.is_empty() {
                    terms = recall_terms(forget);
                }
                let mut removed = 0;
                if !terms.is_empty() {
                    removed = self.filter_rewrite_archive(&mut st, 0, |t| score(t, &terms) != terms.len());
                    st.forgets.push((terms, now.timestamp().as_second() + FORGET_QUIET));
                }
                out.push(format!("Забываю «{forget}»: из архива удалено реплик — {removed}, досье, конспект и летопись перепишутся в течение минуты."));
                log::info!("память: хозяин попросил забыть — удалено реплик из архива: {removed}");
            }
            st.edits.push((forget.into(), remember.into()));
            self.save(&st);
        }
        if let Some(rt) = &self.rt {
            let me = self.clone();
            rt.spawn(async move { me.maintain().await });
        }
        out.join(" ") + " Коротко скажи хозяину, что сделал."
    }

    /// Переписать память по просьбам «забудь/запомни» (моделью; без неё — убрать строки со словами).
    async fn apply_edits(self: &Arc<Self>) {
        let has = self.summarize.lock().unwrap().is_some();
        let (edits, task, epoch, rev, id, mem, months) = {
            let mut st = self.st.lock().unwrap();
            if st.edits.is_empty() || st.compacting {
                return;
            }
            let edits = std::mem::take(&mut st.edits);
            let mut task = vec![];
            let mut terms = vec![];
            for (f, r) in &edits {
                if !f.is_empty() {
                    task.push(format!("Забыть: {f}"));
                    terms.push(recall_terms(f));
                }
                if !r.is_empty() {
                    task.push(format!("Запомнить: {r}"));
                }
            }
            if !has {
                for t in &terms {
                    st.d.profile = drop_lines(&st.d.profile, t);
                    st.d.summary = drop_lines(&st.d.summary, t);
                    for m in st.d.chronicle.iter_mut() {
                        m.text = drop_lines(&m.text, t);
                    }
                }
                self.save(&st);
                return;
            }
            st.compacting = true;
            let mem = format!("О ХОЗЯИНЕ:\n{}\n{}", or(&st.d.profile, "пока пусто"), or(&st.d.summary, "ДОГОВОРЁННОСТИ:\nХРОНОЛОГИЯ:"));
            let mut months = HashMap::new();
            for m in &st.d.chronicle {
                if terms.iter().any(|t| !t.is_empty() && score_text(&m.text, t) > 0) {
                    months.insert(m.month.clone(), m.text.clone());
                }
            }
            (edits, task.join("\n"), st.epoch, st.profile_rev, st.d.id.clone(), mem, months)
        };
        let out = self.ask(EDIT_INSTRUCTIONS, format!("{task}\n\nПамять:\n{mem}")).await;
        let mut new_months = HashMap::new();
        if out.is_ok() {
            for (m, text) in months {
                if let Ok(t) = self.ask(EDIT_INSTRUCTIONS, format!("{task}\n\nПамять:\n{text}")).await
                    && !t.trim().is_empty()
                {
                    new_months.insert(m, t.trim().to_string());
                }
            }
        }
        let now = self.now();
        let mut st = self.st.lock().unwrap();
        st.compacting = false;
        if st.epoch != epoch {
            return;
        }
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                let later = std::mem::take(&mut st.edits);
                st.edits = edits.into_iter().chain(later).collect(); // повторим позже
                self.fail(&mut st, "переписать память по просьбе хозяина", &e);
                return;
            }
        };
        let (profile, summary, ok) = sections(&out);
        if ok {
            if st.profile_rev == rev {
                st.d.profile = clip(&profile_lines(&profile), PROFILE_MAX * 3 / 2);
            }
            if st.d.id == id {
                st.d.summary = clip(&summary, SUMMARY_MAX * 3 / 2);
            }
        }
        for (m, text) in new_months {
            if let Some(x) = st.d.chronicle.iter_mut().find(|x| x.month == m) {
                x.text = clip(&text, CHRONICLE_MAX * 3 / 2);
            }
        }
        st.d.summary_at = now.timestamp().as_second();
        self.save(&st);
        log::info!("память: переписана по просьбе хозяина ({} правок)", edits.len());
    }

    // ---- «вспомни»

    /// Найти в архиве реплики по словам и/или за день, а в летописи — месяцы по словам или по дню.
    pub fn search(&self, query: &str, date: &str) -> String {
        if !self.enabled() {
            return "Память выключена в настройках: прошлых разговоров ты не помнишь. Так и скажи.".into();
        }
        let now = self.now();
        let st = self.st.lock().unwrap();
        if st.archive_len == 0 && st.d.chronicle.is_empty() {
            return "Архив разговоров пуст — вспоминать нечего. Не выдумывай: скажи, что не помнишь.".into();
        }
        let day = recall_day(date, query, &now);
        let in_day = |t: &ChatTurn| day.is_none_or(|d| at(t.ts, &now).date() == d);
        let since = match (st.d.chronicle.first(), st.archive_first_ts) {
            (Some(m), _) => format!("летописи за {}", month_label(&m.month)),
            (None, Some(ts)) => day_label(at(ts, &now).date(), &now, true),
            (None, None) => day_label(now.date(), &now, true),
        };
        let terms = recall_terms(query);

        // летопись: месяцы по словам; за день старше архива — его месяц
        let mut chron: Vec<&ChronMonth> = vec![];
        if !terms.is_empty() {
            let mut ms: Vec<(&ChronMonth, usize)> = st.d.chronicle.iter().map(|m| (m, score_text(&m.text, &terms))).filter(|x| x.1 > 0).collect();
            ms.sort_by(|a, b| b.1.cmp(&a.1));
            chron.extend(ms.iter().take(2).map(|x| x.0));
        }
        if let Some(d) = day {
            let day_start = d.to_zoned(now.time_zone().clone()).map(|z| z.timestamp().as_second()).unwrap_or(0);
            if st.archive_first_ts.is_none_or(|ts| day_start < ts - 86400) {
                let key = d.strftime("%Y-%m").to_string();
                for m in &st.d.chronicle {
                    if m.month == key && !chron.iter().any(|c| c.month == m.month) {
                        chron.push(m);
                    }
                }
            }
        }

        let mut hits: Vec<(usize, usize)> = vec![];
        let mut day_meta: Vec<(usize, usize)> = vec![];
        let mut prev_meta: Option<(bool, usize)> = None;
        let mut sizes: HashMap<usize, usize> = HashMap::new();
        let mut idx = 0usize;
        stream_archive(&self.archive_path(), |t| {
            let i = idx;
            idx += 1;
            let cur_ok = in_day(&t);
            let cur_sz = t.size();
            if cur_ok {
                if let Some((_, ps)) = prev_meta.filter(|(ok, _)| *ok) {
                    sizes.insert(i - 1, ps);
                }
                sizes.insert(i, cur_sz);
                day_meta.push((i, cur_sz));
                if !terms.is_empty() {
                    let s = score(&t, &terms);
                    if s > 0 {
                        hits.push((i, s));
                    }
                }
            }
            if let Some(prev_hit) = hits.last() && prev_hit.0 + 1 == i && cur_ok {
                sizes.insert(i, cur_sz);
            }
            prev_meta = Some((cur_ok, cur_sz));
            true
        });
        if chron.is_empty() {
            if let Some(d) = day
                && day_meta.is_empty()
            {
                return format!("За {} в памяти разговоров нет (память начинается с {since}). Не выдумывай.", day_label(d, &now, true));
            }
            if day.is_none() && hits.is_empty() {
                return format!(
                    "В памяти ничего не нашлось по запросу «{}» (она начинается с {since}). Не выдумывай — скажи, что не помнишь, или попробуй другие слова.",
                    query.trim()
                );
            }
        }
        let mut pick: Vec<usize> = vec![];
        if !hits.is_empty() {
            hits.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0))); // при равенстве — свежее
            for limit in (1..=hits.len().min(8)).rev() {
                let mut set = BTreeSet::new();
                for &(i, _) in &hits[..limit] {
                    // соседние реплики — чтобы был понятен контекст
                    for j in i.saturating_sub(1)..=i + 1 {
                        if sizes.contains_key(&j) {
                            set.insert(j);
                        }
                    }
                }
                pick = set.into_iter().collect();
                if pick.iter().filter_map(|j| sizes.get(j)).sum::<usize>() <= RECALL_MAX {
                    break;
                }
            }
        } else if day.is_some() {
            pick = day_meta.into_iter().map(|(i, _)| i).collect(); // весь день
        }
        let mut parts = vec![];
        if !pick.is_empty() {
            let mut keep = BTreeSet::new();
            let (mut n, mut more) = (0, 0);
            for &j in &pick {
                let sz = sizes.get(&j).copied().unwrap_or(0);
                n += sz;
                if n > RECALL_MAX && !keep.is_empty() {
                    more += 1;
                    continue;
                }
                keep.insert(j);
            }
            let mut turns = Vec::with_capacity(keep.len());
            let mut cur_i = 0usize;
            stream_archive(&self.archive_path(), |t| {
                if keep.contains(&cur_i) {
                    turns.push(t);
                }
                cur_i += 1;
                cur_i <= *keep.last().unwrap_or(&0)
            });
            let mut s = format!(
                "Из архива разговоров (время — у хозяина; «Человек» — хозяин, «Джарвис» — ты; в скобках — что ты сделал):\n{}",
                transcript(&turns, &now, true)
            );
            if more > 0 {
                s += &format!("\n(и ещё {more} реплик за этот день — уточни запрос словами, чтобы найти нужное)");
            }
            parts.push(s);
        }
        for m in chron {
            parts.push(format!("Из летописи (старые разговоры, сжато) — {}:\n{}", month_label(&m.month), m.text));
        }
        parts.join("\n\n")
    }

    // ---- кнопки панели

    /// «Начать новый чат»: конспект и недавние реплики забыть; досье, летопись и архив остаются.
    pub fn reset(&self) {
        let mut st = self.st.lock().unwrap();
        st.reset();
        self.save(&st);
    }

    /// «Стереть память»: досье, летопись и архив.
    pub fn erase(&self) {
        let mut st = self.st.lock().unwrap();
        st.reset();
        st.epoch += 1;
        st.d.profile.clear();
        st.d.chronicle.clear();
        st.d.summary_at = 0;
        st.recent.clear();
        st.archive_len = 0;
        st.archive_first_ts = None;
        st.chat_turns = 0;
        st.edits.clear();
        st.forgets.clear();
        let _ = std::fs::remove_file(self.archive_path());
        self.save(&st);
    }

    /// Любая правка руками отменяет сжатие, которое шло в этот момент (оно повторится само).
    fn edited(&self, st: &mut St) {
        st.epoch += 1;
        st.profile_rev += 1;
        self.save(st);
    }

    pub fn set_profile(&self, text: &str) -> Result<(), String> {
        let text = memory_text(text)?;
        let mut st = self.st.lock().unwrap();
        st.d.profile = text;
        self.edited(&mut st);
        Ok(())
    }

    pub fn set_summary(&self, text: &str) -> Result<(), String> {
        let text = memory_text(text)?;
        let now = self.now().timestamp().as_second();
        let mut st = self.st.lock().unwrap();
        if !text.is_empty() {
            st.d.summary_at = now;
        }
        st.d.summary = text;
        self.edited(&mut st);
        Ok(())
    }

    /// Переписать месяц летописи; пустой текст — удалить месяц.
    pub fn set_month(&self, month: &str, text: &str) -> Result<(), String> {
        let text = memory_text(text)?;
        let mut st = self.st.lock().unwrap();
        let i = st.d.chronicle.iter().position(|m| m.month == month).ok_or("такого месяца в летописи нет")?;
        if text.is_empty() {
            st.d.chronicle.remove(i);
        } else {
            st.d.chronicle[i].text = text;
        }
        self.edited(&mut st);
        Ok(())
    }

    /// Реплики архива от новых к старым: before — номер, раньше которого (0 — с самой новой); q — только с этими словами.
    pub fn archive_page(&self, q: &str, before: i64, limit: usize) -> (Vec<PanelTurn>, bool) {
        let limit = if limit == 0 || limit > 200 { 50 } else { limit };
        let terms: Vec<String> = q.to_lowercase().replace('ё', "е").split_whitespace().map(String::from).collect();
        let now = self.now();
        let st = self.st.lock().unwrap();
        let matches_turn = |t: &ChatTurn| -> bool {
            if before > 0 && t.n >= before {
                return false;
            }
            if !terms.is_empty() {
                let text = format!("{} {}", t.user, t.bot).to_lowercase().replace('ё', "е");
                if !terms.iter().all(|w| text.contains(w.as_str())) {
                    return false;
                }
            }
            true
        };
        let can_use_recent = st.archive_len == st.recent.len()
            || st.recent.first().is_some_and(|first| {
                let matched_in_recent = st.recent.iter().rev().filter(|t| matches_turn(t)).count();
                matched_in_recent > limit || (before > 0 && first.n >= before && matched_in_recent == limit)
            });
        if can_use_recent {
            let mut out = vec![];
            for t in st.recent.iter().rev() {
                if !matches_turn(t) {
                    continue;
                }
                if out.len() == limit {
                    return (out, true);
                }
                out.push(PanelTurn { n: t.n, ts: t.ts, user: t.user.clone(), bot: t.bot.clone(), day: day_label(at(t.ts, &now).date(), &now, true) });
            }
            return (out, false);
        }
        let mut ring: VecDeque<PanelTurn> = VecDeque::with_capacity(limit + 1);
        stream_archive(&self.archive_path(), |t| {
            if before > 0 && t.n >= before {
                return false;
            }
            if matches_turn(&t) {
                if ring.len() == limit + 1 {
                    ring.pop_front();
                }
                ring.push_back(PanelTurn { n: t.n, ts: t.ts, user: t.user, bot: t.bot, day: day_label(at(t.ts, &now).date(), &now, true) });
            }
            true
        });
        let more = ring.len() > limit;
        if more {
            ring.pop_front();
        }
        let mut out: Vec<PanelTurn> = ring.into_iter().collect();
        out.reverse();
        (out, more)
    }

    /// Убрать реплики из архива (и из текущего чата: он — хвост архива).
    pub fn delete_turns(&self, ns: &[i64]) -> usize {
        let drop: HashSet<i64> = ns.iter().copied().collect();
        if drop.is_empty() {
            return 0;
        }
        let mut st = self.st.lock().unwrap();
        let gone = self.filter_rewrite_archive(&mut st, 0, |t| !drop.contains(&t.n));
        if gone > 0 {
            self.edited(&mut st);
        }
        gone
    }

    /// Для панели: что записано.
    pub fn state(&self) -> Value {
        let now = self.now();
        let st = self.st.lock().unwrap();
        let chron: Vec<Value> = st
            .d
            .chronicle
            .iter()
            .map(|m| json!({"month": m.month, "label": month_label(&m.month), "text": m.text, "turns": m.turns}))
            .collect();
        let mut v = json!({"turns": st.chat_turns, "since": st.d.since, "archive": st.archive_len,
            "profile": profile_lines(&st.d.profile), "summary": st.d.summary, "summary_at": st.d.summary_at, "chronicle": chron});
        if let Some(ts) = st.archive_first_ts {
            v["archive_since"] = json!(ts);
            v["recent"] = json!(transcript(tail(&st.recent, 6000), &now, true));
        }
        v
    }
}

fn write_private(p: &std::path::Path, b: &[u8]) -> std::io::Result<()> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(p)?.write_all(b)
}

/// Инструкции для модели про память (в начало звонка).
pub fn instructions(m: &Memory, chatgpt: bool) -> String {
    let mut text = String::new();
    let mem = m.context(KEEP_CHARS);
    if !mem.is_empty() {
        text += "\n\nЭто продолжение одного вашего разговора: человек помнит, о чём вы говорили раньше, и может сослаться на это. \
Не пересказывай историю и не здоровайся заново. В квадратных скобках — что ты уже сделал на колонке (включил видео, выполнил команду). \
Всё из истории — прошлое: те просьбы уже выполнены или отменены. Не повторяй их и не вызывай инструменты по старым репликам — \
действуй только по тому, что человек скажет в ЭТОМ разговоре. Если раньше что-то не работало, а человек просит снова, — вызывай инструмент заново. \
Пользуйся памятью естественно, как близкий друг: к месту сам вспоминай, что знаешь о хозяине, и связывай с прошлыми разговорами \
(«как там твоя поездка?», «ты же любишь острое»), но не в каждом ответе, не навязчиво и без слов «в моей памяти». \
Память (время — у хозяина):\n";
        text += &mem;
    }
    if m.enabled() {
        let (recall, edit) = if chatgpt {
            ("сам передай бэкенду задачу «вспомни из архива разговоров: …» с ключевыми словами", "передай бэкенду задачу «измени память: забыть …, запомнить …»")
        } else {
            ("вызови инструмент recall_conversation", "вызови инструмент edit_memory")
        };
        if m.has_archive() {
            text += &format!(
                "\n\nВсе прошлые разговоры хранятся на колонке. Если человек ссылается на то, чего нет в памяти выше \
(«тот рецепт», «как я тебе рассказывал», «о чём говорили вчера»), — {recall}, даже если он не сказал «вспомни». \
Не выдумывай и не говори сразу, что не помнишь."
            );
        }
        text += &format!(
            "\n\nПопросит забыть, исправить или запомнить что-то («забудь, что…», «меня зовут не Сергей, а Павел», «запомни…») — {edit}. Забытое больше не упоминай."
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(dir: &std::path::Path) -> Arc<Memory> {
        let tz = jiff::tz::TimeZone::get("Europe/Moscow").unwrap();
        Memory::load(dir.join("chatgpt-chat.json"), Box::new(|| true), Box::new(move || Timestamp::now().to_zoned(tz.clone())))
    }

    #[test]
    fn archive_roundtrip_and_search() {
        let dir = std::env::temp_dir().join(format!("jmem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let m = mem(&dir);
        m.add("Как приготовить пиццу?", "Тесто, соус, сыр.");
        m.add("Включи свет", "[Готово.]");
        m.add_fact("включил на YouTube: «Лекция»");
        let id = m.thread();
        drop(m);
        let m = mem(&dir);
        assert_eq!(m.thread(), id);
        assert_eq!(m.st.lock().unwrap().archive_len, 3);
        assert!(m.search("тот рецепт пиццы", "").contains("пиццу"));
        assert!(m.search("квантовая физика", "").contains("ничего не нашлось"));
        assert!(m.search("", "сегодня").contains("Включи свет"));
        assert!(m.context(KEEP_CHARS).contains("Человек: Как приготовить пиццу?"));
        let r = m.edit("рецепт пиццы", "хозяина зовут Павел", &["пицца".into()]);
        assert!(r.contains("удалено реплик — 1"), "{r}");
        assert!(m.state()["profile"].as_str().unwrap().contains("Павел"));
        m.add("про пиццу ещё раз", "ок");
        assert_eq!(m.st.lock().unwrap().archive_len, 2, "после «забудь» реплики про это не пишутся");
        let (page, more) = m.archive_page("свет", 0, 10);
        assert_eq!((page.len(), more), (1, false));
        assert_eq!(m.delete_turns(&[page[0].n]), 1);
        m.reset();
        assert_ne!(m.thread(), id);
        m.erase();
        assert_eq!(m.state()["archive"], 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn helpers() {
        assert_eq!(recall_terms("Что ты говорил про пиццу вчера?"), vec!["пицц"]);
        assert_eq!(month_label("2026-11"), "ноябрь 2026");
        let (p, s, ok) = sections("О ХОЗЯИНЕ:\n- зовут Павел\nДОГОВОРЁННОСТИ:\n- нет\nХРОНОЛОГИЯ:\n- 14.11 пицца");
        assert!(ok && p == "- зовут Павел" && s.starts_with("ДОГОВОРЁННОСТИ:"));
        assert_eq!(profile_lines("Зовут Павел. Любит пиццу."), "- Зовут Павел.\n- Любит пиццу.");
        assert!(!worthy("Яндекс недоступен") && worthy("Готово."));
    }
}
