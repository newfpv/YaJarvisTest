//! Подсветка через /sys/class/leds: светодиоды с цветом в имени («micro:red:ring1») группируются по цвету.
//! Перед своим режимом запоминаем, что горело у Алисы, и потом возвращаем как было.
//!
//! Кольцо живое: «слушаю» — белый свет загорается сразу и дышит; «думаю» — бирюзовый огонёк бежит
//! по кругу; «говорю» — фиолетовый свет вспыхивает в такт голосу.

use crate::settings::LedSettings;
use std::{
    collections::{BTreeMap, HashMap},
    f64::consts::PI,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Mode {
    #[default]
    Off,
    Listening,
    Thinking,
    Speaking,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Off => "",
            Mode::Listening => "listening",
            Mode::Thinking => "thinking",
            Mode::Speaking => "speaking",
        }
    }
}

struct Chan {
    path: PathBuf,
    color: String, // red | green | blue | white; у часов — имя сегмента
    color_idx: usize,
    max: i64,
    group: usize,
    last: i64,
    trig: String, // штатный триггер ядра (Лайт 2: led_ring_fb), пока кольцом управляет Джарвис — «none»
}

type Rgbw = [f64; 4];

#[derive(Clone, Copy)]
struct Palette {
    listening: Rgbw,
    thinking: Rgbw,
    speaking: Rgbw,
}

impl Palette {
    fn get(&self, m: Mode) -> Rgbw {
        match m {
            Mode::Listening => self.listening,
            Mode::Thinking => self.thinking,
            Mode::Speaking => self.speaking,
            Mode::Off => [0.0; 4],
        }
    }

    fn set(&mut self, m: Mode, c: Rgbw) {
        match m {
            Mode::Listening => self.listening = c,
            Mode::Thinking => self.thinking = c,
            Mode::Speaking => self.speaking = c,
            Mode::Off => {}
        }
    }
}

fn color_idx(name: &str) -> usize {
    match name {
        "red" => 0,
        "green" => 1,
        "blue" => 2,
        _ => 3,
    }
}

struct St {
    palette: Palette,
    off: bool,
    stopped: bool,
    chans: Vec<Chan>,
    clock: Vec<Chan>, // сегменты LED-часов (Мини 3): у них своя анимация
    saved: HashMap<PathBuf, i64>,
    mode: Mode,
    until: Option<Instant>,
    groups: usize,
    since: Instant,
    level: f64,
    target: f64,
    hear: f64,
    hear_t: f64,
    frames: u64,
    lvl_at: Instant,
    phase: Mode,
}

struct Inner {
    st: Mutex<St>,
    wake: Condvar,
    source: Box<dyn Fn() -> LedSettings + Send + Sync>,
}

pub struct Leds {
    inner: Arc<Inner>,
}

impl Drop for Leds {
    fn drop(&mut self) {
        self.inner.st.lock().unwrap().stopped = true;
        self.inner.wake.notify_all();
    }
}

fn default_palette() -> Palette {
    Palette {
        listening: [0.43, 0.43, 0.43, 0.43],
        thinking: [0.0, 0.43, 0.6, 0.2],
        speaking: [0.2, 0.1, 0.55, 0.3],
    }
}

fn parse_hex(s: &str) -> (f64, f64, f64) {
    let s = s.trim().trim_start_matches('#');
    match u32::from_str_radix(s, 16) {
        Ok(v) if s.len() == 6 => ((v >> 16 & 255) as f64 / 255.0, (v >> 8 & 255) as f64 / 255.0, (v & 255) as f64 / 255.0),
        _ => (1.0, 1.0, 1.0),
    }
}

fn color_rgbw(hex: &str, bright: i64) -> Rgbw {
    let k = bright.clamp(0, 100) as f64 / 100.0;
    let (r, g, b) = parse_hex(hex);
    [r * k, g * k, b * k, r.min(g).min(b) * k]
}

