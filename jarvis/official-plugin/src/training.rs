use crate::{Result, training_records::Records};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::PathBuf,
    sync::{Arc, Mutex},
};

const RATE: usize = 16_000;

pub struct Training {
    core: PathBuf,
    records: Arc<Records>,
    ring: Mutex<Vec<i16>>,
}

impl Training {
    pub fn new(init: &Value) -> std::result::Result<Self, String> {
        let core = init["core_data"]
            .as_str()
            .map(PathBuf::from)
            .ok_or("Нет каталога моделей")?;
        let data = std::env::current_dir().map_err(|error| error.to_string())?;
        Ok(Self {
            core,
            records: Arc::new(Records::new(data)),
            ring: Mutex::new(Vec::new()),
        })
    }

    pub async fn request(&self, method: &str, params: Value) -> Result {
        match method {
            "training.observe" => {
                let bytes = B64
                    .decode(params["pcm"].as_str().unwrap_or_default())
                    .map_err(|error| error.to_string())?;
                let pcm: Vec<i16> = bytes
                    .chunks_exact(2)
                    .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
                    .collect();
                let rt = tokio::runtime::Handle::current();
                let collect = params["collect"] == true;
                let hit = params["hit"].as_bool().unwrap_or(false);
                let discard = params["discard"].as_bool().unwrap_or(true);
                self.records.background.frame(&rt, &pcm, discard || hit);
                let mut ring = self.ring.lock().unwrap();
                if !collect {
                    if !ring.is_empty() {
                        ring.clear();
                        ring.shrink_to_fit();
                    }
                    return Ok(Value::Null);
                }
                if ring.len() + pcm.len() > RATE * 6 {
                    let skip = ring.len().saturating_sub(RATE * 3);
                    ring.drain(..skip);
                }
                ring.extend_from_slice(&pcm);
                if !discard {
                    let prob = params["prob"].as_f64().unwrap_or(0.) as f32;
                    let loud = pcm.iter().any(|sample| !(-600..=600).contains(sample));
                    if let Some(kind) = self.records.sample_kind(hit, prob, loud) {
                        let clip = &ring[ring.len().saturating_sub(RATE * 3)..];
                        if clip.len() >= RATE {
                            let limit = params["limit"].as_i64().unwrap_or(1);
                            self.records
                                .save_soon(&rt, kind, clip.to_vec(), prob, limit);
                        }
                    }
                }
                Ok(Value::Null)
            }
            "training.background.status" => Ok(self.records.background.status()),
            "training.background.start" => {
                let minutes = params["minutes"]
                    .as_u64()
                    .and_then(|x| u32::try_from(x).ok())
                    .ok_or("Неверная длительность")?;
                self.records.background.start(minutes)?;
                Ok(self.records.background.status())
            }
            "training.background.stop" => {
                self.records.background.stop();
                Ok(self.records.background.status())
            }
            "training.background.connect" => {
                let login = serde_json::from_value(params).map_err(|error| error.to_string())?;
                self.records.background.connect(login).await?;
                Ok(self.records.background.status())
            }
            "training.background.disconnect" => {
                self.records.background.disconnect().await?;
                Ok(self.records.background.status())
            }
            method => {
                let method = method.to_owned();
                let core = self.core.clone();
                let records = self.records.clone();
                tokio::task::spawn_blocking(move || Self::files(&method, params, records, core))
                    .await
                    .map_err(|error| error.to_string())?
            }
        }
    }

    fn files(method: &str, params: Value, records: Arc<Records>, core: PathBuf) -> Result {
        let field = |name| {
            params[name]
                .as_str()
                .ok_or_else(|| format!("Нет поля {name}"))
        };
        let spool = || field("file").map(PathBuf::from);
        match method {
            "training.records" => {
                let previous = core
                    .join("plugins/training/data/wake.bin.tuned.prev")
                    .is_file();
                Ok(records.snapshot(
                    params["mode"] == true,
                    params["limit"].as_i64().unwrap_or(1),
                    previous,
                ))
            }
            "training.compact" => records.compact(),
            "training.clear" => records.clear().map(|_| Value::Null),
            "training.move" => records
                .move_clip(field("kind")?, field("name")?, field("to")?)
                .map(|_| Value::Null),
            "training.clip" => {
                let clip = records.read(field("kind")?, field("name")?)?;
                fs::write(spool()?, clip).map_err(|error| error.to_string())?;
                Ok(Value::Null)
            }
            "training.archive" => {
                let archive =
                    records.archive(&core, params["sensitivity"].as_i64().unwrap_or(0))?;
                fs::write(spool()?, archive).map_err(|error| error.to_string())?;
                Ok(Value::Null)
            }
            "training.train" => {
                let features = fs::File::open(core.join("oww.bin"))
                    .map_err(|_| "На колонке нет слоя признаков openWakeWord")?;
                let mut bytes = Vec::new();
                features
                    .take(4 << 20)
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())?;
                let input = fs::read(spool()?).map_err(|error| error.to_string())?;
                let model = jarvis_wake_train::train(&bytes, &input)?;
                Ok(json!(B64.encode(model)))
            }
            _ => Err(format!("Плагин не поддерживает метод {method}")),
        }
    }

    pub fn stop(&self) {
        self.records.stop();
    }
}
