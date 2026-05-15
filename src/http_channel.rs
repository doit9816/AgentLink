use crate::core::{
    Attachment, FileAttachment, ImageAttachment, Message, MessageHandler, MessageType, Platform,
    ReplyContext,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex};

#[derive(Debug, Clone, Default)]
pub struct HttpPlatformConfig {
    pub name: String,
    pub listen: String,
    pub bearer_token: Option<String>,
    pub outbound_url: Option<String>,
}

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

pub struct HttpPlatform {
    name: String,
    listen: String,
    bearer_token: Option<String>,
    outbound_url: Option<String>,
    handler: Arc<Mutex<Option<MessageHandler>>>,
    outbox: Arc<Mutex<Vec<HttpOutboundMessage>>>,
    out_tx: mpsc::UnboundedSender<HttpOutboundMessage>,
    out_rx: Arc<Mutex<mpsc::UnboundedReceiver<HttpOutboundMessage>>>,
    local_addr: Arc<Mutex<Option<SocketAddr>>>,
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    client: reqwest::Client,
}

impl HttpPlatform {
    pub fn new(config: HttpPlatformConfig) -> Arc<Self> {
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            name: if config.name.trim().is_empty() {
                "http".to_string()
            } else {
                config.name
            },
            listen: if config.listen.trim().is_empty() {
                "127.0.0.1:18080".to_string()
            } else {
                config.listen
            },
            bearer_token: config.bearer_token.filter(|v| !v.trim().is_empty()),
            outbound_url: config.outbound_url.filter(|v| !v.trim().is_empty()),
            handler: Arc::new(Mutex::new(None)),
            outbox: Arc::new(Mutex::new(Vec::new())),
            out_tx,
            out_rx: Arc::new(Mutex::new(out_rx)),
            local_addr: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(Mutex::new(None)),
            client: reqwest::Client::new(),
        })
    }

    pub fn from_options(opts: toml::value::Table) -> Self {
        let config = HttpPlatformConfig {
            name: string_option(&opts, "name").unwrap_or_else(|| "http".to_string()),
            listen: string_option(&opts, "listen").unwrap_or_else(|| "127.0.0.1:18080".to_string()),
            bearer_token: string_option(&opts, "bearer_token"),
            outbound_url: string_option(&opts, "outbound_url"),
        };
        let platform = Self::new(config);
        Arc::try_unwrap(platform).unwrap_or_else(|_| unreachable!("new platform has one owner"))
    }

    pub async fn local_addr(&self) -> Option<SocketAddr> {
        *self.local_addr.lock().await
    }

    pub async fn outbox(&self) -> Vec<HttpOutboundMessage> {
        self.outbox.lock().await.clone()
    }

    pub async fn wait_for_outbound(&self) -> Option<HttpOutboundMessage> {
        self.out_rx.lock().await.recv().await
    }

    async fn record_and_forward(
        &self,
        kind: &str,
        reply_ctx: ReplyContext,
        content: String,
    ) -> Result<()> {
        let msg = HttpOutboundMessage {
            kind: kind.to_string(),
            reply_ctx,
            content,
        };
        self.outbox.lock().await.push(msg.clone());
        let _ = self.out_tx.send(msg.clone());

        if let Some(url) = &self.outbound_url {
            self.client
                .post(url)
                .json(&msg)
                .send()
                .await?
                .error_for_status()?;
        }
        Ok(())
    }

    fn check_auth(&self, headers: &HeaderMap) -> std::result::Result<(), StatusCode> {
        let Some(token) = &self.bearer_token else {
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
}

#[async_trait]
impl Platform for HttpPlatform {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);

        let listener = TcpListener::bind(&self.listen).await?;
        let addr = listener.local_addr()?;
        *self.local_addr.lock().await = Some(addr);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);

        let platform = Arc::new(self.clone_for_server());
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

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        self.record_and_forward("reply", reply_ctx, content).await
    }

    async fn send(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        self.record_and_forward("send", reply_ctx, content).await
    }

    async fn stop(&self) -> Result<()> {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

impl HttpPlatform {
    fn clone_for_server(&self) -> Self {
        Self {
            name: self.name.clone(),
            listen: self.listen.clone(),
            bearer_token: self.bearer_token.clone(),
            outbound_url: self.outbound_url.clone(),
            handler: Arc::clone(&self.handler),
            outbox: Arc::clone(&self.outbox),
            out_tx: self.out_tx.clone(),
            out_rx: Arc::clone(&self.out_rx),
            local_addr: Arc::clone(&self.local_addr),
            shutdown: Arc::clone(&self.shutdown),
            client: self.client.clone(),
        }
    }
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
    if let Err(status) = platform.check_auth(&headers) {
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
    if let Err(status) = platform.check_auth(&headers) {
        return status.into_response();
    }
    Json(platform.outbox.lock().await.clone()).into_response()
}

fn string_option(opts: &toml::value::Table, key: &str) -> Option<String> {
    opts.get(key)
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
}

impl std::fmt::Debug for HttpPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpPlatform")
            .field("name", &self.name)
            .field("listen", &self.listen)
            .field("bearer_token", &self.bearer_token.as_ref().map(|_| "***"))
            .field("outbound_url", &self.outbound_url)
            .finish()
    }
}

impl TryFrom<toml::value::Table> for HttpPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let listen =
            string_option(&opts, "listen").unwrap_or_else(|| "127.0.0.1:18080".to_string());
        if listen.parse::<SocketAddr>().is_err() {
            return Err(anyhow!(
                "http platform listen must be a socket address, got `{listen}`"
            ));
        }
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "http".to_string()),
            listen,
            bearer_token: string_option(&opts, "bearer_token"),
            outbound_url: string_option(&opts, "outbound_url"),
        })
    }
}
