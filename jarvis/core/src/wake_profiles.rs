//! Named wake models stored separately from the built-in detector.
use crate::{
    fastcmd::norm_text,
    settings::{Store, write_atomic},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
};

pub const DEFAULT_PHRASE: &str = "Джарвис";
pub const TUNED_MODEL: &str = "plugins/training/data/wake.bin.tuned";
pub const TUNED_PREV: &str = "plugins/training/data/wake.bin.tuned.prev";
pub const MAX_MODEL: usize = 4 << 20;
const MAGIC: &[u8; 4] = b"JWP1";

#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub phrase: String,
}

fn dir(data: &Path) -> PathBuf {
    data.join("plugins/training/data/wake-profiles")
}

pub fn phrase(input: &str) -> Result<String, String> {
    let value = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.chars().count() > 48
        || !value.chars().any(char::is_alphabetic)
        || !value
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '\'' | '’'))
    {
        return Err("Введите слово или короткую фразу до 48 символов".into());
    }
    Ok(value)
}

fn model_id(phrase: &str, bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(norm_text(phrase));
    digest.update([0]);
    digest.update(bytes);
    hex::encode(&digest.finalize()[..16])
}

fn path(data: &Path, id: &str) -> Result<PathBuf, String> {
    if id.len() != 32 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Неверный ID фразы".into());
    }
    Ok(dir(data).join(format!("{id}.bin")))
}

fn header(file: &mut fs::File) -> Result<String, String> {
    let err = |_| "Повреждён профиль активации".to_string();
    let mut meta = [0u8; 6];
    file.read_exact(&mut meta).map_err(err)?;
    let length = u16::from_le_bytes([meta[4], meta[5]]) as usize;
    if &meta[..4] != MAGIC || !(1..=192).contains(&length) {
        return Err("Повреждён профиль активации".into());
    }
    let mut name = vec![0; length];
    file.read_exact(&mut name).map_err(err)?;
    let name = String::from_utf8(name).map_err(|_| "Неверное имя фразы")?;
    if phrase(&name)? != name {
        return Err("Неверное имя фразы".into());
    }
    Ok(name)
}

pub fn get(data: &Path, id: &str) -> Result<(String, Vec<u8>), String> {
    let mut file = fs::File::open(path(data, id)?).map_err(|_| "Модель этой фразы не найдена")?;
    let name = header(&mut file)?;
    let mut model = Vec::new();
    file.take((MAX_MODEL + 1) as u64)
        .read_to_end(&mut model)
        .map_err(|e| e.to_string())?;
    if model.is_empty() || model.len() > MAX_MODEL {
        return Err("Модель фразы неправильного размера".into());
    }
    Ok((name, model))
}

pub fn list(data: &Path) -> Vec<Profile> {
    let mut items = Vec::new();
    if let Ok(entries) = fs::read_dir(dir(data)) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !entry.file_type().is_ok_and(|t| t.is_file())
                || path.extension().and_then(|e| e.to_str()) != Some("bin")
            {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|name| name.to_str()) else {
                continue;
            };
            if let Ok(name) = fs::File::open(&path)
                .map_err(|e| e.to_string())
                .and_then(|mut f| header(&mut f))
            {
                items.push(Profile {
                    id: id.into(),
                    phrase: name,
                });
            }
        }
    }
    items.sort_by(|a, b| a.phrase.cmp(&b.phrase).then_with(|| a.id.cmp(&b.id)));
    items.insert(
        0,
        Profile {
            id: String::new(),
            phrase: DEFAULT_PHRASE.into(),
        },
    );
    items
}

pub fn model(
    data: &Path,
    bytes: &[u8],
    store: &Store,
) -> Result<Box<dyn jarvis_dsp::Wake>, String> {
    let store = store.clone();
    let sensitivity = Box::new(move || store.wake_sensitivity());
    if bytes.starts_with(b"OWWH") {
        let features = features(data)?;
        return jarvis_dsp::oww::Oww::load(&features, bytes, sensitivity)
            .map(|d| Box::new(d) as Box<dyn jarvis_dsp::Wake>);
    }
    jarvis_dsp::kws::Kws::load(bytes, sensitivity).map(|d| Box::new(d) as Box<dyn jarvis_dsp::Wake>)
}

