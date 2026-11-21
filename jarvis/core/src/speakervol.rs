//! Громкость самой колонки (её слышно у Алисы, музыки и Джарвиса). Ставим через Алису — «громче»/«тише»
//! беззвучными запросами, чтобы её шкала и кольцо остались в согласии; читаем из журнала Алисы и ALSA (Master).
//! Плюс ночной режим: пока ночь, громкость колонки не выше заданной.

use crate::{alice, led::Leds, settings::Store};
use std::{
    collections::HashMap,
    io::{Read, Seek, SeekFrom},
    process::Command,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

#[derive(Default)]
struct Vol {
    set: Option<i64>, // что попросили последним — пока шагаем к нему, панель показывает его
    set_at: Option<Instant>,
    busy: bool, // фоновый шагатель идёт к set
    // Уровень, который насчитали сами: журнал Алисы пишет уровень не на каждую смену (Мини 3),
    // поэтому помним, какой уровень при каком проценте Master.
    last_stamp: String,
    by_pct: HashMap<i64, f64>,
    // последний прочитанный процент Master (amixer — отдельный процесс, не чаще раза в 3 с)
    pct: Option<f64>,
    pct_at: Option<Instant>,
    pct_busy: bool,
    log_meta: [(u64, u64); 4],
    log_cached: (Option<f64>, String),
}

static VOL: LazyLock<Mutex<Vol>> = LazyLock::new(Mutex::default);

fn recent(t: Option<Instant>, d: Duration) -> bool {
    t.is_some_and(|t| t.elapsed() < d)
}

/// Громкость колонки 0..10 (None — неизвестна).
pub fn speaker_volume() -> Option<i64> {
    {
        let v = VOL.lock().unwrap();
        if let Some(set) = v.set
            && (v.busy || recent(v.set_at, Duration::from_secs(3)))
        {
            return Some(set);
        }
    }
    current_volume().map(|v| v.round() as i64)
}

/// Уровень Алисы 0..10 с половинками: свежая строка журнала, иначе то, что знаем про текущий процент Master,
/// иначе последняя строка журнала, иначе грубо по Master.
fn current_volume() -> Option<f64> {
    let (v, stamp) = alice_volume_log();
    let pct = master_percent();
    let mut st = VOL.lock().unwrap();
    if let Some(v) = v
        && stamp != st.last_stamp
    {
        st.last_stamp = stamp;
        if let Some(p) = pct {
            st.by_pct.insert(p, v);
        }
        return Some(v);
    }
    if let Some(known) = pct.and_then(|p| st.by_pct.get(&p)) {
        return Some(*known);
    }
    if let Some(v) = v {
        if let Some(p) = pct
            && st.by_pct.is_empty()
        {
            st.by_pct.insert(p, v); // первое чтение: считаем журнал верным для текущего Master
        }
        return Some(v);
    }
    pct.map(|p| (p as f64 / 10.0).round())
}

/// Журналы Алисы: «glagol … Changed volume to 0.7». Маленькие (ротация на ~110 КБ) — смотрим и предыдущий файл.
const VOL_LOGS: [&str; 4] = [
    "/data/quasar/daemons_logs/glagold.log",
    "/data/quasar/daemons_logs/maind.log",
    "/data/quasar/daemons_logs/glagold.log.1",
    "/data/quasar/daemons_logs/maind.log.1",
];
const MARKER: &str = "Changed volume to ";

/// Самая свежая по времени строка с уровнем из всех журналов: уровень и её метка.
fn alice_volume_log() -> (Option<f64>, String) {
    let mut meta = [(0u64, 0u64); 4];
    for (i, p) in VOL_LOGS.iter().enumerate() {
        if let Ok(m) = std::fs::metadata(p) {
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs().saturating_mul(1_000_000_000).saturating_add(d.subsec_nanos() as u64))
                .unwrap_or(0);
            meta[i] = (m.len(), mtime);
        }
    }
    if meta == [(0, 0); 4] {
        return (None, String::new());
    }
    {
        let st = VOL.lock().unwrap();
        if st.log_meta == meta {
            return st.log_cached.clone();
        }
    }
    let mut best: (Option<f64>, String) = (None, String::new());
    for p in VOL_LOGS {
        if let (Some(v), stamp) = last_volume_in(p)
            && stamp >= best.1
        {
            best = (Some(v), stamp);
        }
    }
    let mut st = VOL.lock().unwrap();
    st.log_meta = meta;
    st.log_cached = best.clone();
    best
}