pub fn color(hex: &str, bright: i64) -> HashMap<&'static str, f64> {
    let [r, g, b, w] = color_rgbw(hex, bright);
    HashMap::from([("red", r), ("green", g), ("blue", b), ("white", w)])
}

fn palette_of(s: &LedSettings) -> Palette {
    Palette {
        listening: color_rgbw(&s.listening, s.brightness),
        thinking: color_rgbw(&s.thinking, s.brightness),
        speaking: color_rgbw(&s.speaking, s.brightness),
    }
}

fn read_int(p: &Path) -> i64 {
    std::fs::read_to_string(p).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(-1)
}

fn write(p: &Path, v: i64) {
    let mut buf = [0u8; 24];
    let mut cur = std::io::Cursor::new(&mut buf[..]);
    if write!(cur, "{v}").is_ok() {
        let len = cur.position() as usize;
        let _ = std::fs::write(p, &buf[..len]);
    }
}

fn current_trigger(raw: &str) -> &str {
    raw.split_whitespace().find(|f| f.starts_with('[') && f.ends_with(']')).map(|f| f.trim_matches(['[', ']'])).unwrap_or("")
}

impl St {
    fn available(&self) -> bool {
        !self.chans.is_empty() || !self.clock.is_empty()
    }

    fn take_triggers(&mut self) {
        for c in &mut self.chans {
            let tp = c.path.with_file_name("trigger");
            if let Ok(b) = std::fs::read_to_string(&tp) {
                let t = current_trigger(&b);
                if !t.is_empty() && t != "none" {
                    c.trig = t.into();
                    let _ = std::fs::write(&tp, "none");
                }
            }
        }
    }

    fn restore_triggers(&mut self) {
        for c in &mut self.chans {
            if !c.trig.is_empty() {
                let _ = std::fs::write(c.path.with_file_name("trigger"), &c.trig);
                c.trig.clear();
            }
        }
    }

    fn shape_at(mode: Mode, groups: usize, lvl_elapsed: Duration, level: f64, group: usize, t: f64) -> f64 {
        match mode {
            Mode::Listening => 0.75 + 0.25 * (2.0 * PI * t / 2.8).cos(),
            Mode::Thinking => {
                let n = groups.max(1) as f64;
                let c = (2.0 * PI * (t * 1.2 - group as f64 / n)).cos();
                0.15 + 0.85 * c.max(0.0).powi(2)
            }
            Mode::Speaking => {
                if lvl_elapsed > Duration::from_millis(400) {
                    0.2 + 0.8 * (2.0 * PI * t * 1.7).sin().max(0.0).powi(3)
                } else {
                    0.3 + 0.7 * level
                }
            }
            Mode::Off => 0.0,
        }
    }

    fn clock_shape_at(mode: Mode, level: f64, hear: f64, seg: &str, t: f64) -> f64 {
        let b = seg.as_bytes();
        if b.len() != 2 || b[0] != b'g' || !(b'0'..=b'3').contains(&b[1]) {
            return 0.0;
        }
        let pos = (b[1] - b'0') as f64;
        let inner = pos == 1.0 || pos == 2.0;
        match mode {
            Mode::Thinking => {
                let c = (2.0 * PI * (t * 1.2 - pos / 4.0)).cos();
                0.1 + 0.9 * c.max(0.0).powi(2)
            }
            Mode::Listening | Mode::Speaking => {
                let v = if mode == Mode::Speaking { level } else { hear };
                if inner {
                    let onset = if mode == Mode::Listening { 1.0 } else { (t / 0.2).min(1.0) };
                    onset * (0.2 + 0.8 * (v / 0.35).min(1.0))
                } else {
                    ((v - 0.3) / 0.35).clamp(0.0, 1.0)
                }
            }
            Mode::Off => 0.0,
        }
    }

