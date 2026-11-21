//! Smart Life/Tuya: облачный API с локальным кешем устройств.
//! Локальная сеть вынесена в `lan`; если устройство доступно, control пытается
//! выполнить команду без облака, а затем безопасно откатывается к Tuya API.
mod crypto;
mod fast;
mod lan;

use jarvis_home_common::{Host, load, now, qr, save};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json as j};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::Duration,
};

const CLIENT_ID: &str = "HA_3y9q4ak7g4ephrvke";
const LOGIN_API: &str = "https://apigw.iotbing.com";

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Auth {
    pub user_code: String,
    pub endpoint: String,
    pub terminal_id: String,
    pub uid: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expire: i64,
}
impl Auth {
    fn signed_in(&self) -> bool {
        !self.endpoint.is_empty() && !self.refresh_token.is_empty()
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Function {
    pub code: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub values: String,
    pub dp: i32,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub room: String,
    pub category: String,
    pub product: String,
    pub online: bool,
    pub local_key: String,
    pub funcs: Vec<Function>,
    pub status: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct Login {
    code: String,
    error: String,
    at: i64,
}

pub struct SmartLife {
    host: Arc<Host>,
    lan: Arc<lan::Lan>,
    login: Arc<Mutex<Login>>,
    devices: Mutex<Option<Vec<Device>>>,
    refresh: tokio::sync::Mutex<()>,
    account: tokio::sync::Mutex<()>,
    catalog: tokio::sync::Mutex<()>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    poller: Mutex<Option<tokio::task::JoinHandle<()>>>,
    auth_notify: Arc<tokio::sync::Notify>,
}

impl SmartLife {
    pub fn new(host: Host) -> Self {
        Self {
            host: Arc::new(host),
            lan: Arc::new(lan::Lan::default()),
            login: Arc::new(Mutex::new(Login::default())),
            devices: Mutex::default(),
            refresh: Default::default(),
            account: Default::default(),
            catalog: Default::default(),
            tasks: Default::default(),
            poller: Default::default(),
            auth_notify: Arc::new(tokio::sync::Notify::new()),
        }
    }

    pub fn start_lan(&self) {
        struct AbortOnDrop(Vec<tokio::task::JoinHandle<()>>);
        impl Drop for AbortOnDrop {
            fn drop(&mut self) {
                for task in self.0.drain(..) {
                    task.abort();
                }
            }
        }

        let lan = self.lan.clone();
        let host = self.host.clone();
        let notify = self.auth_notify.clone();
        self.tasks.lock().unwrap().push(tokio::spawn(async move {
            let mut children = AbortOnDrop(Vec::new());
            loop {
                let signed_in = host.config().await.is_ok_and(|c| {
                    c["auth"]["refresh_token"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
                });
                if signed_in && children.0.is_empty() {
                    for port in [6666, 6667, 7000] {
                        let lan = lan.clone();
                        children.0.push(tokio::spawn(async move {
                            loop {
                                lan.listen(port).await;
                                tokio::time::sleep(Duration::from_secs(5)).await;
                            }
                        }));
                    }
                } else if !signed_in && !children.0.is_empty() {
                    for task in children.0.drain(..) {
                        task.abort();
                    }
                }
                tokio::select! {
                    _ = notify.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                }
            }
        }));
    }

    pub fn stop(&self) {
        if let Some(task) = self.poller.lock().unwrap().take() {
            task.abort();
        }
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
        self.host.close();
    }

    async fn config(&self) -> Result<Auth, String> {
        let value = self.host.config().await?;
        serde_json::from_value(
            value
                .get("auth")
                .filter(|v| v.is_object())
                .cloned()
                .unwrap_or_else(|| j!({})),
        )
        .map_err(|_| "Неверные настройки Smart Life".into())
    }
    async fn update_auth(
        &self,
        before: &Auth,
        after: &Auth,
        connected: bool,
    ) -> Result<(), String> {
        self.host
            .set(
                serde_json::to_value(before).unwrap_or_default(),
                serde_json::to_value(after).unwrap_or_default(),
                connected,
            )
            .await
    }
    fn cache_path(&self) -> std::path::PathBuf {
        self.host.data.join("tuya-devices.json")
    }

    pub async fn request(self: &Arc<Self>, p: Value) -> Result<Value, String> {
        tokio::time::timeout(Duration::from_secs(90), self.handle(p))
            .await
            .map_err(|_| "Smart Life не ответил вовремя".to_string())?
    }
    async fn handle(self: &Arc<Self>, p: Value) -> Result<Value, String> {
        if matches!(
            p["op"].as_str(),
            Some("state" | "devices" | "context" | "control" | "fast" | "refresh")
        ) {
            self.host.migrate("tuya-devices.json").await?;
        }
        match p["op"].as_str().unwrap_or("") {
            "login" => {
                self.begin_login(p["args"].as_str().unwrap_or("")).await?;
                Ok(true.into())
            }
            "logout" => {
                self.logout().await?;
                Ok(true.into())
            }
            "login-state" => Ok(self.login_state()),
            "state" => Ok(self.state().await),
            "qr" => Ok(self.qr_image()),
            "refresh" => Ok(self.refresh_devices().await?.into()),
            "devices" => {
                let mut devices = if self.config().await?.signed_in() {
                    self.devices()
                } else {
                    vec![]
                };
                for device in &mut devices {
                    device.local_key.clear();
                }
                Ok(j!(devices))
            }
            "context" => Ok(self.context().await.into()),
            "control" => Ok(self.control(p["args"].as_str().unwrap_or("")).await.into()),
            "fast" => {
                let (handled, answer) = self.fast(p["args"].as_str().unwrap_or("")).await;
                Ok(j!({"handled":handled,"answer":answer}))
            }
            _ => Err("Неизвестная команда Smart Life".into()),
        }
    }

    async fn begin_login(self: &Arc<Self>, user_code: &str) -> Result<(), String> {
        let user_code = user_code.trim();
        if user_code.is_empty() || user_code.len() > 128 {
            return Err("Введите код пользователя Smart Life".into());
        }
        let _account = self.account.lock().await;
        if let Some(task) = self.poller.lock().unwrap().take() {
            task.abort();
        }
        *self.login.lock().unwrap() = Login::default();
        let client = self.host.client().await?;
        let old = self.config().await?;
        let url = format!(
            "{LOGIN_API}/v1.0/m/life/home-assistant/qrcode/tokens?clientid={CLIENT_ID}&usercode={}&schema=haauthorize",
            urlencoding(user_code)
        );
        let response = client
            .post(url)
            .send()
            .await
            .map_err(|_| "Smart Life недоступен".to_string())?;
        let value = jarvis_home_common::json(response).await?;
        let code = value["result"]["qrcode"].as_str().unwrap_or("").to_string();
        if value["success"] != true || code.is_empty() {
            return Err(format!(
                "Smart Life не принял код пользователя: {}",
                value["msg"].as_str().unwrap_or("ошибка")
            ));
        }
        {
            let mut login = self.login.lock().unwrap();
            login.code = code.clone();
            login.at = now();
            login.error.clear();
        }
        let this = self.clone();
        let user_code = user_code.to_owned();
        let task = tokio::spawn(async move {
            let result=tokio::time::timeout(Duration::from_secs(300),async {
                loop {
                    tokio::time::sleep(Duration::from_millis(2500)).await;
                    let Ok(client)=this.host.client().await else {continue};
                    let url=format!("{LOGIN_API}/v1.0/m/life/home-assistant/qrcode/tokens/{}?clientid={CLIENT_ID}&usercode={}",percent(&code),percent(&user_code));
                    let Ok(response)=client.get(url).send().await else {continue};
                    let Ok(value)=jarvis_home_common::json(response).await else {continue};
                    let r=&value["result"];
                    if value["success"]!=true || r["refresh_token"].as_str().is_none_or(str::is_empty){continue;}
                    let mut auth:Auth=serde_json::from_value(r.clone()).map_err(|_|"Неверные настройки входа Smart Life")?;
                    auth.user_code=user_code;
                    auth.expire=expiry(&value["t"],&r["expire_time"]);
                    endpoint(&auth.endpoint)?;
                    let _account=this.account.lock().await;
                    this.update_auth(&old,&auth,true).await?;
                    this.clear_catalog()?;
                    return Ok::<(),String>(());
                }
            }).await;
            let success = matches!(&result, Ok(Ok(())));
            let error = match result {
                Ok(Ok(())) => String::new(),
                Ok(Err(e)) => e,
                Err(_) => "QR-код устарел. Нажмите «Войти» ещё раз".into(),
            };
            *this.login.lock().unwrap() = Login {
                error,
                ..Default::default()
            };
            if success {
                this.auth_notify.notify_one();
                let _ = this.refresh_devices().await;
            }
        });
        *self.poller.lock().unwrap() = Some(task);
        Ok(())
    }
    fn login_state(&self) -> Value {
        let l = self.login.lock().unwrap();
        j!({"code":!l.code.is_empty(),"error":l.error,"at":l.at})
    }
    fn qr_image(&self) -> Value {
        let l = self.login.lock().unwrap();
        if l.code.is_empty() {
            Value::Null
        } else {
            qr(&format!("tuyaSmart--qrLogin?token={}", l.code))
                .map(Value::String)
                .unwrap_or(Value::Null)
        }
    }
    async fn logout(&self) -> Result<(), String> {
        let _account = self.account.lock().await;
        if let Some(task) = self.poller.lock().unwrap().take() {
            task.abort();
        }
        let _refresh = self.refresh.lock().await;
        let old = self.config().await?;
        self.update_auth(&old, &Auth::default(), false).await?;
        self.auth_notify.notify_one();
        *self.login.lock().unwrap() = Login::default();
        self.clear_catalog()
    }

    fn clear_catalog(&self) -> Result<(), String> {
        *self.devices.lock().unwrap() = Some(vec![]);
        save(&self.cache_path(), b"[]")
    }

    async fn refresh_token(&self) -> Result<Auth, String> {
        let _guard = self.refresh.lock().await;
        let auth = self.config().await?;
        if !auth.signed_in() {
            return Err("Smart Life не подключён".into());
        }
        if auth.expire.saturating_sub(60_000) > now() {
            return Ok(auth);
        }
        let mut result = Value::Null;
        self.api_with(
            &auth,
            reqwest::Method::GET,
            &format!("/v1.0/m/token/{}", percent(&auth.refresh_token)),
            None,
            None,
            &mut result,
        )
        .await?;
        let access = result["accessToken"].as_str().unwrap_or("");
        if access.is_empty() {
            return Err("Smart Life не продлил вход".into());
        }
        let mut updated = auth.clone();
        updated.access_token = access.into();
        if let Some(s) = result["refreshToken"].as_str() {
            if !s.is_empty() {
                updated.refresh_token = s.into();
            }
        }
        updated.uid = result["uid"].as_str().unwrap_or(&updated.uid).into();
        updated.expire = expiry(&result["_t"], &result["expireTime"]);
        self.update_auth(&auth, &updated, false).await?;
        Ok(updated)
    }
    async fn api(
        &self,
        method: reqwest::Method,
        path: &str,
        params: Option<Value>,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let auth = self.config().await?;
        if !auth.signed_in() {
            return Err("Smart Life не подключён".into());
        }
        let auth = if !path.starts_with("/v1.0/m/token/") {
            self.refresh_token().await?
        } else {
            auth
        };
        let mut out = Value::Null;
        self.api_with(&auth, method, path, params, body, &mut out)
            .await?;
        Ok(out)
    }
    async fn api_with(
        &self,
        auth: &Auth,
        method: reqwest::Method,
        path: &str,
        params: Option<Value>,
        body: Option<Value>,
        out: &mut Value,
    ) -> Result<(), String> {
        let rid = uuid()?;
        let (hash_key, secret) = crypto::cloud_secret(&rid, &auth.refresh_token);
        let mut url = format!(
            "{}{path}",
            endpoint(&auth.endpoint)?.as_str().trim_end_matches('/')
        );
        let mut query = String::new();
        if let Some(value) = params {
            let enc = crypto::encrypt(&value, &secret)?;
            query = enc.clone();
            url.push_str("?encdata=");
            url.push_str(&percent(&enc));
        }
        let mut body_enc = String::new();
        let request_body = if let Some(value) = body {
            let enc = crypto::encrypt(&value, &secret)?;
            body_enc = enc.clone();
            Some(j!({"encdata":enc}).to_string())
        } else {
            None
        };
        let mut headers = BTreeMap::new();
        headers.insert("X-appKey", CLIENT_ID.to_string());
        headers.insert("X-requestId", rid.clone());
        headers.insert("X-sid", String::new());
        headers.insert("X-time", now().to_string());
        headers.insert("X-token", auth.access_token.clone());
        let mut sign_input = String::new();
        for key in ["X-appKey", "X-requestId", "X-sid", "X-time", "X-token"] {
            if let Some(v) = headers.get(key).filter(|v| !v.is_empty()) {
                if !sign_input.is_empty() {
                    sign_input.push_str("||");
                }
                sign_input.push_str(key);
                sign_input.push('=');
                sign_input.push_str(v);
            }
        }
        let sign = hex::encode(crypto::mac(
            hash_key.as_bytes(),
            format!("{sign_input}{query}{body_enc}").as_bytes(),
        ));
        let client = self.host.client().await?;
        let mut request = client.request(method, url).header("X-sign", sign);
        for (k, v) in headers {
            request = request.header(k, v);
        }
        if request_body.is_some() {
            request = request
                .header("Content-Type", "application/json")
                .body(request_body.unwrap());
        }
        let value = jarvis_home_common::json(
            request
                .send()
                .await
                .map_err(|_| "Smart Life недоступен".to_string())?,
        )
        .await?;
        if value["success"] != true {
            return Err(format!(
                "Smart Life: {} (код {})",
                value["msg"].as_str().unwrap_or("ошибка"),
                value["code"]
            ));
        }
        *out = match value["result"].as_str() {
            Some(enc) => crypto::decrypt(enc, &secret)?,
            None => value["result"].clone(),
        };
        if path.starts_with("/v1.0/m/token/") && out.is_object() {
            out["_t"] = value["t"].clone();
        }
        Ok(())
    }

    async fn refresh_devices(&self) -> Result<usize, String> {
        let _catalog = self.catalog.lock().await;
        let account = self.config().await?.uid;
        let homes = self
            .api(reqwest::Method::GET, "/v1.0/m/life/users/homes", None, None)
            .await?;
        let mut all = vec![];
        let mut seen = std::collections::HashSet::new();
        for h in homes.as_array().into_iter().flatten() {
            let home = h["ownerId"].to_string().trim_matches('"').to_string();
            let list = self
                .api(
                    reqwest::Method::GET,
                    "/v1.0/m/life/ha/home/devices",
                    Some(j!({"homeId":home})),
                    None,
                )
                .await?;
            for x in list.as_array().into_iter().flatten() {
                let id = x["id"].as_str().unwrap_or("");
                if id.is_empty() || !seen.insert(id.to_string()) {
                    continue;
                }
                let mut d = Device {
                    id: id.into(),
                    name: x["name"].as_str().unwrap_or("").into(),
                    category: x["category"].as_str().unwrap_or("").into(),
                    product: x["product_name"].as_str().unwrap_or("").into(),
                    online: x["online"].as_bool().unwrap_or(false),
                    local_key: if x["sub"].as_bool().unwrap_or(false) {
                        String::new()
                    } else {
                        x["local_key"].as_str().unwrap_or("").into()
                    },
                    ..Default::default()
                };
                self.describe(&mut d).await;
                all.push(d);
            }
        }
        let _account = self.account.lock().await;
        let current = self.config().await?;
        if !current.signed_in() || current.uid != account {
            return Err("Аккаунт Smart Life изменился".into());
        }
        all.sort_by_key(|d| format!("{}{}", d.room.to_lowercase(), d.name.to_lowercase()));
        save(
            &self.cache_path(),
            &serde_json::to_vec(&all).unwrap_or_default(),
        )?;
        *self.devices.lock().unwrap() = Some(all.clone());
        Ok(all.len())
    }
    async fn describe(&self, d: &mut Device) {
        if let Ok(spec) = self
            .api(
                reqwest::Method::GET,
                &format!("/v1.1/m/life/{}/specifications", percent(&d.id)),
                None,
                None,
            )
            .await
        {
            for f in spec["functions"].as_array().into_iter().flatten() {
                d.funcs.push(Function {
                    code: f["code"].as_str().unwrap_or("").into(),
                    kind: f["type"].as_str().unwrap_or("").into(),
                    values: f["values"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(300)
                        .collect(),
                    dp: 0,
                });
            }
            for s in spec["status"].as_array().into_iter().flatten() {
                if let Some(c) = s["code"].as_str() {
                    d.status.push(c.into());
                }
            }
        }
        if let Ok(rel) = self
            .api(
                reqwest::Method::GET,
                &format!("/v1.0/m/life/devices/{}/status", percent(&d.id)),
                None,
                None,
            )
            .await
        {
            let mut map = HashMap::new();
            for x in rel["dpStatusRelationDTOS"].as_array().into_iter().flatten() {
                if x["supportLocal"].as_bool().unwrap_or(false)
                    && ["", "default"].contains(&x["valueConvert"].as_str().unwrap_or(""))
                {
                    map.insert(
                        x["statusCode"].as_str().unwrap_or(""),
                        x["dpId"].as_i64().unwrap_or(0) as i32,
                    );
                }
            }
            for f in &mut d.funcs {
                f.dp = *map.get(f.code.as_str()).unwrap_or(&0);
            }
        }
        if let Ok(room) = self
            .api(
                reqwest::Method::GET,
                &format!("/v1.0/m/thing/ha/{}/room", percent(&d.id)),
                None,
                None,
            )
            .await
        {
            d.room = room["name"].as_str().unwrap_or("").into();
        }
    }
    fn devices(&self) -> Vec<Device> {
        let mut cache = self.devices.lock().unwrap();
        cache
            .get_or_insert_with(|| {
                load(&self.cache_path())
                    .ok()
                    .and_then(|v| serde_json::from_value(v).ok())
                    .unwrap_or_default()
            })
            .clone()
    }
    async fn state(&self) -> Value {
        let a = self.config().await.unwrap_or_default();
        let ds = self.devices();
        j!({"signed_in":a.signed_in(),"login":self.login_state(),"devices":ds.iter().map(|d|j!({"name":d.name,"room":d.room,"product":d.product,"online":d.online,"local":!d.local_key.is_empty()&&self.lan.known(&d.id)})).collect::<Vec<_>>()})
    }
    async fn control(&self, args: &str) -> String {
        let Ok(a) = serde_json::from_str::<Control>(args) else {
            return "Не понял команду для Smart Life.".into();
        };
        let Some(d) = self
            .devices()
            .into_iter()
            .find(|d| d.id == a.id || d.name.to_lowercase() == a.id.to_lowercase())
        else {
            return "Такого устройства Smart Life нет в списке.".into();
        };
        if a.action == "get" {
            let Ok(v) = self
                .api(
                    reqwest::Method::GET,
                    "/v1.0/m/life/ha/devices/detail",
                    Some(j!({"devIds":d.id})),
                    None,
                )
                .await
            else {
                return "Smart Life не ответил.".into();
            };
            let status: serde_json::Map<String, Value> = v
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|item| item["status"].as_array().into_iter().flatten())
                .filter_map(|entry| {
                    Some((entry["code"].as_str()?.to_owned(), entry["value"].clone()))
                })
                .collect();
            return format!("«{}»: {}", d.name, j!(status));
        };
        if a.action != "set" {
            return "Неизвестное действие Smart Life.".into();
        }
        if a.commands.is_empty() {
            return "Что изменить? Передай commands.".into();
        }
        match self.set(&d, &a.commands).await {
            Ok(_) => format!("Готово: «{}».", d.name),
            Err(e) => format!("Smart Life не выполнил: {}", e),
        }
    }
    async fn set(&self, d: &Device, commands: &HashMap<String, Value>) -> Result<(), String> {
        if commands.len() > 32
            || commands
                .keys()
                .any(|code| !d.funcs.iter().any(|f| &f.code == code))
        {
            return Err("Неизвестная функция устройства".into());
        }
        let mut dps = BTreeMap::new();
        for (c, v) in commands {
            if let Some(f) = d.funcs.iter().find(|f| f.code == *c) {
                if f.dp > 0 {
                    dps.insert(f.dp.to_string(), v.clone());
                }
            }
        }
        if !dps.is_empty() && dps.len() == commands.len() && !d.local_key.is_empty() {
            if self
                .lan
                .control(d.id.as_str(), &d.local_key, &dps)
                .await
                .is_ok()
            {
                return Ok(());
            }
        }
        let list: Vec<_> = commands
            .iter()
            .map(|(code, value)| j!({"code":code,"value":value}))
            .collect();
        self.api(
            reqwest::Method::POST,
            &format!("/v1.1/m/thing/{}/commands", percent(&d.id)),
            None,
            Some(j!({"commands":list})),
        )
        .await
        .map(|_| ())
    }
    async fn context(&self) -> String {
        let a = self.config().await.unwrap_or_default();
        if !a.signed_in() {
            return String::new();
        }
        let ds = self.devices();
        if ds.is_empty() {
            return String::new();
        }
        let mut s:String="\n\nУстройства Smart Life (управление — инструмент smart_life; id и коды функций ровно как здесь, значения — по типу и диапазону):".into();
        for d in ds {
            s.push_str(&format!(
                "\n- id {} «{}» ({}, {}, {})",
                d.id,
                d.name,
                if d.room.is_empty() { "—" } else { &d.room },
                if d.product.is_empty() {
                    "—"
                } else {
                    &d.product
                },
                if d.online {
                    "в сети"
                } else {
                    "не в сети"
                }
            ));
            if !d.funcs.is_empty() {
                s.push_str(": ");
                s.push_str(
                    &d.funcs
                        .iter()
                        .map(|f| {
                            format!(
                                "{} [{}{}]",
                                f.code,
                                f.kind,
                                if f.values.len() <= 160 && f.values != "{}" && !f.values.is_empty()
                                {
                                    format!(" {}", f.values)
                                } else {
                                    String::new()
                                }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; "),
                );
            }
        }
        s
    }
    async fn fast(self: &Arc<Self>, text: &str) -> (bool, String) {
        let Ok(config) = self.host.config().await else {
            return (false, String::new());
        };
        if config["auth"]["refresh_token"]
            .as_str()
            .is_none_or(str::is_empty)
        {
            return (false, String::new());
        }
        let Some((on, picked)) = fast::pick(
            text,
            self.devices(),
            config["speaker_room"].as_str().unwrap_or(""),
        ) else {
            return (false, String::new());
        };
        let count = picked.len();
        let mut tasks = tokio::task::JoinSet::new();
        for device in picked {
            let this = self.clone();
            tasks.spawn(async move {
                let commands = fast::switches(&device)
                    .map(|f| (f.code.clone(), Value::Bool(on)))
                    .collect();
                this.set(&device, &commands).await.map_err(|_| device.name)
            });
        }
        let mut failed = vec![];
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(Ok(())) => {}
                Ok(Err(name)) => failed.push(name),
                Err(_) => failed.push("устройство".into()),
            }
        }
        failed.sort();
        (
            true,
            if failed.is_empty() {
                "Готово.".into()
            } else if failed.len() == count {
                format!("Не получилось: {}.", failed.join(", "))
            } else {
                format!("Готово, кроме: {}.", failed.join(", "))
            },
        )
    }
}

#[derive(Deserialize)]
struct Control {
    id: String,
    action: String,
    #[serde(default)]
    commands: HashMap<String, Value>,
}
fn uuid() -> Result<String, String> {
    let mut b = jarvis_home_common::random::<16>()?;
    b[6] = (b[6] & 15) | 64;
    b[8] = (b[8] & 63) | 128;
    let h = hex::encode(b);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    ))
}
fn urlencoding(s: &str) -> String {
    percent(s)
}
fn percent(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            o.push(b as char)
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

impl Drop for SmartLife {
    fn drop(&mut self) {
        self.stop();
    }
}
fn expiry(time: &Value, seconds: &Value) -> i64 {
    time.as_i64()
        .filter(|t| *t > 0)
        .unwrap_or_else(now)
        .saturating_add(seconds.as_i64().unwrap_or(0).saturating_mul(1000))
}
fn endpoint(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "Неверный адрес Smart Life")?;
    let host = url.host_str().unwrap_or("");
    let allowed = [
        "tuyaeu.com",
        "tuyaus.com",
        "tuyaus.com.cn",
        "tuyacn.com",
        "tuyain.com",
        "tuya.com",
        "iotbing.com",
    ];
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !allowed.iter().any(|d| {
            host == *d
                || host
                    .strip_suffix(d)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
    {
        return Err("Неверный адрес Smart Life".into());
    }
    Ok(url)
}
