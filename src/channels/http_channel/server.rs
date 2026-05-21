use super::HttpPlatform;
use crate::core::{
    Attachment, FileAttachment, ImageAttachment, Message, MessageType, Platform, ReplyContext,
};
use anyhow::Result;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpInboundMessage {
    pub project: Option<String>,
    pub session_key: String,
    pub user_id: String,
    pub user_name: Option<String>,
    #[serde(default)]
    pub message_type: MessageType,
    pub content: String,
    pub reply_ctx: Option<String>,
    pub message_id: Option<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub images: Vec<ImageAttachment>,
    #[serde(default)]
    pub files: Vec<FileAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HttpOutboundMessage {
    pub kind: String,
    pub reply_ctx: ReplyContext,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
struct HealthResponse {
    ok: bool,
    platform: String,
}

#[derive(Debug, Clone, Serialize)]
struct WebhookResponse {
    ok: bool,
}

pub(super) async fn start_http_server(platform: HttpPlatform) -> Result<()> {
    let listener = TcpListener::bind(&platform.listen).await?;
    let addr = listener.local_addr()?;
    *platform.local_addr.lock().await = Some(addr);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    *platform.shutdown.lock().await = Some(shutdown_tx);

    let platform = Arc::new(platform.clone_for_server());
    let app = Router::new()
        .route("/healthz", get(health))
        .route("/webhook", post(webhook))
        .route("/outbox", get(outbox))
        .with_state(platform);

    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    Ok(())
}

async fn health(State(platform): State<Arc<HttpPlatform>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        platform: platform.name.clone(),
    })
}

async fn webhook(
    State(platform): State<Arc<HttpPlatform>>,
    headers: HeaderMap,
    Json(inbound): Json<HttpInboundMessage>,
) -> Response {
    if let Err(status) = check_auth(&platform, &headers) {
        return status.into_response();
    }
    let Some(handler) = platform.handler.lock().await.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "platform is not started").into_response();
    };

    let message = Message {
        project: inbound.project,
        session_key: inbound.session_key,
        platform: Some(platform.name.clone()),
        message_id: inbound.message_id,
        user_id: inbound.user_id,
        user_name: inbound.user_name,
        content: inbound.content,
        message_type: inbound.message_type,
        attachments: inbound.attachments,
        images: inbound.images,
        files: inbound.files,
        reply_ctx: ReplyContext {
            value: inbound.reply_ctx.unwrap_or_default(),
        },
        created_at: std::time::SystemTime::now(),
    };

    let platform_dyn = platform as Arc<dyn Platform>;
    tokio::spawn(async move {
        handler(platform_dyn, message).await;
    });
    (StatusCode::ACCEPTED, Json(WebhookResponse { ok: true })).into_response()
}

async fn outbox(State(platform): State<Arc<HttpPlatform>>, headers: HeaderMap) -> Response {
    if let Err(status) = check_auth(&platform, &headers) {
        return status.into_response();
    }
    Json(platform.outbox.lock().await.clone()).into_response()
}

fn check_auth(platform: &HttpPlatform, headers: &HeaderMap) -> std::result::Result<(), StatusCode> {
    let Some(token) = &platform.bearer_token else {
        return Ok(());
    };
    let Some(value) = headers.get(axum::http::header::AUTHORIZATION) else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    let Ok(value) = value.to_str() else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    if value == format!("Bearer {token}") {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}