    fn frame(&mut self) {
        if self.mode == Mode::Off {
            return;
        }
        self.level = self.level * 0.6 + self.target * 0.4;
        self.target *= 0.5;
        self.hear = self.hear * 0.6 + self.hear_t * 0.4;
        self.hear_t *= 0.5;
        let t = self.since.elapsed().as_secs_f64();
        let pal = self.palette.get(self.mode);
        let (mode, level, hear, groups, lvl_elapsed) =
            (self.mode, self.level, self.hear, self.groups, self.lvl_at.elapsed());
        if !self.clock.is_empty() {
            self.frames += 1;
            let k = (pal.iter().fold(0f64, |a, &v| a.max(v)) * 1.6).min(1.0);
            let rewrite = self.frames % 12 == 0; // maind мог дорисовать время
            for c in self.clock.iter_mut() {
                if rewrite {
                    c.last = -1;
                }
                let s = Self::clock_shape_at(mode, level, hear, &c.color, t);
                let v = (k * s * c.max as f64) as i64;
                if v != c.last {
                    write(&c.path, v);
                    c.last = v;
                }
            }
        }
        for c in self.chans.iter_mut() {
            let s = Self::shape_at(mode, groups, lvl_elapsed, level, c.group, t);
            let v = (pal[c.color_idx] * s * c.max as f64) as i64;
            if v != c.last {
                write(&c.path, v);
                c.last = v;
            }
        }
    }
}

impl Leds {
    pub fn new(source: Box<dyn Fn() -> LedSettings + Send + Sync>) -> Arc<Leds> {
        let mut chans = vec![];
        let mut clock = vec![];
        let mut groups: BTreeMap<String, usize> = BTreeMap::new();
        let mut entries: Vec<PathBuf> = std::fs::read_dir("/sys/class/leds").map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        entries.sort();
        for e in entries {
            let name = e.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            let Some(color) = ["red", "green", "blue", "white"].iter().rev().find(|c| name.contains(*c)) else { continue };
            if name.contains("mute") {
                continue;
            }
            let mut max = read_int(&e.join("max_brightness"));
            if max <= 0 {
                max = 255;
            }
            if let Some(i) = name.rfind("clock") {
                clock.push(Chan {
                    path: e.join("brightness"),
                    color: name[i + 5..].into(),
                    color_idx: 3,
                    max,
                    group: 0,
                    last: -1,
                    trig: String::new(),
                });
                continue;
            }
            let key = name.replacen(color, "", 1);
            let n = groups.len();
            let group = *groups.entry(key).or_insert(n);
            chans.push(Chan {
                path: e.join("brightness"),
                color: color.to_string(),
                color_idx: color_idx(color),
                max,
                group,
                last: -1,
                trig: String::new(),
            });
        }
        let now = Instant::now();
        let inner = Arc::new(Inner {
            st: Mutex::new(St {
                palette: default_palette(),
                off: false,
                stopped: false,
                chans,
                clock,
                saved: HashMap::new(),
                mode: Mode::Off,
                until: None,
                groups: groups.len(),
                since: now,
                level: 0.0,
                target: 0.0,
                hear: 0.0,
                hear_t: 0.0,
                frames: 0,
                lvl_at: now,
                phase: Mode::Off,
            }),
            wake: Condvar::new(),
            source,
        });
        let l = Arc::new(Leds { inner: inner.clone() });
        if inner.st.lock().unwrap().available() {
            std::thread::Builder::new()
                .name("leds".into())
                .stack_size(128 * 1024)
                .spawn(move || {
                    let mut st = inner.st.lock().unwrap();
                    loop {
                        if st.stopped {
                            break;
                        }
                        if st.mode == Mode::Off {
                            st = inner.wake.wait(st).unwrap();
                            continue;
                        }
                        st.frame();
                        if st.until.is_some_and(|u| Instant::now() > u) {
                            drop(st);
                            inner.set_inner(Mode::Off, false);
                            st = inner.st.lock().unwrap();
                            continue;
                        }
                        let (guard, _) = inner.wake.wait_timeout(st, Duration::from_millis(40)).unwrap();
                        st = guard;
                    }
                })
                .ok();
        }
        l
    }

    pub fn available(&self) -> bool {
        self.inner.st.lock().unwrap().available()
    }

