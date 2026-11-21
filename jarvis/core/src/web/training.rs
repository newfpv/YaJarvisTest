use super::{App, bad, ok, reply};
use crate::settings::MAX_TRAIN_LIMIT_MB;
use crate::training_io::{self, Transfer};
use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Query, State},
    http::{StatusCode, header},
    response::Response,
};
use futures_util::stream::try_unfold;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{io, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt};

const ARCHIVE_TIMEOUT: Duration = Duration::from_secs(300);

pub fn routes() -> axum::Router<App> {
    use axum::routing::{get, post, put};
    axum::Router::new()
        .route("/api/train/records", get(list).put(settings).delete(clear))
        .route("/api/train/records/clip", get(clip))
        .route("/api/train/records/move", post(move_clip))
        .route("/api/train/records/compact", post(compact))
        .route("/api/train/records/download", get(download))
        .route(
            "/api/train/background",
            get(background)
                .post(start_background)
                .delete(stop_background),
        )
        .route(
            "/api/train/background/cloud",
            put(connect).delete(disconnect),
        )
}

async fn request(app: &App, method: &str, params: Value) -> Result<Value, String> {
    training_io::call(&app.plugins, method, params, Duration::from_secs(35)).await
}

fn result(result: Result<Value, String>) -> Response {
    match result {
        Ok(Value::Null) => ok(),
        Ok(value) => reply(StatusCode::OK, value),
        Err(error) => bad(error),
    }
}

async fn list(State(app): State<App>) -> Response {
    let (mode, limit) = app.store.with(|s| (s.train_mode, s.train_limit_mb));
    result(request(&app, "training.records", json!({"mode":mode,"limit":limit})).await)
}

async fn compact(State(app): State<App>) -> Response {
    result(request(&app, "training.compact", Value::Null).await)
}

#[derive(Deserialize)]
struct Settings {
    mode: Option<bool>,
    limit_mb: Option<i64>,
}

async fn settings(State(app): State<App>, Json(input): Json<Settings>) -> Response {
    match app.store.update(|s| -> std::io::Result<()> {
        if let Some(mode) = input.mode {
            s.train_mode = mode;
        }
        if let Some(limit) = input.limit_mb {
            s.train_limit_mb = limit.clamp(1, MAX_TRAIN_LIMIT_MB);
        }
        Ok(())
    }) {
        Ok(()) => ok(),
        Err(error) => bad(error.to_string()),
    }
}

#[derive(Deserialize)]
struct Clip {
    kind: String,
    name: String,
}

async fn clip(State(app): State<App>, Query(input): Query<Clip>) -> Response {
    let transfer = match Transfer::new() {
        Ok(transfer) => transfer,
        Err(error) => return bad(error),
    };
    let params = json!({"kind":input.kind,"name":input.name,"file":transfer.path()});
    if let Err(error) = request(&app, "training.clip", params).await {
        return bad(error);
    }
    let body = match tokio::fs::read(transfer.path()).await {
        Ok(body) => body,
        Err(error) => return bad(error.to_string()),
    };
    Response::builder()
        .header(header::CONTENT_TYPE, "audio/wav")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(body))
        .unwrap()
}

#[derive(Deserialize)]
struct Move {
    #[serde(flatten)]
    clip: Clip,
    to: String,
}

async fn move_clip(State(app): State<App>, Json(input): Json<Move>) -> Response {
    result(
        request(
            &app,
            "training.move",
            json!({"kind":input.clip.kind,"name":input.clip.name,"to":input.to}),
        )
        .await,
    )
}

async fn clear(State(app): State<App>) -> Response {
    result(request(&app, "training.clear", Value::Null).await)
}

async fn download(State(app): State<App>) -> Response {
    let sensitivity = app.store.with(|s| s.wake_sensitivity);
    let (body, length) = match archive(app, sensitivity).await {
        Ok(archive) => archive,
        Err(error) => return bad(error),
    };
    let name = format!(
        "attachment; filename=jarvis-train-{}.zip",
        jiff::Timestamp::now().strftime("%Y%m%d-%H%M%S")
    );
    let mut response = Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_DISPOSITION, name)
        .header(header::CACHE_CONTROL, "no-store");
    if let Some(length) = length {
        response = response.header(header::CONTENT_LENGTH, length);
    }
    response.body(body).unwrap()
}

