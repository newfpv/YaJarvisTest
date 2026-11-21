//! Shared transport/storage helpers; provider protocols stay in their optional modules.
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::{Value, json};
use std::{
    future::Future,
    net::SocketAddr,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub type Callback = Arc<
    dyn Fn(String, Value) -> Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>
        + Send
        + Sync,
>;
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
pub fn random<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| "Не получить случайные данные")?;
    Ok(bytes)
}
pub fn text(v: &Value) -> String {
    v.as_str().map(str::to_string).unwrap_or_else(|| {
        if v.is_number() {
            v.to_string()
        } else {
            String::new()
        }
    })
}
pub fn normal(s: &str) -> String {
    s.to_lowercase()
        .replace('ё', "е")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
pub struct Host {
    pub services: Callback,
    pub data: PathBuf,
    clients: Mutex<Option<(Value, reqwest::Client)>>,
    migrated: tokio::sync::OnceCell<()>,
}
impl Host {
    pub fn new(services: Callback, data: PathBuf) -> Self {
        Self {
            services,
            data,
            clients: Mutex::default(),
            migrated: Default::default(),
        }
    }
    pub async fn config(&self) -> Result<Value, String> {
        (self.services)("config.get".into(), json!({})).await
    }
    pub async fn set(&self, before: Value, after: Value, connected: bool) -> Result<(), String> {
        (self.services)(
            "config.set".into(),
            json!({"before":before,"after":after,"connected":connected}),
        )
        .await
        .map(|_| ())
    }
    pub async fn client(&self) -> Result<reqwest::Client, String> {
        let network = (self.services)("network".into(), json!({})).await?;
        let mut clients = self.clients.lock().unwrap();
        if let Some((route, client)) = clients.as_ref() {
            if route == &network {
                return Ok(client.clone());
            }
        }
        let client = http_client(&network)?;
        *clients = Some((network, client.clone()));
        Ok(client)
    }
    pub async fn session(&self) -> Result<reqwest::Client, String> {
        http_client(&(self.services)("network".into(), json!({})).await?)
    }
    pub fn close(&self) {
        self.clients.lock().unwrap().take();
    }
    pub async fn migrate(&self, name: &str) -> Result<(), String> {
        self.migrated
            .get_or_try_init(|| async {
                if self.data.join(name).exists() {
                    return Ok(());
                }
                let old = (self.services)("legacy".into(), json!({})).await?;
                if let Some(encoded) = old.as_str() {
                    let bytes = B64.decode(encoded).map_err(|_| "Неверный старый кеш")?;
                    if bytes.len() > 4 << 20
                        || !serde_json::from_slice::<Value>(&bytes)
                            .is_ok_and(|v| v.is_array() || v.is_null())
                    {
                        return Err("Неверный старый кеш".into());
                    }
                    save(&self.data.join(name), &bytes)?;
                }
                Ok(())
            })
            .await
            .map(|_| ())
    }
}
pub fn http_client(network: &Value) -> Result<reqwest::Client, String> {
    if let Some(error) = network["error"].as_str() {
        return Err(error.into());
    }
    if !network.is_object() {
        return Err("Нет маршрута подключения".into());
    }
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .cookie_store(true)
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(30));
    if let Some(socks) = network["socks"].as_str() {
        let addr: SocketAddr = socks.parse().map_err(|_| "Неверный SOCKS VPN")?;
        let user = network["user"].as_str().unwrap_or("");
        let pass = network["pass"].as_str().unwrap_or("");
        if !addr.ip().is_loopback()
            || addr.port() == 0
            || !(1..=255).contains(&user.len())
            || !(1..=255).contains(&pass.len())
        {
            return Err("Неверный SOCKS VPN".into());
        }
        builder = builder.proxy(
            reqwest::Proxy::all(format!("socks5h://{addr}"))
                .map_err(|_| "Неверный SOCKS VPN")?
                .basic_auth(user, pass),
        );
    } else if ["socks", "user", "pass"]
        .iter()
        .any(|k| network.get(*k).is_some())
    {
        return Err("Неполный маршрут VPN".into());
    }
    builder.build().map_err(|_| "Не создать HTTP-клиент".into())
}
pub async fn bytes(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, String> {
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Сервис ответил {}", status.as_u16()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "Обрыв ответа сервиса")?
    {
        if bytes.len() + chunk.len() > limit {
            return Err("Слишком большой ответ сервиса".into());
        }
        bytes.extend(chunk);
    }
    Ok(bytes)
}
pub async fn json(response: reqwest::Response) -> Result<Value, String> {
    serde_json::from_slice(&bytes(response, 8 << 20).await?)
        .map_err(|_| "Неверный JSON сервиса".into())
}
pub fn save(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("Нет папки кеша")?;
    std::fs::create_dir_all(parent).map_err(|_| "Не создать папку подключения")?;
    let temp = parent.join(format!(".cache-{}", hex::encode(random::<8>()?)));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(|_| "Не сохранить кеш подключения".into())
}
pub fn load(path: &Path) -> Result<Value, String> {
    let file = std::fs::File::open(path).map_err(|_| "Нет сохранённого списка устройств")?;
    use std::io::Read;
    let mut bytes = Vec::new();
    file.take((4 << 20) + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Не прочитать кеш")?;
    if bytes.len() > 4 << 20 {
        return Err("Слишком большой кеш".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Неверный кеш устройств".into())
}
#[cfg(feature = "qr")]
pub fn qr(text: &str) -> Result<String, String> {
    use qrcode::{Color, EcLevel, QrCode};
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::Q)
        .map_err(|_| "Не создать QR-код")?;
    let width = (code.width() + 8) * 6;
    let mut pixels = vec![255; width * width];
    for y in 0..code.width() {
        for x in 0..code.width() {
            if code[(x, y)] == Color::Dark {
                for dy in 0..6 {
                    for dx in 0..6 {
                        pixels[((y + 4) * 6 + dy) * width + (x + 4) * 6 + dx] = 0;
                    }
                }
            }
        }
    }
    let mut output = Vec::new();
    {
        let mut png = png::Encoder::new(&mut output, width as u32, width as u32);
        png.set_color(png::ColorType::Grayscale);
        png.set_depth(png::BitDepth::Eight);
        png.write_header()
            .map_err(|_| "Не создать PNG")?
            .write_image_data(&pixels)
            .map_err(|_| "Не создать PNG")?;
    }
    Ok(B64.encode(output))
}