    /// Громкость голоса в микрофоне (0..1), пока Джарвис слушает — линия на часах Мини 3.
    pub fn hear(&self, v: f64) {
        let mut st = self.inner.st.lock().unwrap();
        if !st.clock.is_empty() {
            st.hear_t = st.hear_t.max(v.min(1.0));
        }
    }

    /// Громкость голоса Джарвиса прямо сейчас (0..1): кольцо вспыхивает в такт.
    pub fn level(&self, v: f64) {
        let mut st = self.inner.st.lock().unwrap();
        st.target = st.target.max(v.min(1.0));
        st.lvl_at = Instant::now();
    }

    /// Цвета и яркость из настроек.
    pub fn configure(&self, s: &LedSettings) {
        {
            let mut st = self.inner.st.lock().unwrap();
            st.palette = palette_of(s);
            st.off = !s.enabled;
        }
        if !s.enabled {
            self.set(Mode::Off);
        }
    }

    /// Показать цвет на dur (примерка цвета в настройках). Работает и при выключенной подсветке.
    pub fn show(self: &Arc<Self>, mode: Mode, hex: &str, bright: i64, dur: Duration) {
        let same = {
            let mut st = self.inner.st.lock().unwrap();
            st.palette.set(mode, color_rgbw(hex, bright));
            let same = st.mode == mode;
            if same {
                st.frame();
            }
            same
        };
        if !same {
            self.inner.set_inner(mode, true);
        }
        {
            let mut st = self.inner.st.lock().unwrap();
            if st.mode == mode {
                st.until = Some(Instant::now() + dur);
                self.inner.wake.notify_one();
            }
        }
    }

    /// Что делает Джарвис (даже если подсветки нет) — для панели.
    pub fn phase(&self) -> Mode {
        self.inner.st.lock().unwrap().phase
    }

    pub fn set(&self, mode: Mode) {
        self.inner.set_inner(mode, false)
    }
}

impl Inner {
    fn set_inner(&self, mode: Mode, force: bool) {
        let mut st = self.st.lock().unwrap();
        if !force {
            st.phase = mode;
        }
        if st.mode == Mode::Off && mode != Mode::Off && !force {
            let s = (self.source)();
            st.palette = palette_of(&s);
            st.off = !s.enabled;
        }
        if mode == st.mode || !st.available() || (st.off && mode != Mode::Off && !force) {
            return;
        }
        if st.mode == Mode::Off {
            let paths: Vec<PathBuf> = st.chans.iter().chain(st.clock.iter()).map(|c| c.path.clone()).collect();
            for p in paths {
                let v = read_int(&p);
                st.saved.insert(p, v);
            }
            st.hear = 0.0;
            st.hear_t = 0.0;
            st.take_triggers();
        }
        st.mode = mode;
        st.since = Instant::now();
        if mode == Mode::Off {
            let St { chans, clock, saved, .. } = &mut *st;
            for c in chans.iter_mut() {
                write(&c.path, saved.get(&c.path).copied().unwrap_or(0).max(0));
                c.last = -1;
            }
            for c in clock.iter_mut() {
                if let Some(&v) = saved.get(&c.path)
                    && v >= 0
                {
                    write(&c.path, v);
                }
                c.last = -1;
            }
            st.restore_triggers();
        } else {
            st.frame();
        }
        st.until = match mode {
            Mode::Listening => Some(Instant::now() + Duration::from_secs(20)),
            Mode::Thinking | Mode::Speaking => Some(Instant::now() + Duration::from_secs(600)),
            Mode::Off => None,
        };
        if mode != Mode::Off {
            self.wake.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn colors() {
        let c = super::color("#ff8000", 50);
        assert!((c["red"] - 0.5).abs() < 1e-9 && (c["green"] - 0.5 * 128.0 / 255.0).abs() < 1e-9 && c["blue"] == 0.0);
        assert_eq!(super::current_trigger("none [led_ring_fb] timer"), "led_ring_fb");
    }
}
