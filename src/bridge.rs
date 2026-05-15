use crate::core::{
    Attachment, FileAttachment, ImageAttachment, Message, MessageHandler, MessageType, Platform,
    ReplyContext,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex};

pub const MESSAGE_TYPE_REGISTER: &str = "register";
pub const MESSAGE_TYPE_REGISTER_ACK: &str = "register_ack";
pub const MESSAGE_TYPE_MESSAGE: &str = "message";
pub const MESSAGE_TYPE_REPLY: &str = "reply";
pub const MESSAGE_TYPE_PING: &str = "ping";
pub const MESSAGE_TYPE_PONG: &str = "pong";
pub const MESSAGE_TYPE_ERROR: &str = "error";

#[derive(Debug, Clone, Default)]
pub struct BridgePlatformConfig {
    pub name: String,
    pub listen: String,
    pub path: String,
    pub token: Option<String>,
    pub insecure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisterMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    pub platform: String,
    pub project: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InboundMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    pub project: Option<String>,
    pub msg_id: Option<String>,
    pub session_key: String,
    pub user_id: String,
    pub user_name: Option<String>,
    #[serde(default, rename = "message_type")]
    pub content_type: MessageType,
    pub content: String,
    pub reply_ctx: Option<String>,
    #[serde(default)]
    pub attachments: Vec<BridgeAttachment>,
    #[serde(default)]
    pub images: Vec<BridgeBinaryData>,
    #[serde(default)]
    pub files: Vec<BridgeBinaryData>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeBinaryData {
    pub mime_type: String,
    pub data: String,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeAttachment {
    pub kind: crate::core::AttachmentKind,
    pub mime_type: Option<String>,
    pub data: Option<String>,
    pub url: Option<String>,
    pub path: Option<String>,
    pub file_name: Option<String>,
    pub text: Option<String>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplyMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    pub project: Option<String>,
    pub session_key: String,
    pub reply_ctx: Option<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisterAckMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    pub ok: bool,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ErrorMessage {
    #[serde(rename = "type")]
    message_type: String,
    code: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PongMessage {
    #[serde(rename = "type")]
    message_type: String,
    ts: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct BridgeReplyContext {
    platform: String,
    session_key: String,
    reply_ctx: String,
}

#[derive(Debug, Clone)]
struct BridgeAdapter {
    platform: String,
    capabilities: Vec<String>,
    metadata: Option<Value>,
    tx: mpsc::UnboundedSender<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeAdapterInfo {
    pub platform: String,
    pub capabilities: Vec<String>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
struct HealthResponse {
    ok: bool,
    platform: String,
    adapters: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeOutboundRecord {
    pub adapter: String,
    pub message: ReplyMessage,
}

pub struct BridgePlatform {
    name: String,
    listen: String,
    path: String,
    token: Option<String>,
    insecure: bool,
    handler: Arc<Mutex<Option<MessageHandler>>>,
    adapters: Arc<Mutex<HashMap<String, BridgeAdapter>>>,
    outbox: Arc<Mutex<Vec<BridgeOutboundRecord>>>,
    local_addr: Arc<Mutex<Option<SocketAddr>>>,
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl BridgePlatform {
    pub fn new(config: BridgePlatformConfig) -> Arc<Self> {
        Arc::new(Self {
            name: if config.name.trim().is_empty() {
                "bridge".to_string()
            } else {
                config.name
            },
            listen: if config.listen.trim().is_empty() {
                "127.0.0.1:9810".to_string()
            } else {
                config.listen
            },
            path: normalize_path(if config.path.trim().is_empty() {
                "/bridge/ws".to_string()
            } else {
                config.path
            }),
            token: config.token.filter(|v| !v.trim().is_empty()),
            insecure: config.insecure,
            handler: Arc::new(Mutex::new(None)),
            adapters: Arc::new(Mutex::new(HashMap::new())),
            outbox: Arc::new(Mutex::new(Vec::new())),
            local_addr: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn local_addr(&self) -> Option<SocketAddr> {
        *self.local_addr.lock().await
    }

    pub async fn outbox(&self) -> Vec<BridgeOutboundRecord> {
        self.outbox.lock().await.clone()
    }

    pub async fn adapter_infos(&self) -> Vec<BridgeAdapterInfo> {
        self.adapters
            .lock()
            .await
            .values()
            .map(|a| BridgeAdapterInfo {
                platform: a.platform.clone(),
                capabilities: a.capabilities.clone(),
                metadata: a.metadata.clone(),
            })
            .collect()
    }

    async fn send_to_adapter(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx = parse_reply_context(&reply_ctx.value)?;
        let message = ReplyMessage {
            message_type: MESSAGE_TYPE_REPLY.to_string(),
            project: None,
            session_key: ctx.session_key,
            reply_ctx: Some(ctx.reply_ctx),
            content,
            format: Some("text".to_string()),
        };
        let payload = serde_json::to_string(&message)?;
        let adapters = self.adapters.lock().await;
        let Some(adapter) = adapters.get(&ctx.platform) else {
            return Err(anyhow!(
                "bridge adapter `{}` is not connected",
                ctx.platform
            ));
        };
        adapter
            .tx
            .send(payload)
            .map_err(|_| anyhow!("bridge adapter `{}` is disconnected", ctx.platform))?;
        drop(adapters);
        self.outbox.lock().await.push(BridgeOutboundRecord {
            adapter: ctx.platform,
            message,
        });
        Ok(())
    }

    fn clone_for_server(&self) -> Self {
        Self {
            name: self.name.clone(),
            listen: self.listen.clone(),
            path: self.path.clone(),
            token: self.token.clone(),
            insecure: self.insecure,
            handler: Arc::clone(&self.handler),
            adapters: Arc::clone(&self.adapters),
            outbox: Arc::clone(&self.outbox),
            local_addr: Arc::clone(&self.local_addr),
            shutdown: Arc::clone(&self.shutdown),
        }
    }

    fn authenticate(
        &self,
        headers: &HeaderMap,
        token_query: Option<&str>,
    ) -> std::result::Result<(), StatusCode> {
        let Some(expected) = &self.token else {
            return if self.insecure {
                Ok(())
            } else {
                Err(StatusCode::UNAUTHORIZED)
            };
        };
        if token_query.is_some_and(|token| constant_time_eq(token, expected)) {
            return Ok(());
        }
        if header_matches(headers, "x-bridge-token", expected) {
            return Ok(());
        }
        if let Some(value) = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
        {
            if let Some(token) = value.strip_prefix("Bearer ") {
                if constant_time_eq(token, expected) {
                    return Ok(());
                }
            }
        }
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[async_trait]
impl Platform for BridgePlatform {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        if self.token.is_none() && !self.insecure {
            return Err(anyhow!(
                "bridge platform requires token, or set insecure = true for local development"
            ));
        }
        *self.handler.lock().await = Some(handler);

        let listener = TcpListener::bind(&self.listen).await?;
        let addr = listener.local_addr()?;
        *self.local_addr.lock().await = Some(addr);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);

        let state = Arc::new(self.clone_for_server());
        let app = Router::new()
            .route(&self.path, get(ws_handler))
            .route("/bridge/healthz", get(health))
            .route("/bridge/adapters", get(adapters))
            .with_state(state);

        tokio::spawn(async move {
            if let Err(err) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
            {
                tracing::error!(error = %err, "bridge websocket server stopped with error");
            }
        });
        tracing::info!(addr = %addr, path = %self.path, "bridge platform started");
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        self.send_to_adapter(reply_ctx, content).await
    }

    async fn send(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        self.send_to_adapter(reply_ctx, content).await
    }

    async fn stop(&self) -> Result<()> {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct BridgeQuery {
    token: Option<String>,
}

async fn ws_handler(
    State(platform): State<Arc<BridgePlatform>>,
    Query(query): Query<BridgeQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if let Err(status) = platform.authenticate(&headers, query.token.as_deref()) {
        return status.into_response();
    }
    ws.on_upgrade(move |socket| handle_socket(platform, socket))
}

async fn health(State(platform): State<Arc<BridgePlatform>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        platform: platform.name.clone(),
        adapters: platform.adapters.lock().await.len(),
    })
}

async fn adapters(State(platform): State<Arc<BridgePlatform>>) -> Json<Vec<BridgeAdapterInfo>> {
    Json(platform.adapter_infos().await)
}

async fn handle_socket(platform: Arc<BridgePlatform>, socket: WebSocket) {
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(payload) = out_rx.recv().await {
            if ws_tx.send(WsMessage::Text(payload)).await.is_err() {
                break;
            }
        }
    });

    let mut registered_platform: Option<String> = None;
    while let Some(frame) = ws_rx.next().await {
        let Ok(frame) = frame else {
            break;
        };
        let WsMessage::Text(text) = frame else {
            if matches!(frame, WsMessage::Close(_)) {
                break;
            }
            continue;
        };
        if let Err(err) = handle_text_frame(
            Arc::clone(&platform),
            out_tx.clone(),
            &mut registered_platform,
            &text,
        )
        .await
        {
            tracing::warn!(error = %err, "bridge frame handling failed");
            let _ = send_ws_json(
                &out_tx,
                &ErrorMessage {
                    message_type: MESSAGE_TYPE_ERROR.to_string(),
                    code: "bad_message".to_string(),
                    message: err.to_string(),
                },
            );
        }
    }

    if let Some(name) = registered_platform {
        let mut adapters = platform.adapters.lock().await;
        if adapters
            .get(&name)
            .is_some_and(|adapter| adapter.tx.same_channel(&out_tx))
        {
            adapters.remove(&name);
        }
    }
    writer.abort();
}

async fn handle_text_frame(
    platform: Arc<BridgePlatform>,
    out_tx: mpsc::UnboundedSender<String>,
    registered_platform: &mut Option<String>,
    text: &str,
) -> Result<()> {
    let value: Value = serde_json::from_str(text)?;
    let message_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing message type"))?;
    match message_type {
        MESSAGE_TYPE_REGISTER => {
            let register: RegisterMessage = serde_json::from_value(value)?;
            if register.platform.trim().is_empty() {
                return Err(anyhow!("register.platform is required"));
            }
            let adapter_name = register.platform;
            platform.adapters.lock().await.insert(
                adapter_name.clone(),
                BridgeAdapter {
                    platform: adapter_name.clone(),
                    capabilities: register.capabilities,
                    metadata: register.metadata,
                    tx: out_tx.clone(),
                },
            );
            *registered_platform = Some(adapter_name.clone());
            tracing::info!(adapter = %adapter_name, "bridge adapter registered");
            send_ws_json(
                &out_tx,
                &RegisterAckMessage {
                    message_type: MESSAGE_TYPE_REGISTER_ACK.to_string(),
                    ok: true,
                    error: String::new(),
                },
            )?;
        }
        MESSAGE_TYPE_MESSAGE => {
            let adapter_name = registered_platform
                .clone()
                .ok_or_else(|| anyhow!("adapter must register before sending messages"))?;
            let inbound: InboundMessage = serde_json::from_value(value)?;
            let Some(handler) = platform.handler.lock().await.clone() else {
                return Err(anyhow!("bridge platform is not started"));
            };
            let message = inbound.into_core(&platform.name, &adapter_name)?;
            let platform_dyn = platform as Arc<dyn Platform>;
            tokio::spawn(async move {
                handler(platform_dyn, message).await;
            });
        }
        MESSAGE_TYPE_PING => {
            let ts = value.get("ts").and_then(Value::as_i64);
            send_ws_json(
                &out_tx,
                &PongMessage {
                    message_type: MESSAGE_TYPE_PONG.to_string(),
                    ts,
                },
            )?;
        }
        other => return Err(anyhow!("unsupported bridge message type `{other}`")),
    }
    Ok(())
}

impl InboundMessage {
    pub fn into_core(self, platform: &str, adapter: &str) -> Result<Message> {
        let reply_ctx = BridgeReplyContext {
            platform: adapter.to_string(),
            session_key: self.session_key.clone(),
            reply_ctx: self.reply_ctx.unwrap_or_default(),
        };
        Ok(Message {
            project: self.project,
            session_key: self.session_key,
            platform: Some(platform.to_string()),
            message_id: self.msg_id,
            message_type: self.content_type,
            user_id: self.user_id,
            user_name: self.user_name,
            content: self.content,
            attachments: decode_attachments(self.attachments)?,
            images: decode_images(self.images)?,
            files: decode_files(self.files)?,
            reply_ctx: ReplyContext {
                value: serde_json::to_string(&reply_ctx)?,
            },
            created_at: std::time::SystemTime::now(),
        })
    }
}

fn decode_attachments(values: Vec<BridgeAttachment>) -> Result<Vec<Attachment>> {
    values
        .into_iter()
        .map(|v| {
            Ok(Attachment {
                kind: v.kind,
                mime_type: v.mime_type,
                data: v.data.map(|data| STANDARD.decode(data)).transpose()?,
                url: v.url,
                path: v.path,
                file_name: v.file_name,
                text: v.text,
                metadata: v.metadata,
            })
        })
        .collect()
}

fn decode_images(values: Vec<BridgeBinaryData>) -> Result<Vec<ImageAttachment>> {
    values
        .into_iter()
        .map(|v| {
            Ok(ImageAttachment {
                mime_type: v.mime_type,
                data: STANDARD.decode(v.data)?,
                file_name: v.file_name,
            })
        })
        .collect()
}

fn decode_files(values: Vec<BridgeBinaryData>) -> Result<Vec<FileAttachment>> {
    values
        .into_iter()
        .map(|v| {
            Ok(FileAttachment {
                mime_type: v.mime_type,
                data: STANDARD.decode(v.data)?,
                file_name: v.file_name,
            })
        })
        .collect()
}

fn parse_reply_context(value: &str) -> Result<BridgeReplyContext> {
    if let Ok(ctx) = serde_json::from_str::<BridgeReplyContext>(value) {
        if !ctx.platform.trim().is_empty() {
            return Ok(ctx);
        }
    }
    Err(anyhow!(
        "bridge reply context is invalid; expected JSON with platform/session_key/reply_ctx"
    ))
}

fn send_ws_json<T: Serialize>(tx: &mpsc::UnboundedSender<String>, value: &T) -> Result<()> {
    tx.send(serde_json::to_string(value)?)
        .map_err(|_| anyhow!("websocket writer is closed"))
}

fn normalize_path(path: String) -> String {
    if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    }
}

fn string_option(opts: &toml::value::Table, key: &str) -> Option<String> {
    opts.get(key)
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
}

fn bool_option(opts: &toml::value::Table, key: &str) -> Option<bool> {
    opts.get(key).and_then(|v| v.as_bool())
}

fn header_matches(headers: &HeaderMap, name: &str, expected: &str) -> bool {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| constant_time_eq(value, expected))
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .as_bytes()
            .iter()
            .zip(right.as_bytes())
            .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

impl std::fmt::Debug for BridgePlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgePlatform")
            .field("name", &self.name)
            .field("listen", &self.listen)
            .field("path", &self.path)
            .field("token", &self.token.as_ref().map(|_| "***"))
            .field("insecure", &self.insecure)
            .finish()
    }
}

impl TryFrom<toml::value::Table> for BridgePlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let listen = string_option(&opts, "listen").unwrap_or_else(|| "127.0.0.1:9810".to_string());
        if listen.parse::<SocketAddr>().is_err() {
            return Err(anyhow!(
                "bridge platform listen must be a socket address, got `{listen}`"
            ));
        }
        let path = normalize_path(
            string_option(&opts, "path").unwrap_or_else(|| "/bridge/ws".to_string()),
        );
        let insecure = bool_option(&opts, "insecure").unwrap_or(false);
        let token = string_option(&opts, "token");
        if token.as_deref().unwrap_or_default().is_empty() && !insecure {
            return Err(anyhow!(
                "bridge platform requires options.token, or options.insecure = true for local development"
            ));
        }
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "bridge".to_string()),
            listen,
            path,
            token,
            insecure,
        })
    }
}

#[allow(dead_code)]
fn _json(_: Value) {
    let _ = json!({});
}
