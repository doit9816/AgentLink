use crate::core::{Message, MessageHandler, Platform, ReplyContext};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

const FEISHU_WS_ENDPOINT_PATH: &str = "/callback/ws/endpoint";
const HEADER_TYPE: &str = "type";
const HEADER_BIZ_RT: &str = "biz_rt";
const MESSAGE_TYPE_EVENT: &str = "event";
const MESSAGE_TYPE_PING: &str = "ping";
const MESSAGE_TYPE_PONG: &str = "pong";
const FRAME_TYPE_CONTROL: i32 = 0;
const FRAME_TYPE_DATA: i32 = 1;

#[derive(Debug, Clone, Default)]
pub struct FeishuPlatformConfig {
    pub name: String,
    pub app_id: String,
    pub app_secret: String,
    pub api_base: String,
    pub connection_mode: String,
    pub listen: String,
    pub callback_path: String,
    pub verification_token: Option<String>,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeishuOutboundRecord {
    pub chat_id: String,
    pub message_id: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FeishuReplyContext {
    message_id: Option<String>,
    chat_id: String,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct FeishuWsHeader {
    #[prost(string, required, tag = "1")]
    key: String,
    #[prost(string, required, tag = "2")]
    value: String,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct FeishuWsFrame {
    #[prost(uint64, required, tag = "1")]
    seq_id: u64,
    #[prost(uint64, required, tag = "2")]
    log_id: u64,
    #[prost(int32, required, tag = "3")]
    service: i32,
    #[prost(int32, required, tag = "4")]
    method: i32,
    #[prost(message, repeated, tag = "5")]
    headers: Vec<FeishuWsHeader>,
    #[prost(string, optional, tag = "6")]
    payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    payload_type: Option<String>,
    #[prost(bytes, optional, tag = "8")]
    payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    log_id_new: Option<String>,
}

pub struct FeishuPlatform {
    config: FeishuPlatformConfig,
    handler: Arc<Mutex<Option<MessageHandler>>>,
    outbox: Arc<Mutex<Vec<FeishuOutboundRecord>>>,
    out_tx: mpsc::UnboundedSender<FeishuOutboundRecord>,
    out_rx: Arc<Mutex<mpsc::UnboundedReceiver<FeishuOutboundRecord>>>,
    local_addr: Arc<Mutex<Option<SocketAddr>>>,
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    client: reqwest::Client,
    token: Arc<Mutex<Option<CachedToken>>>,
}

#[derive(Debug, Clone)]
struct CachedToken {
    value: String,
    expires_at: Instant,
}

impl FeishuPlatform {
    pub fn new(config: FeishuPlatformConfig) -> Arc<Self> {
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            config: FeishuPlatformConfig {
                name: if config.name.trim().is_empty() {
                    "feishu".to_string()
                } else {
                    config.name
                },
                api_base: if config.api_base.trim().is_empty() {
                    "https://open.feishu.cn".to_string()
                } else {
                    config.api_base.trim_end_matches('/').to_string()
                },
                connection_mode: if config.connection_mode.trim().is_empty() {
                    "websocket".to_string()
                } else {
                    config.connection_mode.trim().to_ascii_lowercase()
                },
                listen: if config.listen.trim().is_empty() {
                    "127.0.0.1:18200".to_string()
                } else {
                    config.listen
                },
                callback_path: normalize_path(if config.callback_path.trim().is_empty() {
                    "/feishu/webhook".to_string()
                } else {
                    config.callback_path
                }),
                ..config
            },
            handler: Arc::new(Mutex::new(None)),
            outbox: Arc::new(Mutex::new(Vec::new())),
            out_tx,
            out_rx: Arc::new(Mutex::new(out_rx)),
            local_addr: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(Mutex::new(None)),
            client: reqwest::Client::new(),
            token: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn local_addr(&self) -> Option<SocketAddr> {
        *self.local_addr.lock().await
    }

    pub async fn wait_for_outbound(&self) -> Option<FeishuOutboundRecord> {
        self.out_rx.lock().await.recv().await
    }

    async fn tenant_access_token(&self) -> Result<String> {
        if let Some(cached) = self.token.lock().await.clone() {
            if Instant::now() < cached.expires_at {
                return Ok(cached.value);
            }
        }
        let resp: Value = self
            .client
            .post(format!(
                "{}/open-apis/auth/v3/tenant_access_token/internal",
                self.config.api_base
            ))
            .json(&json!({
                "app_id": self.config.app_id,
                "app_secret": self.config.app_secret
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let token = resp
            .get("tenant_access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("feishu token response missing tenant_access_token"))?
            .to_string();
        let expire = resp.get("expire").and_then(Value::as_u64).unwrap_or(7200);
        *self.token.lock().await = Some(CachedToken {
            value: token.clone(),
            expires_at: Instant::now() + Duration::from_secs(expire.saturating_sub(300)),
        });
        Ok(token)
    }

    fn should_use_websocket(&self) -> bool {
        self.config.connection_mode == "websocket"
            && !(self.config.dry_run && self.config.app_id.trim().is_empty())
    }

    async fn websocket_endpoint(&self) -> Result<FeishuWsEndpoint> {
        #[derive(Debug, Deserialize)]
        struct EndpointResponse {
            code: i64,
            msg: Option<String>,
            data: Option<EndpointData>,
        }
        #[derive(Debug, Deserialize)]
        struct EndpointData {
            #[serde(rename = "URL")]
            url: String,
            #[serde(rename = "ClientConfig")]
            client_config: Option<EndpointClientConfig>,
        }
        #[derive(Debug, Deserialize)]
        struct EndpointClientConfig {
            #[serde(rename = "PingInterval")]
            ping_interval: Option<u64>,
        }

        let resp: EndpointResponse = self
            .client
            .post(format!(
                "{}{}",
                self.config.api_base, FEISHU_WS_ENDPOINT_PATH
            ))
            .header("locale", "zh")
            .json(&json!({
                "AppID": self.config.app_id,
                "AppSecret": self.config.app_secret,
            }))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if resp.code != 0 {
            return Err(anyhow!(
                "feishu websocket endpoint failed: code={}, msg={}",
                resp.code,
                resp.msg.unwrap_or_default()
            ));
        }
        let data = resp
            .data
            .ok_or_else(|| anyhow!("feishu websocket endpoint response missing data"))?;
        if data.url.trim().is_empty() {
            return Err(anyhow!("feishu websocket endpoint response missing URL"));
        }
        let cfg = data.client_config;
        Ok(FeishuWsEndpoint {
            url: data.url,
            ping_interval: cfg.as_ref().and_then(|v| v.ping_interval).unwrap_or(120),
        })
    }

    fn clone_for_server(&self) -> Self {
        Self {
            config: self.config.clone(),
            handler: Arc::clone(&self.handler),
            outbox: Arc::clone(&self.outbox),
            out_tx: self.out_tx.clone(),
            out_rx: Arc::clone(&self.out_rx),
            local_addr: Arc::clone(&self.local_addr),
            shutdown: Arc::clone(&self.shutdown),
            client: self.client.clone(),
            token: Arc::clone(&self.token),
        }
    }
}

#[async_trait]
impl Platform for FeishuPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);

        if self.should_use_websocket() {
            let platform = Arc::new(self.clone_for_server());
            tokio::spawn(async move {
                run_feishu_websocket(platform, shutdown_rx).await;
            });
            tracing::info!(mode = "websocket", api_base = %self.config.api_base, "feishu platform started");
            return Ok(());
        }

        let listener = TcpListener::bind(&self.config.listen)
            .await
            .with_context(|| {
                format!(
                    "failed to bind feishu webhook listen {}",
                    self.config.listen
                )
            })?;
        let addr = listener.local_addr()?;
        *self.local_addr.lock().await = Some(addr);
        let app = Router::new()
            .route(&self.config.callback_path, post(feishu_webhook))
            .route("/feishu/healthz", get(feishu_health))
            .with_state(Arc::new(self.clone_for_server()));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        tracing::info!(addr = %addr, path = %self.config.callback_path, mode = "webhook", "feishu platform started");
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: FeishuReplyContext = serde_json::from_str(&reply_ctx.value)?;
        let record = FeishuOutboundRecord {
            chat_id: ctx.chat_id.clone(),
            message_id: ctx.message_id.clone(),
            content: content.clone(),
        };
        self.outbox.lock().await.push(record.clone());
        let _ = self.out_tx.send(record);

        if self.config.dry_run {
            return Ok(());
        }
        let token = self.tenant_access_token().await?;
        let body = json!({
            "msg_type": "text",
            "content": serde_json::to_string(&json!({ "text": content }))?
        });
        let has_message_id = ctx.message_id.is_some();
        let url = if let Some(message_id) = &ctx.message_id {
            format!(
                "{}/open-apis/im/v1/messages/{}/reply",
                self.config.api_base, message_id
            )
        } else {
            format!(
                "{}/open-apis/im/v1/messages?receive_id_type=chat_id",
                self.config.api_base
            )
        };
        let body = if has_message_id {
            body
        } else {
            json!({
                "receive_id": ctx.chat_id,
                "msg_type": "text",
                "content": serde_json::to_string(&json!({ "text": content }))?
            })
        };
        self.client
            .post(url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    async fn send(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        self.reply(reply_ctx, content).await
    }

    async fn stop(&self) -> Result<()> {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

async fn feishu_health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

async fn feishu_webhook(
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
    match dispatch_feishu_payload(Arc::clone(&platform), &payload).await {
        Ok(Some(true)) => Json(json!({ "ok": true })).into_response(),
        Ok(None) => Json(json!({ "ok": true, "ignored": true })).into_response(),
        Ok(Some(false)) => Json(json!({ "ok": true, "ignored": true })).into_response(),
        Err(err) => {
            tracing::warn!(error = %err, "feishu webhook parse failed");
            (StatusCode::BAD_REQUEST, err.to_string()).into_response()
        }
    }
}

#[derive(Debug, Clone)]
struct FeishuWsEndpoint {
    url: String,
    ping_interval: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeishuWsExit {
    Reconnect,
    Shutdown,
}

async fn run_feishu_websocket(
    platform: Arc<FeishuPlatform>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    loop {
        match run_feishu_websocket_once(Arc::clone(&platform), &mut shutdown_rx).await {
            Ok(FeishuWsExit::Shutdown) => return,
            Ok(FeishuWsExit::Reconnect) => {}
            Err(err) => {
                tracing::warn!(error = %err, "feishu websocket disconnected");
            }
        }
        tokio::select! {
            _ = &mut shutdown_rx => return,
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
        }
    }
}

async fn run_feishu_websocket_once(
    platform: Arc<FeishuPlatform>,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> Result<FeishuWsExit> {
    let endpoint = platform.websocket_endpoint().await?;
    let service_id = query_param(&endpoint.url, "service_id")
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(0);
    let (stream, _) = connect_async(&endpoint.url).await?;
    tracing::info!(url = %mask_ws_url(&endpoint.url), "feishu websocket connected");

    let (mut write, mut read) = stream.split();
    let mut ping = tokio::time::interval(Duration::from_secs(endpoint.ping_interval.clamp(1, 600)));
    loop {
        tokio::select! {
            _ = &mut *shutdown_rx => {
                let _ = write.close().await;
                return Ok(FeishuWsExit::Shutdown);
            }
            _ = ping.tick() => {
                let frame = new_ping_frame(service_id);
                write.send(WsMessage::Binary(encode_ws_frame(&frame)?)).await?;
            }
            message = read.next() => {
                let Some(message) = message else {
                    return Ok(FeishuWsExit::Reconnect);
                };
                match message? {
                    WsMessage::Binary(bytes) => {
                        if let Some(response) = handle_feishu_ws_frame(Arc::clone(&platform), &bytes).await? {
                            write.send(WsMessage::Binary(response)).await?;
                        }
                    }
                    WsMessage::Ping(bytes) => {
                        write.send(WsMessage::Pong(bytes)).await?;
                    }
                    WsMessage::Close(_) => return Ok(FeishuWsExit::Reconnect),
                    _ => {}
                }
            }
        }
    }
}

async fn handle_feishu_ws_frame(
    platform: Arc<FeishuPlatform>,
    bytes: &[u8],
) -> Result<Option<Vec<u8>>> {
    let mut frame = FeishuWsFrame::decode(bytes)?;
    let frame_type = frame.method;
    let message_type = header_value(&frame.headers, HEADER_TYPE).unwrap_or_default();
    if frame_type == FRAME_TYPE_CONTROL {
        if message_type == MESSAGE_TYPE_PONG {
            return Ok(None);
        }
        return Ok(None);
    }
    if frame_type != FRAME_TYPE_DATA || message_type != MESSAGE_TYPE_EVENT {
        return Ok(None);
    }

    let started = Instant::now();
    let payload = frame.payload.clone().unwrap_or_default();
    let status = match serde_json::from_slice::<Value>(&payload) {
        Ok(value) => match dispatch_feishu_payload(platform, &value).await {
            Ok(_) => StatusCode::OK.as_u16(),
            Err(err) => {
                tracing::warn!(error = %err, "feishu websocket event parse failed");
                StatusCode::INTERNAL_SERVER_ERROR.as_u16()
            }
        },
        Err(err) => {
            tracing::warn!(error = %err, "feishu websocket payload is not json");
            StatusCode::INTERNAL_SERVER_ERROR.as_u16()
        }
    };

    upsert_header(
        &mut frame.headers,
        HEADER_BIZ_RT,
        started.elapsed().as_millis().to_string(),
    );
    frame.payload = Some(serde_json::to_vec(&json!({
        "code": status,
        "headers": {},
        "data": null
    }))?);
    Ok(Some(encode_ws_frame(&frame)?))
}

async fn dispatch_feishu_payload(
    platform: Arc<FeishuPlatform>,
    payload: &Value,
) -> Result<Option<bool>> {
    match feishu_message_from_payload(&platform, payload).await? {
        Some(message) => {
            let handler = platform.handler.lock().await.clone();
            if let Some(handler) = handler {
                let platform_dyn = Arc::clone(&platform) as Arc<dyn Platform>;
                tokio::spawn(async move {
                    handler(platform_dyn, message).await;
                });
            }
            Ok(Some(true))
        }
        None => Ok(None),
    }
}

fn new_ping_frame(service_id: i32) -> FeishuWsFrame {
    FeishuWsFrame {
        seq_id: 0,
        log_id: 0,
        service: service_id,
        method: FRAME_TYPE_CONTROL,
        headers: vec![FeishuWsHeader {
            key: HEADER_TYPE.to_string(),
            value: MESSAGE_TYPE_PING.to_string(),
        }],
        payload_encoding: None,
        payload_type: None,
        payload: None,
        log_id_new: None,
    }
}

fn encode_ws_frame(frame: &FeishuWsFrame) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    frame.encode(&mut buf)?;
    Ok(buf)
}

fn header_value(headers: &[FeishuWsHeader], key: &str) -> Option<String> {
    headers
        .iter()
        .find(|header| header.key == key)
        .map(|header| header.value.clone())
}

fn upsert_header(headers: &mut Vec<FeishuWsHeader>, key: &str, value: String) {
    if let Some(header) = headers.iter_mut().find(|header| header.key == key) {
        header.value = value;
        return;
    }
    headers.push(FeishuWsHeader {
        key: key.to_string(),
        value,
    });
}

fn query_param(input: &str, name: &str) -> Option<String> {
    let query = input.split_once('?')?.1;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key == name {
            return Some(value.to_string());
        }
    }
    None
}

fn mask_ws_url(input: &str) -> String {
    let Some((base, query)) = input.split_once('?') else {
        return input.to_string();
    };
    let masked = query
        .split('&')
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            if matches!(key, "device_id" | "access_key" | "ticket" | "conn_id") {
                format!("{key}=***")
            } else {
                format!("{key}={value}")
            }
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{masked}")
}

async fn feishu_message_from_payload(
    platform: &FeishuPlatform,
    payload: &Value,
) -> Result<Option<Message>> {
    let event = payload.get("event").unwrap_or(payload);
    let Some(message) = event.get("message") else {
        let event_type = payload
            .get("header")
            .and_then(|header| header.get("event_type"))
            .and_then(Value::as_str)
            .or_else(|| event.get("type").and_then(Value::as_str))
            .unwrap_or("unknown");
        tracing::debug!(event_type = %event_type, "feishu non-message event ignored");
        return Ok(None);
    };
    let msg_type = message
        .get("message_type")
        .and_then(Value::as_str)
        .unwrap_or("text");
    if msg_type != "text" {
        return Ok(None);
    }
    let content_raw = message
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing message.content"))?;
    let content_value: Value = serde_json::from_str(content_raw)?;
    let content = content_value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Ok(None);
    }

    let chat_id = message
        .get("chat_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing message.chat_id"))?
        .to_string();
    let message_id = message
        .get("message_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let chat_type = message
        .get("chat_type")
        .and_then(Value::as_str)
        .unwrap_or("p2p");
    let sender = event.get("sender").unwrap_or(&Value::Null);
    let sender_id = sender.get("sender_id").unwrap_or(sender);
    let user_id = sender_id
        .get("open_id")
        .or_else(|| sender_id.get("user_id"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let scope = if chat_type == "group" { "g" } else { "p" };
    let session_key = if platform.config.share_session_in_channel {
        format!("{}:{}:{}", platform.config.name, scope, chat_id)
    } else {
        format!("{}:{}:{}:{}", platform.config.name, scope, chat_id, user_id)
    };
    let reply_ctx = FeishuReplyContext {
        message_id,
        chat_id,
    };
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.config.name.clone()),
        message_id: message
            .get("message_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        message_type: crate::core::MessageType::Text,
        user_id,
        user_name: None,
        content,
        attachments: Vec::new(),
        images: Vec::new(),
        files: Vec::new(),
        reply_ctx: ReplyContext {
            value: serde_json::to_string(&reply_ctx)?,
        },
        created_at: std::time::SystemTime::now(),
    }))
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

impl TryFrom<toml::value::Table> for FeishuPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let app_id = string_option(&opts, "app_id").unwrap_or_default();
        let app_secret = string_option(&opts, "app_secret").unwrap_or_default();
        let dry_run = bool_option(&opts, "dry_run").unwrap_or(false);
        if !dry_run && (app_id.is_empty() || app_secret.is_empty()) {
            return Err(anyhow!(
                "feishu requires app_id and app_secret unless dry_run = true"
            ));
        }
        let connection_mode = string_option(&opts, "connection_mode")
            .or_else(|| string_option(&opts, "mode"))
            .unwrap_or_else(|| "websocket".to_string())
            .trim()
            .to_ascii_lowercase();
        if connection_mode != "websocket" && connection_mode != "webhook" {
            return Err(anyhow!(
                "feishu connection_mode must be websocket or webhook, got `{connection_mode}`"
            ));
        }
        let listen =
            string_option(&opts, "listen").unwrap_or_else(|| "127.0.0.1:18200".to_string());
        if listen.parse::<SocketAddr>().is_err() {
            return Err(anyhow!(
                "feishu listen must be a socket address, got `{listen}`"
            ));
        }
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "feishu".to_string()),
            app_id,
            app_secret,
            api_base: string_option(&opts, "api_base")
                .unwrap_or_else(|| "https://open.feishu.cn".to_string()),
            connection_mode,
            listen,
            callback_path: string_option(&opts, "callback_path")
                .unwrap_or_else(|| "/feishu/webhook".to_string()),
            verification_token: string_option(&opts, "verification_token"),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run,
        })
    }
}