pub fn features(data: &Path) -> Result<Vec<u8>, String> {
    let features = fs::File::open(data.join("oww.bin"))
        .map_err(|_| "На колонке нет слоя признаков openWakeWord")?;
    let mut bytes = Vec::new();
    features
        .take((MAX_MODEL + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_MODEL {
        return Err("Файл признаков openWakeWord слишком большой".into());
    }
    Ok(bytes)
}

pub fn save(data: &Path, name: &str, bytes: &[u8], store: &Store) -> Result<Profile, String> {
    let name = phrase(name)?;
    if bytes.is_empty() || bytes.len() > MAX_MODEL {
        return Err("Модель фразы неправильного размера".into());
    }
    model(data, bytes, store)?;
    let id = model_id(&name, bytes);
    fs::create_dir_all(dir(data)).map_err(|e| e.to_string())?;
    let mut profile = Vec::with_capacity(6 + name.len() + bytes.len());
    profile.extend_from_slice(MAGIC);
    profile.extend_from_slice(&(name.len() as u16).to_le_bytes());
    profile.extend_from_slice(name.as_bytes());
    profile.extend_from_slice(bytes);
    write_atomic(&path(data, &id)?, &profile, 0o600).map_err(|e| e.to_string())?;
    Ok(Profile { id, phrase: name })
}

pub fn select(data: &Path, id: &str, store: &Store) -> Result<(), String> {
    if !id.is_empty() {
        let (_, bytes) = get(data, id)?;
        model(data, &bytes, store)?;
    }
    store
        .update(|s| -> std::io::Result<()> {
            s.wake_profile = id.into();
            Ok(())
        })
        .map_err(|e| e.to_string())
}

pub fn remove(data: &Path, id: &str, store: &Store) -> Result<(), String> {
    if store.with(|s| s.wake_profile == id) {
        return Err("Сначала выберите другое слово активации".into());
    }
    match fs::remove_file(path(data, id)?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Err("Модель этой фразы не найдена".into()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_model_can_be_loaded_and_rolled_back_without_touching_builtin() {
        let root = std::env::temp_dir().join(format!(
            "jarvis-wake-profile-{}",
            crate::settings::rand_hex(8)
        ));
        fs::create_dir_all(&root).unwrap();
        let store = Store::open(root.join("settings.json")).unwrap();
        let builtin = include_bytes!("../../../models/wake.bin");
        fs::write(root.join("wake.bin"), builtin).unwrap();

        let first = save(&root, " Привет   Орион ", builtin, &store).unwrap();
        assert!(save(&root, "Привет Орион", b"bad model", &store).is_err());
        assert!(select(&root, "../wake", &store).is_err());
        let (listener, _) = crate::wake::Listener::new(&root, &store, false);
        assert_eq!(listener.phrase(), DEFAULT_PHRASE);
        select(&root, &first.id, &store).unwrap();
        let (after_restart, _) = crate::wake::Listener::new(&root, &store, false);
        assert_eq!(after_restart.phrase(), DEFAULT_PHRASE);
        assert!(listener.take_reload(&root, &store, true).is_some());
        assert_eq!(listener.phrase(), "Привет Орион");
        assert!(listener.take_reload(&root, &store, false).is_some());
        assert_eq!(listener.phrase(), DEFAULT_PHRASE);
        assert!(listener.take_reload(&root, &store, true).is_some());
        assert_eq!(listener.phrase(), "Привет Орион");
        assert!(remove(&root, &first.id, &store).is_err());
        select(&root, "", &store).unwrap();
        listener.request_reload();
        assert!(listener.take_reload(&root, &store, true).is_some());
        assert_eq!(listener.phrase(), DEFAULT_PHRASE);
        remove(&root, &first.id, &store).unwrap();
        assert_eq!(list(&root).len(), 1);
        assert_eq!(fs::read(root.join("wake.bin")).unwrap(), builtin);
        fs::remove_dir_all(root).unwrap();
    }
}
