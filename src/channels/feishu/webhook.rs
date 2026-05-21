use super::{payload, FeishuPlatform};
use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

pub(super) async fn start_http_server(
    platform: FeishuPlatform,
    shutdown_rx: oneshot::Receiver<()>,
) -> Result<()> {
    let listener = TcpListener::bind(&platform.config.listen)
        .await
        .with_context(|| {
            format!(
                "failed to bind feishu webhook listen {}",
                platform.config.listen
            )
        })?;
    let addr = listener.local_addr()?;
    *platform.local_addr.lock().await = Some(addr);
    let app = Router::new()
        .route(&platform.config.callback_path, post(feishu_webhook))
        .route("/feishu/healthz", get(feishu_health))
        .with_state(Arc::new(platform.clone_for_server()));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    tracing::info!(addr = %addr, path = %platform.config.callback_path, mode = "webhook", "feishu platform started");
    Ok(())
}

pub(super) async fn feishu_health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

pub(super) async fn feishu_webhook(
    State(platform): State<Arc<FeishuPlatform>>,
    Json(payload): Json<Value>,
) -> Response {
    if payload.get("type").and_then(Value::as_str) == Some("url_verification") {
        return Json(
            json!({ "challenge": payload.get("challenge").cloned().unwrap_or(Value::Null) }),
        )
        .into_response();
    }
    if let Some(expected) = &platform.config.verification_token {
        let token = payload
            .get("token")
            .or_else(|| payload.pointer("/header/token"))
            .and_then(Value::as_str);
        if token != Some(expected.as_str()) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    match payload::dispatch(Arc::clone(&platform), &payload).await {
        Ok(Some(true)) => Json(json!({ "ok": true })).into_response(),
        Ok(None) => Json(json!({ "ok": true, "ignored": true })).into_response(),
        Ok(Some(false)) => Json(json!({ "ok": true, "ignored": true })).into_response(),
        Err(err) => {
            tracing::warn!(error = %err, "feishu webhook parse failed");
            (StatusCode::BAD_REQUEST, err.to_string()).into_response()
        }
    }
}

pub(super) fn normalize_path(path: String) -> String {
    if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    }
}