async fn archive(app: App, sensitivity: i64) -> Result<(Body, Option<u64>), String> {
    // Старые версии плагина используют файл и игнорируют путь сокета.
    let transfer = Transfer::new()?;
    let params = json!({"sensitivity": sensitivity, "file": transfer.path()});
    #[cfg(unix)]
    if let Ok((socket, listener)) = training_io::ArchiveSocket::bind() {
        let mut params = params;
        params["socket"] = json!(socket.path());
        let mut request = tokio::spawn(async move {
            training_io::call(&app.plugins, "training.archive", params, ARCHIVE_TIMEOUT).await
        });
        return tokio::select! {
            biased;
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(|error| error.to_string())?;
                let body = download_stream(stream, async move {
                    request.await.map_err(io::Error::other)?.map_err(io::Error::other)?;
                    Ok(())
                });
                Ok((body, None))
            }
            result = &mut request => {
                result.map_err(|error| error.to_string())??;
                archive_file(transfer).await
            }
        };
    }
    training_io::call(&app.plugins, "training.archive", params, ARCHIVE_TIMEOUT).await?;
    archive_file(transfer).await
}

async fn archive_file(transfer: Transfer) -> Result<(Body, Option<u64>), String> {
    let file = tokio::fs::File::open(transfer.path())
        .await
        .map_err(|error| error.to_string())?;
    let length = file.metadata().await.map_err(|error| error.to_string())?.len();
    let body = download_stream(file, async move {
        drop(transfer);
        Ok(())
    });
    Ok((body, Some(length)))
}

fn download_stream(
    reader: impl AsyncRead + Send + Unpin + 'static,
    finish: impl Future<Output = io::Result<()>> + Send + 'static,
) -> Body {
    Body::from_stream(try_unfold((reader, finish), |(mut reader, finish)| async move {
        let mut chunk = vec![0; 64 * 1024];
        let len = reader.read(&mut chunk).await?;
        if len == 0 {
            drop(reader);
            finish.await?;
            return Ok::<_, io::Error>(None);
        }
        chunk.truncate(len);
        Ok(Some((Bytes::from(chunk), (reader, finish))))
    }))
}

async fn background(State(app): State<App>) -> Response {
    result(request(&app, "training.background.status", Value::Null).await)
}

#[derive(Deserialize)]
struct Background {
    minutes: u32,
}

async fn start_background(State(app): State<App>, Json(input): Json<Background>) -> Response {
    if !app.engine.mic.online()
        || !app
            .engine
            .listener
            .ready
            .load(std::sync::atomic::Ordering::Acquire)
        || !app.store.with(|s| s.wake_enabled)
    {
        return bad("Для записи фона включите микрофон и распознавание фразы активации");
    }
    let started = request(
        &app,
        "training.background.start",
        json!({"minutes":input.minutes}),
    )
    .await;
    if started.is_ok() {
        app.engine.set_background_training(Some(input.minutes));
    }
    result(started)
}

async fn stop_background(State(app): State<App>) -> Response {
    app.engine.set_background_training(None);
    result(request(&app, "training.background.stop", Value::Null).await)
}

async fn connect(State(app): State<App>, Json(input): Json<Value>) -> Response {
    result(request(&app, "training.background.connect", input).await)
}

async fn disconnect(State(app): State<App>) -> Response {
    let disconnected = request(&app, "training.background.disconnect", Value::Null).await;
    if disconnected.is_ok() {
        app.engine.set_background_training(None);
    }
    result(disconnected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn sends_archive_bytes_before_completion_and_reports_producer_errors() {
        let (complete, result) = tokio::sync::oneshot::channel();
        let body = download_stream(&b"ZIP chunk"[..], async move { result.await.unwrap() });
        let mut chunks = body.into_data_stream();

        assert_eq!(chunks.next().await.unwrap().unwrap(), "ZIP chunk");
        complete.send(Err(io::Error::other("archive failed"))).unwrap();
        let error = chunks.next().await.unwrap().unwrap_err();
        assert_eq!(error.to_string(), "archive failed");
    }

    #[tokio::test]
    async fn removes_archive_file_if_download_is_dropped() {
        let transfer = Transfer::new().unwrap();
        let path = transfer.path().to_owned();
        let data = vec![42; 100 * 1024];
        tokio::fs::write(&path, &data).await.unwrap();
        let (body, length) = archive_file(transfer).await.unwrap();
        assert_eq!(length, Some(data.len() as u64));

        let mut chunks = body.into_data_stream();
        let chunk = chunks.next().await.unwrap().unwrap();
        assert_eq!(&chunk[..], &data[..chunk.len()]);
        assert!(chunk.len() < data.len());
        drop(chunks);
        assert!(!path.exists());
    }
}
