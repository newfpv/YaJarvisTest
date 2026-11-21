//! Слово активации «Джарвис» прямо на колонке. Модели — файлы в папке настроек:
//! openWakeWord (oww.bin + wake_oww.bin), если не выключена файлом disable-oww, иначе wake.bin (KWS).
//! Нет модели — Джарвиса можно позвать кнопкой «Поговорить» в панели.

use crate::settings::Store;
use jarvis_dsp::{Wake, kws::Kws, oww::Oww};
use std::path::Path;

struct NoWake;

impl Wake for NoWake {
    fn feed(&mut self, _: &[i16]) -> bool {
        false
    }
    fn reset(&mut self) {}
    fn last_prob(&self) -> f32 {
        0.0
    }
}

pub struct Loaded {
    pub det: Box<dyn Wake>,
    pub ready: bool,
    pub phrase: String,
}

fn load(data: &Path, store: &Store, training: bool) -> Loaded {
    use crate::wake_profiles::{self, DEFAULT_PHRASE};
    if training {
        let selected = store.with(|s| s.wake_profile.clone());
        if !selected.is_empty() {
            let custom = wake_profiles::get(data, &selected).and_then(|(phrase, bytes)| {
                Ok(Loaded { det: wake_profiles::model(data, &bytes, store)?, ready: true, phrase })
            });
            match custom {
                Ok(loaded) => return loaded,
                Err(error) => log::warn!("Слово активации: {error}. Загружаю штатную модель"),
            }
        }
        if let Ok(bin) = std::fs::read(data.join(wake_profiles::TUNED_MODEL)) {
            match wake_profiles::model(data, &bin, store) {
                Ok(det) => return Loaded { det, ready: true, phrase: DEFAULT_PHRASE.into() },
                Err(error) => log::warn!("Дообученное слово активации: {error}"),
            }
        }
    }
    let sens = |store: &Store| {
        let s = store.clone();
        Box::new(move || s.wake_sensitivity()) as Box<dyn Fn() -> i64 + Send>
    };
    if !data.join("disable-oww").exists() {
        if let (Ok(feat), Ok(head)) = (std::fs::read(data.join("oww.bin")), std::fs::read(data.join("wake_oww.bin"))) {
            match Oww::load(&feat, &head, sens(store)) {
                Ok(m) => {
                    log::info!("слово активации: модель openWakeWord загружена ({} КБ)", (feat.len() + head.len()) / 1024);
                    return Loaded { det: Box::new(m), ready: true, phrase: DEFAULT_PHRASE.into() };
                }
                Err(e) => log::info!("слово активации: openWakeWord: {e} — беру прежнюю модель"),
            }
        }
    }
    let path = data.join("wake.bin");
    let Ok(bin) = std::fs::read(&path) else {
        log::info!("слово активации: модели {} нет — только кнопка в настройках", path.display());
        return Loaded { det: Box::new(NoWake), ready: false, phrase: DEFAULT_PHRASE.into() };
    };
    match Kws::load(&bin, sens(store)) {
        Ok(m) => {
            log::info!("слово активации: модель загружена ({} КБ)", bin.len() / 1024);
            Loaded { det: Box::new(m), ready: true, phrase: DEFAULT_PHRASE.into() }
        }
        Err(e) => {
            log::info!("слово активации: {e}");
            Loaded { det: Box::new(NoWake), ready: false, phrase: DEFAULT_PHRASE.into() }
        }
    }
}

/// Проверка на колонке без микрофона: прогнать WAV (16 кГц моно) через модель и замерить скорость.
pub fn check_file(data: &Path, wav: &Path, store: &Store) -> Result<(), String> {
    let raw = std::fs::read(wav).map_err(|e| e.to_string())?;
    let pcm: Vec<i16> = if raw.len() > 44 && &raw[..4] == b"RIFF" {
        raw[44..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
    } else {
        raw.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
    };
    let mut w = load(data, store, true);
    let t0 = std::time::Instant::now();
    let mut hits = vec![];
    let mut peak = 0f32;
    for (i, chunk) in pcm.chunks(160).enumerate() {
        if w.det.feed(chunk) {
            hits.push(format!("{:.2} с ({:.2})", i as f64 * 0.01, w.det.wake_prob()));
        }
        peak = peak.max(w.det.last_prob());
    }
    let el = t0.elapsed();
    let secs = pcm.len() as f64 / 16000.0;
    println!(
        "{:.1} с звука за {:?} ({:.1}% реального времени), пик {:.3}, срабатываний {}: {}",
        secs,
        el,
        100.0 * el.as_secs_f64() / secs,
        peak,
        hits.len(),
        hits.join(", ")
    );
    Ok(())
}

/// Состояние слова для панели; сам детектор крутится в цикле движка разговора.
pub struct Listener {
    pub ready: std::sync::atomic::AtomicBool,
    pub last_prob: std::sync::atomic::AtomicU32,
    reload:std::sync::atomic::AtomicBool,
    training: std::sync::atomic::AtomicBool,
    default_phrase: std::sync::atomic::AtomicBool,
    phrase: std::sync::RwLock<String>,
}

impl Listener {
    pub fn new(data: &Path, store: &Store, training: bool) -> (std::sync::Arc<Listener>, Box<dyn Wake>) {
        let w = load(data, store, training);
        let default_phrase = w.phrase == crate::wake_profiles::DEFAULT_PHRASE;
        let me = std::sync::Arc::new(Listener {
            ready: w.ready.into(),
            last_prob: 0.into(),
            reload: false.into(),
            training: training.into(),
            default_phrase: default_phrase.into(),
            phrase: w.phrase.into(),
        });
        (me, w.det)
    }
    pub fn is_default_phrase(&self) -> bool { self.default_phrase.load(std::sync::atomic::Ordering::Acquire) }
    pub fn phrase(&self) -> String { self.phrase.read().unwrap().clone() }
    pub fn request_reload(&self){self.reload.store(true,std::sync::atomic::Ordering::Release);}
    pub fn take_reload(&self,data:&Path,store:&Store,training:bool)->Option<Box<dyn Wake>>{
        let changed = self.training.swap(training, std::sync::atomic::Ordering::Relaxed) != training;
        if !self.reload.swap(false,std::sync::atomic::Ordering::AcqRel) && !changed {return None;}
        let loaded=load(data,store,training);
        let default_phrase = loaded.phrase == crate::wake_profiles::DEFAULT_PHRASE;
        *self.phrase.write().unwrap() = loaded.phrase;
        self.default_phrase.store(default_phrase, std::sync::atomic::Ordering::Release);
        self.ready.store(loaded.ready,std::sync::atomic::Ordering::Release);
        self.last_prob.store(0,std::sync::atomic::Ordering::Relaxed);
        Some(loaded.det)
    }
}