fn last_volume_in(path: &str) -> (Option<f64>, String) {
    let Ok(mut f) = std::fs::File::open(path) else { return (None, String::new()) };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    let off = size.saturating_sub(64 << 10);
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(off)).is_err() || f.read_to_end(&mut buf).is_err() {
        return (None, String::new());
    }
    parse_volume_line(&String::from_utf8_lossy(&buf))
}

/// Последняя строка «Changed volume to X» (ищем с конца, без регулярки): уровень 0..10 с половинками и время строки.
pub fn parse_volume_line(b: &str) -> (Option<f64>, String) {
    let Some(i) = b.rfind(MARKER) else { return (None, String::new()) };
    let num = &b[i + MARKER.len()..];
    let num = &num[..num.find(['\r', '\n', ' ']).unwrap_or(num.len())];
    let Ok(v) = num.parse::<f64>() else { return (None, String::new()) };
    if !(0.0..=1.001).contains(&v) {
        return (None, String::new());
    }
    let line = &b[b[..i].rfind('\n').map_or(0, |j| j + 1)..i];
    // метка вида [2026-11-17 03:14:57.123]
    let stamp = line
        .split('[')
        .filter_map(|s| s.split(']').next())
        .find(|s| s.len() >= 19 && s.as_bytes()[4] == b'-' && s.as_bytes()[10] == b' ' && s[..4].bytes().all(|c| c.is_ascii_digit()))
        .unwrap_or("")
        .to_string();
    (Some((v * 20.0).round() / 2.0), stamp) // 0.55 → 5.5
}

/// Регулятор Master в процентах.
fn master_percent() -> Option<i64> {
    // -c 0 — сразу карта колонки: без него amixer разбирает весь конфиг ALSA и во время музыки думает 2+ с
    let out = Command::new("amixer").args(["-c", "0", "sget", "Master"]).output().ok().filter(|o| o.status.success());
    let out = out.or_else(|| Command::new("amixer").args(["sget", "Master"]).output().ok())?;
    let s = String::from_utf8_lossy(&out.stdout);
    let i = s.find('[')?;
    let rest = &s[i + 1..];
    let j = rest.find("%]")?;
    rest[..j].parse().ok()
}

/// Поставить уровень Алисы lvl (0..10). Сразу возвращается: шагает фоновый поток, а частые нажатия
/// в панели просто меняют цель. Шаги — «громче»/«тише»: их Алиса выполняет на колонке сама, без облака
/// и без ответа голосом. Шаг засчитываем по свежей строке журнала или по сдвигу Master.
pub fn set_alice_volume(lvl: i64) -> Result<(), String> {
    std::net::TcpStream::connect_timeout(&([127, 0, 0, 1], 9875).into(), Duration::from_secs(1)).map_err(|e| format!("Алиса недоступна: {e}"))?;
    let start = {
        let mut v = VOL.lock().unwrap();
        v.set = Some(lvl.clamp(0, 10));
        v.set_at = Some(Instant::now());
        let start = !v.busy;
        v.busy = true;
        start
    };
    if start {
        std::thread::spawn(worker);
    }
    Ok(())
}

fn finish() -> i64 {
    let mut v = VOL.lock().unwrap();
    v.busy = false;
    v.set_at = Some(Instant::now());
    v.set.unwrap_or(0)
}

