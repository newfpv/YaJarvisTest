//! Панель вшита в бинарник сжатой: браузеру отдаём gzip как есть, остальным — распакованной.

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::Response,
};
use std::io::Read;

include!(concat!(env!("OUT_DIR"), "/ui_files.rs"));

fn mime(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "webmanifest" => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

fn accepts_gzip(h: &HeaderMap) -> bool {
    h.get_all(header::ACCEPT_ENCODING).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(',')).any(|e| {
        let mut parts = e.split(';');
        parts.next().map(str::trim) == Some("gzip")
            && parts.all(|p| p.trim().strip_prefix("q=").and_then(|q| q.parse::<f32>().ok()).is_none_or(|q| q > 0.0))
    })
}

pub fn find(name: &str) -> Option<&'static [u8]> {
    UI.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
}

pub async fn serve(uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.split('/').any(|part| part == "..") {
        return super::empty(StatusCode::NOT_FOUND);
    }
    let (name, raw) = match find(path) {
        Some(raw) => (path, Some(raw)),
        None if matches!(path.split('/').next(), Some("api" | "assets" | "_app")) => (path, None),
        None => ("index.html", find("index.html")),
    };
    let Some(raw) = raw else { return super::empty(StatusCode::NOT_FOUND) };
    let cache = if name.starts_with("_app/immutable/") || name.starts_with("assets/") {
        HeaderValue::from_static("public, max-age=31536000, immutable")
    } else {
        HeaderValue::from_static("no-cache")
    };
    let compressed = raw.starts_with(&[0x1f, 0x8b]);
    let mut b = Response::builder()
        .header(header::CONTENT_TYPE, mime(name))
        .header(header::CACHE_CONTROL, cache);
    if compressed {
        b = b.header(header::VARY, "Accept-Encoding");
    }
    let body = if !compressed {
        Body::from(raw)
    } else if accepts_gzip(&headers) {
        b = b.header(header::CONTENT_ENCODING, "gzip");
        Body::from(raw)
    } else {
        let mut out = Vec::new();
        if flate2::read::GzDecoder::new(raw).read_to_end(&mut out).is_err() {
            return super::empty(StatusCode::INTERNAL_SERVER_ERROR);
        }
        Body::from(out)
    };
    b.body(body).unwrap()
}
