use crate::player_script::{Prepared, Scripts};
use jarvis_home_common::{Host, json as response_json};
use reqwest::{Client, Url, header::HeaderMap};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const WEB_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0 Safari/537.36";
pub const WEB_VERSION: &str = "2.20261112.01.00";
pub const TV_AGENT: &str = "Mozilla/5.0 (SMART-TV; Linux; Tizen 2.4.0) AppleWebKit/538.1 (KHTML, like Gecko) Version/2.4.0 TV Safari/538.1";
const TV_VERSION: &str = "5.20260707";
const VR_VERSION: &str = "1.65.10";
const VR_AGENT: &str = "com.google.android.apps.youtube.vr.oculus/1.65.10 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    #[serde(rename = "ID", alias = "id", default)]
    pub id: String,
    #[serde(rename = "Title", alias = "title", default)]
    pub title: String,
    #[serde(rename = "Author", alias = "author", default)]
    pub author: String,
}
impl Track {
    pub fn from_id(id: String) -> Self {
        Self {
            id,
            ..Default::default()
        }
    }
}
#[derive(Clone)]
pub struct Stream {
    pub url: Url,
    pub agent: &'static str,
    pub client: &'static str,
    pub length: u64,
    pub duration: f64,
    pub ready: Instant,
    pub itag: u64,
}
pub struct Remote {
    host: Arc<Host>,
    scripts: Scripts,
    visitor: Mutex<Option<(Instant, String)>>,
}
impl Remote {
    pub fn new(host: Arc<Host>) -> Self {
        Self {
            host,
            scripts: Default::default(),
            visitor: Default::default(),
        }
    }
    pub async fn warm(&self) -> Result<(), String> {
        self.scripts
            .get(&self.host.client().await?)
            .await
            .map(|_| ())
    }
    pub async fn evict_idle(&self, idle: Duration) {
        self.scripts.evict_if_idle(idle).await;
    }
    pub async fn search(&self, query: &str, headers: HeaderMap) -> Result<Vec<Track>, String> {
        let mut body = json!({"context":{"client":web()},"query":query});
        let client = self.host.client().await?;
        for filter in [false, true] {
            if filter {
                body["params"] = "EgIQAQ==".into();
            }
            let data = post(&client, "search", WEB_AGENT, &body, headers.clone()).await?;
            self.note_visitor(&data);
            let section = &data["contents"]["twoColumnSearchResultsRenderer"]["primaryContents"]["sectionListRenderer"]
                ["contents"];
            let mut result = vec![];
            organic(section, &mut result, &mut HashSet::new(), 0);
            if !result.is_empty() {
                return Ok(result);
            }
        }
        Ok(vec![])
    }
    pub async fn recommendations(&self, mut headers: HeaderMap) -> Result<Vec<Track>, String> {
        if headers.is_empty() {
            return Ok(Vec::new());
        }
        let version = "7.20260114.12.00";
        headers.insert(
            "X-YouTube-Client-Name",
            reqwest::header::HeaderValue::from_static("7"),
        );
        headers.insert(
            "X-YouTube-Client-Version",
            reqwest::header::HeaderValue::from_static(version),
        );
        let response = post(&self.host.client().await?, "browse", TV_AGENT, &json!({
            "context":{"client":{"clientName":"TVHTML5","clientVersion":version,"hl":"ru","gl":"RU",
                "platform":"TV","deviceMake":"Samsung","deviceModel":"SmartTV"}},
            "browseId":"FEwhat_to_watch"
        }), headers).await?;
        if response["responseContext"]["mainAppWebResponseContext"]["loggedOut"] == true {
            return Err("YouTube просит войти заново".into());
        }
        let mut tracks = Vec::new();
        organic(&response["contents"], &mut tracks, &mut HashSet::new(), 0);
        Ok(tracks)
    }
    pub async fn related(&self, id: &str, headers: HeaderMap) -> Result<Vec<Track>, String> {
        let client = self.host.client().await?;
        let data = post(
            &client,
            "next",
            WEB_AGENT,
            &json!({"context":{"client":web()},"videoId":id}),
            headers,
        )
        .await?;
        self.note_visitor(&data);
        let items = &data["contents"]["twoColumnWatchNextResults"]["secondaryResults"]["secondaryResults"]
            ["results"];
        let mut result = vec![];
        organic(items, &mut result, &mut HashSet::from([id.to_owned()]), 0);
        Ok(result)
    }
    async fn visitor(&self) -> String {
        let current = || {
            self.visitor
                .lock()
                .unwrap()
                .as_ref()
                .filter(|(at, _)| at.elapsed() < Duration::from_secs(3600))
                .map(|(_, v)| v.clone())
        };
        if let Some(value) = current() {
            return value;
        }
        let _ = self.search("музыка", HeaderMap::new()).await;
        current().unwrap_or_default()
    }
    fn note_visitor(&self, value: &Value) {
        if let Some(visitor) = value["responseContext"]["visitorData"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 1024)
        {
            *self.visitor.lock().unwrap() = Some((Instant::now(), visitor.into()));
        }
    }
    pub async fn resolve(&self, id: &str, auth: HeaderMap) -> Result<(Track, Stream), String> {
        if !valid_id(id) {
            return Err("Неверный ID видео YouTube".into());
        }
        let client = self.host.client().await?;
        let visitor = if auth.is_empty() {
            self.visitor().await
        } else {
            String::new()
        };
        let mut backup = None;
        if auth.is_empty() && !self.scripts.cached() {
            backup = Some(vr(&client, id, &visitor).await);
            if backup.as_ref().is_some_and(|r| {
                r.as_ref().is_ok_and(|(_, stream)| {
                    (0.0..=45.0).contains(&stream.duration) && stream.duration > 0.0
                })
            }) {
                return backup.unwrap();
            }
        }
        let tv=async{
            let prepared=self.scripts.get(&client).await?;
            let mut context=json!({"clientName":"TVHTML5","clientVersion":TV_VERSION,"deviceMake":"Samsung","deviceModel":"UKS9800",
                "userAgent":TV_AGENT,"osName":"Tizen","osVersion":"2.4.0","hl":"ru","gl":"RU"});
            if !visitor.is_empty(){context["visitorData"]=visitor.clone().into();}
            let mut headers=auth.clone();
            if let Ok(value) = reqwest::header::HeaderValue::from_str(&visitor) && !visitor.is_empty() {
                headers.insert("X-Goog-Visitor-Id", value);
            }
            headers.insert("X-YouTube-Client-Name",reqwest::header::HeaderValue::from_static("7"));
            headers.insert("X-YouTube-Client-Version",reqwest::header::HeaderValue::from_static(TV_VERSION));
            headers.insert("Origin",reqwest::header::HeaderValue::from_static("https://www.youtube.com"));
            let body=json!({"context":{"client":context},"videoId":id,
                "playbackContext":{"contentPlaybackContext":{"html5Preference":"HTML5_PREF_WANTS","signatureTimestamp":prepared.timestamp}}});
            let start=Instant::now();
            let data=post(&client,"player",TV_AGENT,&body,headers).await?;
            formats(&data,id,"TVHTML5",TV_AGENT,Some(&prepared),start+ad_wait(&data)).await
        }.await;
        match tv {
            Ok(v) => Ok(v),
            Err(e) if !auth.is_empty() || login_required(&e) => Err(e),
            Err(error) => {
                log::debug!("YouTube: телевизионный плеер недоступен: {error}");
                match backup {
                    Some(Ok(value)) => Ok(value),
                    Some(Err(_)) => Err(error),
                    None => vr(&client, id, &visitor).await,
                }
            }
        }
    }
    pub async fn check_cookies(&self, headers: HeaderMap) -> Result<(), String> {
        let body = json!({"context":{"client":web()},"videoId":"jNQXAC9IVRw"});
        let data = post(
            &self.host.client().await?,
            "player",
            WEB_AGENT,
            &body,
            headers,
        )
        .await?;
        if data["responseContext"]["mainAppWebResponseContext"]["loggedOut"] != false {
            return Err("YouTube не принял куки аккаунта".into());
        }
        Ok(())
    }
}
async fn vr(client: &Client, id: &str, visitor: &str) -> Result<(Track, Stream), String> {
    let body = json!({"context":{"client":{"clientName":"ANDROID_VR","clientVersion":VR_VERSION,"deviceMake":"Oculus",
        "deviceModel":"Quest 3","androidSdkVersion":32,"osName":"Android","osVersion":"12L","visitorData":visitor,"hl":"ru","gl":"RU"}},
        "videoId":id});
    let data = post(client, "player", VR_AGENT, &body, HeaderMap::new()).await?;
    formats(&data, id, "ANDROID_VR", VR_AGENT, None, Instant::now()).await
}
fn web() -> Value {
    json!({"clientName":"WEB","clientVersion":WEB_VERSION,"hl":"ru","gl":"RU"})
}
async fn post(
    client: &Client,
    path: &str,
    agent: &str,
    body: &Value,
    headers: HeaderMap,
) -> Result<Value, String> {
    let response = client
        .post(format!(
            "https://www.youtube.com/youtubei/v1/{path}?prettyPrint=false"
        ))
        .header("User-Agent", agent)
        .headers(headers)
        .json(body)
        .send()
        .await
        .map_err(|_| "Нет соединения с YouTube")?;
    response_json(response).await
}
fn label(v: &Value) -> String {
    if let Some(s) = v.as_str().or_else(|| v["simpleText"].as_str()) {
        return s.chars().take(300).collect();
    }
    v["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["text"].as_str())
        .flat_map(str::chars)
        .take(300)
        .collect()
}
fn organic(value: &Value, out: &mut Vec<Track>, seen: &mut HashSet<String>, depth: u8) {
    if out.len() >= 30 || depth >= 16 {
        return;
    }
    match value {
        Value::Array(list) => {
            for v in list {
                organic(v, out, seen, depth + 1);
            }
        }
        Value::Object(_) => {
            for name in [
                "videoRenderer",
                "compactVideoRenderer",
                "playlistPanelVideoRenderer",
                "gridVideoRenderer",
                "tileRenderer",
                "lockupViewModel",
            ] {
                let Some(item) = value.get(name) else {
                    continue;
                };
                let track = match name {
                    "lockupViewModel" if item["contentType"] == "LOCKUP_CONTENT_TYPE_VIDEO" => {
                        let title =
                            &item["metadata"]["lockupMetadataViewModel"]["title"]["content"];
                        identified(item["contentId"].as_str(), label(title), String::new())
                    }
                    "tileRenderer" if find(item, "adBadgeViewModel", 0).is_none() => {
                        let id = find(item, "watchEndpoint", 0)
                            .and_then(|node| node["videoId"].as_str());
                        let title = find(item, "tileMetadataRenderer", 0)
                            .map(|node| label(&node["title"]))
                            .unwrap_or_default();
                        let author = find(item, "lineItemRenderer", 0)
                            .map(|node| label(&node["text"]))
                            .unwrap_or_default();
                        identified(id, title, author)
                    }
                    "lockupViewModel" | "tileRenderer" => None,
                    _ => {
                        let author = ["ownerText", "longBylineText", "shortBylineText"]
                            .into_iter()
                            .map(|k| label(&item[k]))
                            .find(|s| !s.is_empty())
                            .unwrap_or_default();
                        identified(item["videoId"].as_str(), label(&item["title"]), author)
                    }
                };
                if let Some(track) = track
                    && seen.insert(track.id.clone())
                {
                    out.push(track);
                }
                return;
            }
            for name in [
                "contents",
                "itemSectionRenderer",
                "shelfRenderer",
                "content",
                "verticalListRenderer",
                "items",
                "richItemRenderer",
                "compactAutoplayRenderer",
                "results",
                "tvBrowseRenderer",
                "tvSurfaceContentRenderer",
                "tvPageContentRenderer",
                "gridRenderer",
                "horizontalListRenderer",
                "richGridRenderer",
                "richShelfRenderer",
                "twoColumnBrowseResultsRenderer",
                "tabs",
                "tabRenderer",
            ] {
                if let Some(child) = value.get(name) {
                    organic(child, out, seen, depth + 1);
                }
            }
        }
        _ => {}
    }
}
fn identified(id: Option<&str>, title: String, author: String) -> Option<Track> {
    let id = id.filter(|id| valid_id(id))?;
    (!title.is_empty()).then(|| Track {
        id: id.into(),
        title,
        author,
    })
}
fn find<'a>(value: &'a Value, property: &str, depth: u8) -> Option<&'a Value> {
    if depth > 16 {
        return None;
    }
    match value {
        Value::Object(fields) => fields
            .get(property)
            .or_else(|| fields.values().find_map(|v| find(v, property, depth + 1))),
        Value::Array(items) => items.iter().find_map(|v| find(v, property, depth + 1)),
        _ => None,
    }
}
pub fn valid_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
pub fn video_id(raw: &str) -> Option<String> {
    let url = Url::parse(raw.trim())
        .or_else(|_| Url::parse(&format!("https://{}", raw.trim())))
        .ok()?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let host = url.host_str()?;
    let path = url.path().trim_matches('/');
    if host == "youtu.be" {
        return valid_id(path).then(|| path.into());
    }
    if host != "youtube.com" && !host.ends_with(".youtube.com") {
        return None;
    }
    if path == "watch" {
        return url
            .query_pairs()
            .find(|(k, v)| k == "v" && valid_id(v))
            .map(|(_, v)| v.into_owned());
    }
    let (kind, id) = path.split_once('/')?;
    (matches!(kind, "shorts" | "embed" | "live") && valid_id(id)).then(|| id.into())
}
pub fn login_required(message: &str) -> bool {
    let lower = message.to_lowercase();
    [
        "login_required",
        "age_check_required",
        "sign in",
        "войдите",
        "content_check_required",
    ]
    .iter()
    .any(|s| lower.contains(s))
}
async fn formats(
    data: &Value,
    id: &str,
    client: &'static str,
    agent: &'static str,
    prepared: Option<&Arc<Prepared>>,
    ready: Instant,
) -> Result<(Track, Stream), String> {
    let status = data["playabilityStatus"]["status"]
        .as_str()
        .ok_or("Нет статуса видео YouTube")?;
    if status != "OK" {
        let reason = label(&data["playabilityStatus"]["reason"]);
        return Err(format!("YouTube: {status} {reason}"));
    }
    let track = Track {
        id: id.into(),
        title: label(&data["videoDetails"]["title"]),
        author: label(&data["videoDetails"]["author"]),
    };
    let mut candidates: Vec<_> = data["streamingData"]["adaptiveFormats"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| {
            matches!(item["itag"].as_u64(), Some(249..=251))
                && item["mimeType"]
                    .as_str()
                    .is_some_and(|m| m.starts_with("audio/webm"))
        })
        .collect();
    candidates.sort_by_key(|v| {
        std::cmp::Reverse((
            v["audioTrack"]["audioIsDefault"] != false,
            v["isDrc"] != true,
            v["itag"].as_u64(),
        ))
    });
    for item in candidates {
        let Ok(url) = link(item, prepared).await else {
            continue;
        };
        let length = item["contentLength"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();
        let duration = item["approxDurationMs"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or_default() as f64
            / 1000.0;
        return Ok((
            track,
            Stream {
                url,
                agent,
                client,
                length,
                duration,
                ready,
                itag: item["itag"].as_u64().unwrap(),
            },
        ));
    }
    Err("YouTube не вернул доступную аудиодорожку Opus".into())
}
async fn link(item: &Value, prepared: Option<&Arc<Prepared>>) -> Result<Url, String> {
    let query: BTreeMap<String, String> = reqwest::Url::parse(&format!(
        "https://www.youtube.com/?{}",
        item["signatureCipher"].as_str().unwrap_or("")
    ))
    .map_err(|_| "Неверная ссылка YouTube")?
    .query_pairs()
    .into_owned()
    .collect();
    let raw = item["url"]
        .as_str()
        .or_else(|| query.get("url").map(String::as_str))
        .ok_or("Нет ссылки YouTube")?;
    let mut url = Url::parse(raw).map_err(|_| "Неверная ссылка YouTube")?;
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !url
            .host_str()
            .is_some_and(|h| h.ends_with(".googlevideo.com"))
    {
        return Err("Неверный адрес аудио YouTube".into());
    }
    let signature = query.get("s").cloned().unwrap_or_default();
    let n = url
        .query_pairs()
        .find(|(k, _)| k == "n")
        .map(|(_, n)| n.into_owned())
        .unwrap_or_default();
    if signature.is_empty() && n.is_empty() {
        return Ok(url);
    }
    let prepared = prepared.ok_or("Требуется подпись плеера YouTube")?;
    let sp = query.get("sp").map(String::as_str).unwrap_or("signature");
    if !matches!(sp, "sig" | "signature") {
        return Err("Неизвестный параметр подписи YouTube".into());
    }
    let (new_sig, new_n) = prepared.decode(signature.clone(), n.clone()).await?;
    if (!signature.is_empty() && new_sig.is_empty())
        || (!n.is_empty()
            && (new_n.is_empty() || new_n == n || new_n.starts_with("enhanced_except_")))
    {
        return Err("Плеер не принял подпись YouTube".into());
    }
    let pairs: Vec<_> = url
        .query_pairs()
        .into_owned()
        .filter(|(key, _)| key != "n" && (new_sig.is_empty() || key != sp))
        .collect();
    let mut params = url.query_pairs_mut();
    params.clear().extend_pairs(pairs);
    if !new_sig.is_empty() {
        params.append_pair(sp, &new_sig);
    }
    if !new_n.is_empty() {
        params.append_pair("n", &new_n);
    }
    drop(params);
    Ok(url)
}

fn ad_wait(data: &Value) -> Duration {
    fn scan(value: &Value, total: &mut u64, depth: u8) {
        if depth > 24 || *total >= 60_000 {
            return;
        }
        match value {
            Value::Object(fields) => {
                for (key, child) in fields {
                    if key == "instreamVideoAdRenderer" {
                        let seconds = child["playerVars"].as_str().and_then(|vars| {
                            vars.split('&').find_map(|item| {
                                item.strip_prefix("length_seconds=")?.parse::<f64>().ok()
                            })
                        });
                        let delay = child["skipOffsetMilliseconds"]
                            .as_u64()
                            .or_else(|| seconds.map(|v| (v * 1000.0) as u64))
                            .unwrap_or(0);
                        *total = total.saturating_add(delay);
                    } else {
                        scan(child, total, depth + 1);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    scan(item, total, depth + 1);
                }
            }
            _ => {}
        }
    }
    let mut total = 0;
    for (key, config, property, expected) in [
        (
            "adPlacements",
            "adPlacementConfig",
            "kind",
            "AD_PLACEMENT_KIND_START",
        ),
        (
            "adSlots",
            "adSlotMetadata",
            "triggerEvent",
            "SLOT_TRIGGER_EVENT_BEFORE_CONTENT",
        ),
    ] {
        for entry in data[key].as_array().into_iter().flatten() {
            if find(entry, config, 0).is_some_and(|metadata| metadata[property] == expected) {
                scan(entry, &mut total, 0);
            }
        }
    }
    Duration::from_millis(total.min(60_000))
}