fn worker() {
    let Some(mut cur) = current_volume() else {
        let lvl = finish();
        let _ = alice::text(&format!("громкость {lvl}"), true);
        return;
    };
    for _ in 0..30 {
        let lvl = VOL.lock().unwrap().set.unwrap_or(0);
        if cur.round() as i64 == lvl {
            finish();
            return;
        }
        let (dir, word) = if cur > lvl as f64 { (-1.0, "тише") } else { (1.0, "громче") };
        let (_, stamp0) = alice_volume_log();
        let pct0 = master_percent();
        if let Err(e) = alice::text(word, true) {
            log::info!("громкость: {e}");
            break;
        }
        let mut changed = false;
        // до 2 с: журнал — каждые 50 мс, Master — раз в 250 мс, если журнал молчит
        for j in 0..40 {
            std::thread::sleep(Duration::from_millis(50));
            if let (Some(v), stamp) = alice_volume_log()
                && stamp != stamp0
            {
                cur = v;
                changed = true;
            } else if j % 5 == 4
                && let (Some(p), Some(p0)) = (master_percent(), pct0)
                && p != p0
            {
                cur = (cur.round() + dir).clamp(0.0, 10.0);
                changed = true;
            }
            if changed {
                break;
            }
        }
        if !changed {
            let lvl = finish();
            log::info!("громкость: шаг «{word}» не сработал — прошу у Алисы «громкость {lvl}»");
            let _ = alice::text(&format!("громкость {lvl}"), true);
            return;
        }
        if let Some(p) = master_percent() {
            let (_, stamp) = alice_volume_log();
            let mut v = VOL.lock().unwrap();
            v.by_pct.insert(p, cur);
            if !stamp.is_empty() {
                v.last_stamp = stamp;
            }
        }
    }
    finish();
}

// Голос Джарвиса идёт через тот же регулятор Master, что и Алиса, но сам ещё усилен, чтобы его было слышно
// на тихой колонке. Выше уровня 3 каждый уровень колонки снимает 2 дБ усиления: на 10 — примерно как Алиса.
const TRIM_FROM: f64 = 3.0;
const TRIM_STEP: f64 = 2.0;

/// Множитель громкости голоса при громкости колонки level (0..10).
pub fn voice_trim(level: f64) -> f64 {
    let cut = (level - TRIM_FROM).max(0.0) * TRIM_STEP;
    10f64.powf(-cut / 20.0)
}

/// Громкость колонки 0..10 для поправки голоса: читается в фоне не чаще раза в 3 с — звук не ждёт amixer.
pub fn speaker_level() -> f64 {
    let mut v = VOL.lock().unwrap();
    if let Some(set) = v.set
        && recent(v.set_at, Duration::from_secs(4))
    {
        return set as f64;
    }
    if !recent(v.pct_at, Duration::from_secs(3)) && !v.pct_busy {
        v.pct_busy = true;
        std::thread::spawn(|| {
            let p = master_percent();
            let mut v = VOL.lock().unwrap();
            if let Some(p) = p {
                v.pct = Some(p as f64 / 10.0);
            }
            v.pct_at = Some(Instant::now());
            v.pct_busy = false;
        });
    }
    v.pct.unwrap_or(TRIM_FROM) // ещё не знаем — без поправки
}

/// Ночью громкость колонки (Алиса, музыка) не выше заданной.
pub fn night_cap(store: &Store) {
    let (night, cap) = store.with(|s| (s.is_night(), s.night.speaker));
    if !night {
        return;
    }
    let Some(v) = speaker_volume() else { return };
    if v <= cap {
        return;
    }
    if let Err(e) = set_alice_volume(cap) {
        log::info!("ночной режим: громкость: {e}");
        return;
    }
    log::info!("ночной режим: громкость колонки {v} → {cap}");
}

/// Следить за ночью раз в минуту; на переходе день↔ночь перенастроить кольцо.
pub fn watch_night(store: Store, leds: std::sync::Arc<Leds>) {
    tokio::spawn(async move {
        let mut was = false;
        loop {
            let night = store.is_night();
            if night != was {
                was = night;
                leds.configure(&store.led_for());
                if night {
                    let (from, to) = store.with(|s| (s.night.from.clone(), s.night.to.clone()));
                    log::info!("ночной режим: начался ({from}–{to})");
                }
            }
            if night {
                let s = store.clone();
                let _ = tokio::task::spawn_blocking(move || night_cap(&s)).await;
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_line() {
        let log = "x\n[2026-11-17 03:14:57.123] glagol: Changed volume to 0.55 by user\n[2026-11-17 03:15:00] other\n";
        let (v, stamp) = parse_volume_line(log);
        assert_eq!(v, Some(5.5));
        assert_eq!(stamp, "2026-11-17 03:14:57.123");
        assert_eq!(parse_volume_line("Changed volume to 1.7\n").0, None);
        assert!((voice_trim(3.0) - 1.0).abs() < 1e-9 && voice_trim(10.0) < 0.2);
    }
}
