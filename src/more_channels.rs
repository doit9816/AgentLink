use crate::core::{Message, MessageHandler, Platform, ReplyContext};
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::{Digest as Sha1Digest, Sha1};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimpleOutboundRecord {
    pub channel: String,
    pub target: String,
    pub message_id: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SimpleReplyContext {
    channel: String,
    target: String,
    message_id: Option<String>,
    extra: Value,
}

macro_rules! webhook_platform_struct {
    ($config:ident, $platform:ident, $default_name:literal, $default_listen:literal, $default_path:literal) => {
        #[derive(Debug, Clone)]
        pub struct $config {
            pub name: String,
            pub listen: String,
            pub callback_path: String,
            pub secret: Option<String>,
            pub token: Option<String>,
            pub corp_id: Option<String>,
            pub corp_secret: Option<String>,
            pub agent_id: Option<String>,
            pub callback_token: Option<String>,
            pub callback_aes_key: Option<String>,
            pub api_base: String,
            pub share_session_in_channel: bool,
            pub dry_run: bool,
        }

        impl Default for $config {
            fn default() -> Self {
                Self {
                    name: $default_name.to_string(),
                    listen: $default_listen.to_string(),
                    callback_path: $default_path.to_string(),
                    secret: None,
                    token: None,
                    corp_id: None,
                    corp_secret: None,
                    agent_id: None,
                    callback_token: None,
                    callback_aes_key: None,
                    api_base: String::new(),
                    share_session_in_channel: false,
                    dry_run: false,
                }
            }
        }

        pub struct $platform {
            config: $config,
            handler: Arc<Mutex<Option<MessageHandler>>>,
            outbox: Arc<Mutex<Vec<SimpleOutboundRecord>>>,
            out_tx: mpsc::UnboundedSender<SimpleOutboundRecord>,
            out_rx: Arc<Mutex<mpsc::UnboundedReceiver<SimpleOutboundRecord>>>,
            local_addr: Arc<Mutex<Option<SocketAddr>>>,
            shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
            client: reqwest::Client,
        }

        impl $platform {
            pub fn new(config: $config) -> Arc<Self> {
                let (out_tx, out_rx) = mpsc::unbounded_channel();
                Arc::new(Self {
                    config: $config {
                        name: if config.name.trim().is_empty() {
                            $default_name.to_string()
                        } else {
                            config.name
                        },
                        listen: if config.listen.trim().is_empty() {
                            $default_listen.to_string()
                        } else {
                            config.listen
                        },
                        callback_path: normalize_path(if config.callback_path.trim().is_empty() {
                            $default_path.to_string()
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
                })
            }

            pub async fn local_addr(&self) -> Option<SocketAddr> {
                *self.local_addr.lock().await
            }

            pub async fn wait_for_outbound(&self) -> Option<SimpleOutboundRecord> {
                self.out_rx.lock().await.recv().await
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
                }
            }
        }
    };
}

webhook_platform_struct!(
    LinePlatformConfig,
    LinePlatform,
    "line",
    "127.0.0.1:18400",
    "/line/webhook"
);
webhook_platform_struct!(
    WeComPlatformConfig,
    WeComPlatform,
    "wecom",
    "127.0.0.1:18500",
    "/wecom/callback"
);

#[async_trait]
impl Platform for LinePlatform {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let listener = TcpListener::bind(&self.config.listen).await?;
        *self.local_addr.lock().await = Some(listener.local_addr()?);
        let (tx, rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(tx);
        let path = self.config.callback_path.clone();
        let app = Router::new()
            .route(&path, post(line_webhook))
            .with_state(Arc::new(self.clone_for_server()));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: SimpleReplyContext = serde_json::from_str(&reply_ctx.value)?;
        emit_outbound(self, &ctx, &content).await;
        if self.config.dry_run {
            return Ok(());
        }
        let token = self
            .config
            .token
            .as_ref()
            .ok_or_else(|| anyhow!("line channel_token is required"))?;
        let base = if self.config.api_base.trim().is_empty() {
            "https://api.line.me".to_string()
        } else {
            self.config.api_base.trim_end_matches('/').to_string()
        };
        self.client
            .post(format!("{base}/v2/bot/message/push"))
            .bearer_auth(token)
            .json(&json!({
                "to": ctx.target,
                "messages": [{ "type": "text", "text": content }]
            }))
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

async fn line_webhook(
    State(platform): State<Arc<LinePlatform>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(secret) = &platform.config.secret {
        if !platform.config.dry_run && !verify_line_signature(secret, &headers, &body) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    let payload: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(err) => return (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    };
    for event in payload
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        if let Ok(Some(message)) = line_message_from_event(&platform, &event) {
            dispatch_webhook(
                Arc::clone(&platform) as Arc<dyn Platform>,
                &platform.handler,
                message,
            )
            .await;
        }
    }
    Json(json!({ "ok": true })).into_response()
}

fn line_message_from_event(platform: &LinePlatform, event: &Value) -> Result<Option<Message>> {
    if event.get("type").and_then(Value::as_str) != Some("message")
        || event.pointer("/message/type").and_then(Value::as_str) != Some("text")
    {
        return Ok(None);
    }
    let content = event
        .pointer("/message/text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Ok(None);
    }
    let source = event
        .get("source")
        .ok_or_else(|| anyhow!("missing source"))?;
    let user_id = source
        .get("userId")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let target = source
        .get("groupId")
        .or_else(|| source.get("roomId"))
        .or_else(|| source.get("userId"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing line target"))?
        .to_string();
    let session_key = if platform.config.share_session_in_channel {
        format!("{}:{}", platform.config.name, target)
    } else {
        format!("{}:{}:{}", platform.config.name, target, user_id)
    };
    simple_message(
        &platform.config.name,
        session_key,
        user_id,
        event.pointer("/message/id").and_then(Value::as_str),
        content,
        SimpleReplyContext {
            channel: platform.config.name.clone(),
            target,
            message_id: event
                .pointer("/message/id")
                .and_then(Value::as_str)
                .map(str::to_string),
            extra: Value::Null,
        },
    )
}

#[async_trait]
impl Platform for WeComPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let listener = TcpListener::bind(&self.config.listen).await?;
        *self.local_addr.lock().await = Some(listener.local_addr()?);
        let (tx, rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(tx);
        let path = self.config.callback_path.clone();
        let app = Router::new()
            .route(&path, get(wecom_verify).post(wecom_webhook))
            .with_state(Arc::new(self.clone_for_server()));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: SimpleReplyContext = serde_json::from_str(&reply_ctx.value)?;
        emit_outbound(self, &ctx, &content).await;
        if self.config.dry_run {
            return Ok(());
        }
        let token = self.wecom_access_token().await?;
        let agent_id = ctx
            .extra
            .get("agent_id")
            .and_then(Value::as_str)
            .or_else(|| self.config.token.as_deref())
            .unwrap_or("0");
        let base = if self.config.api_base.trim().is_empty() {
            "https://qyapi.weixin.qq.com".to_string()
        } else {
            self.config.api_base.trim_end_matches('/').to_string()
        };
        self.client
            .post(format!("{base}/cgi-bin/message/send?access_token={token}"))
            .json(&json!({
                "touser": ctx.target,
                "msgtype": "text",
                "agentid": agent_id,
                "text": { "content": content }
            }))
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

impl WeComPlatform {
    async fn wecom_access_token(&self) -> Result<String> {
        let corp_id = self
            .config
            .corp_id
            .as_ref()
            .or(self.config.secret.as_ref())
            .ok_or_else(|| anyhow!("wecom corp_id is required"))?;
        let corp_secret = self
            .config
            .corp_secret
            .as_ref()
            .or(self.config.token.as_ref())
            .ok_or_else(|| anyhow!("wecom corp_secret is required"))?;
        let base = if self.config.api_base.trim().is_empty() {
            "https://qyapi.weixin.qq.com".to_string()
        } else {
            self.config.api_base.trim_end_matches('/').to_string()
        };
        let value: Value = self
            .client
            .get(format!(
                "{base}/cgi-bin/gettoken?corpid={corp_id}&corpsecret={corp_secret}"
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        value
            .get("access_token")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| anyhow!("wecom gettoken response missing access_token"))
    }
}

async fn wecom_verify(
    State(platform): State<Arc<WeComPlatform>>,
    Query(params): Query<BTreeMap<String, String>>,
) -> Response {
    let echostr = params.get("echostr").cloned().unwrap_or_default();
    if platform.config.callback_aes_key.is_some() {
        let sig = params
            .get("msg_signature")
            .map(String::as_str)
            .unwrap_or("");
        let ts = params.get("timestamp").map(String::as_str).unwrap_or("");
        let nonce = params.get("nonce").map(String::as_str).unwrap_or("");
        if !wecom_verify_signature(&platform, sig, ts, nonce, &echostr) {
            return StatusCode::FORBIDDEN.into_response();
        }
        match wecom_decrypt(&platform, &echostr) {
            Ok(plain) => plain.into_response(),
            Err(err) => (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
        }
    } else {
        echostr.into_response()
    }
}

async fn wecom_webhook(
    State(platform): State<Arc<WeComPlatform>>,
    Query(params): Query<BTreeMap<String, String>>,
    body: Bytes,
) -> Response {
    let text = String::from_utf8_lossy(&body).to_string();
    let payload = serde_json::from_slice::<Value>(&body).ok();
    let plain = if payload.is_none() && xml_tag(&text, "Encrypt").is_some() {
        let encrypt = xml_tag(&text, "Encrypt").unwrap_or_default();
        if platform.config.callback_aes_key.is_some() {
            let sig = params
                .get("msg_signature")
                .map(String::as_str)
                .unwrap_or("");
            let ts = params.get("timestamp").map(String::as_str).unwrap_or("");
            let nonce = params.get("nonce").map(String::as_str).unwrap_or("");
            if !wecom_verify_signature(&platform, sig, ts, nonce, &encrypt) {
                return StatusCode::FORBIDDEN.into_response();
            }
        }
        match wecom_decrypt(&platform, &encrypt) {
            Ok(plain) => plain,
            Err(err) => return (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
        }
    } else {
        text
    };
    match wecom_message_from_payload(&platform, payload.as_ref(), &plain) {
        Ok(Some(message)) => {
            dispatch_webhook(
                Arc::clone(&platform) as Arc<dyn Platform>,
                &platform.handler,
                message,
            )
            .await;
            "success".into_response()
        }
        Ok(None) => "success".into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    }
}

fn wecom_message_from_payload(
    platform: &WeComPlatform,
    payload: Option<&Value>,
    raw: &str,
) -> Result<Option<Message>> {
    let (from, content, msg_id, agent_id) = if let Some(v) = payload {
        (
            v.get("FromUserName")
                .or_else(|| v.get("from_user"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
            v.get("Content")
                .or_else(|| v.get("content"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
            v.get("MsgId")
                .or_else(|| v.get("msg_id"))
                .map(value_to_string),
            v.get("AgentID")
                .or_else(|| v.get("agent_id"))
                .map(value_to_string),
        )
    } else {
        (
            xml_tag(raw, "FromUserName").unwrap_or_else(|| "unknown".to_string()),
            xml_tag(raw, "Content").unwrap_or_default(),
            xml_tag(raw, "MsgId"),
            xml_tag(raw, "AgentID"),
        )
    };
    if content.trim().is_empty() {
        return Ok(None);
    }
    simple_message(
        &platform.config.name,
        format!("{}:{}", platform.config.name, from),
        from.clone(),
        msg_id.clone().as_deref(),
        content,
        SimpleReplyContext {
            channel: platform.config.name.clone(),
            target: from,
            message_id: msg_id,
            extra: json!({ "agent_id": agent_id.or_else(|| platform.config.agent_id.clone()).unwrap_or_default() }),
        },
    )
}

fn wecom_verify_signature(
    platform: &WeComPlatform,
    expected: &str,
    timestamp: &str,
    nonce: &str,
    encrypt: &str,
) -> bool {
    let token = platform
        .config
        .callback_token
        .as_deref()
        .or(platform.config.token.as_deref())
        .unwrap_or("");
    if expected.is_empty() || token.is_empty() {
        return false;
    }
    let mut parts = [
        token.to_string(),
        timestamp.to_string(),
        nonce.to_string(),
        encrypt.to_string(),
    ];
    parts.sort();
    let mut hasher = Sha1::new();
    hasher.update(parts.join("").as_bytes());
    let actual = format!("{:x}", hasher.finalize());
    actual == expected
}

fn wecom_decrypt(platform: &WeComPlatform, cipher_base64: &str) -> Result<String> {
    let key = platform
        .config
        .callback_aes_key
        .as_deref()
        .ok_or_else(|| anyhow!("wecom callback_aes_key is required for encrypted XML"))?;
    let key = decode_wecom_aes_key(key)?;
    let cipher_data = base64::engine::general_purpose::STANDARD.decode(cipher_base64)?;
    if cipher_data.len() % 16 != 0 {
        return Err(anyhow!("invalid wecom ciphertext length"));
    }
    let iv = &key[..16];
    let mut buf = cipher_data;
    let plain = cbc::Decryptor::<aes::Aes256>::new_from_slices(&key, iv)?
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|_| anyhow!("wecom pkcs7 unpad failed"))?;
    if plain.len() < 20 {
        return Err(anyhow!("wecom decrypted data too short"));
    }
    let msg_len = u32::from_be_bytes([plain[16], plain[17], plain[18], plain[19]]) as usize;
    if 20 + msg_len > plain.len() {
        return Err(anyhow!("wecom decrypted message length is invalid"));
    }
    let msg = std::str::from_utf8(&plain[20..20 + msg_len])?.to_string();
    let corp_id = std::str::from_utf8(&plain[20 + msg_len..]).unwrap_or("");
    if let Some(expected) = &platform.config.corp_id {
        if !expected.is_empty() && corp_id != expected {
            return Err(anyhow!("wecom corp_id mismatch"));
        }
    }
    Ok(msg)
}

fn decode_wecom_aes_key(value: &str) -> Result<Vec<u8>> {
    if value.len() != 43 {
        return Err(anyhow!(
            "wecom callback_aes_key must be 43 characters, got {}",
            value.len()
        ));
    }
    Ok(base64::engine::general_purpose::STANDARD.decode(format!("{value}="))?)
}

#[derive(Debug, Clone)]
pub struct PollPlatformConfig {
    pub name: String,
    pub app_id: Option<String>,
    pub app_secret: Option<String>,
    pub token_endpoint: Option<String>,
    pub token: String,
    pub api_base: String,
    pub ws_endpoint: Option<String>,
    pub poll_timeout_secs: u64,
    pub webhook_url: Option<String>,
    pub webhook_listen: String,
    pub webhook_path: String,
    pub webhook_secret: Option<String>,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

impl Default for PollPlatformConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            app_id: None,
            app_secret: None,
            token_endpoint: None,
            token: String::new(),
            api_base: String::new(),
            ws_endpoint: None,
            poll_timeout_secs: 30,
            webhook_url: None,
            webhook_listen: "127.0.0.1:18600".to_string(),
            webhook_path: "/max/webhook".to_string(),
            webhook_secret: None,
            share_session_in_channel: false,
            dry_run: false,
        }
    }
}

macro_rules! polling_platform {
    ($platform:ident) => {
        pub struct $platform {
            config: PollPlatformConfig,
            handler: Arc<Mutex<Option<MessageHandler>>>,
            outbox: Arc<Mutex<Vec<SimpleOutboundRecord>>>,
            out_tx: mpsc::UnboundedSender<SimpleOutboundRecord>,
            out_rx: Arc<Mutex<mpsc::UnboundedReceiver<SimpleOutboundRecord>>>,
            shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
            local_addr: Arc<Mutex<Option<SocketAddr>>>,
            client: reqwest::Client,
        }

        impl $platform {
            pub fn new(config: PollPlatformConfig) -> Arc<Self> {
                let (out_tx, out_rx) = mpsc::unbounded_channel();
                Arc::new(Self {
                    config,
                    handler: Arc::new(Mutex::new(None)),
                    outbox: Arc::new(Mutex::new(Vec::new())),
                    out_tx,
                    out_rx: Arc::new(Mutex::new(out_rx)),
                    shutdown: Arc::new(Mutex::new(None)),
                    local_addr: Arc::new(Mutex::new(None)),
                    client: reqwest::Client::new(),
                })
            }
            pub async fn wait_for_outbound(&self) -> Option<SimpleOutboundRecord> {
                self.out_rx.lock().await.recv().await
            }
            pub async fn local_addr(&self) -> Option<SocketAddr> {
                *self.local_addr.lock().await
            }
            fn clone_for_task(&self) -> Arc<Self> {
                Arc::new(Self {
                    config: self.config.clone(),
                    handler: Arc::clone(&self.handler),
                    outbox: Arc::clone(&self.outbox),
                    out_tx: self.out_tx.clone(),
                    out_rx: Arc::clone(&self.out_rx),
                    shutdown: Arc::clone(&self.shutdown),
                    local_addr: Arc::clone(&self.local_addr),
                    client: self.client.clone(),
                })
            }
        }
    };
}

polling_platform!(MaxPlatform);
polling_platform!(WeixinPlatform);

#[async_trait]
impl Platform for MaxPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }
    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let (tx, rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(tx);
        if self.config.webhook_url.is_some() {
            tokio::spawn(run_max_webhook(self.clone_for_task(), rx));
        } else {
            tokio::spawn(run_max_polling(self.clone_for_task(), rx));
        }
        Ok(())
    }
    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: SimpleReplyContext = serde_json::from_str(&reply_ctx.value)?;
        emit_outbound(self, &ctx, &content).await;
        if !self.config.dry_run {
            self.client
                .post(format!(
                    "{}/messages",
                    self.config.api_base.trim_end_matches('/')
                ))
                .bearer_auth(&self.config.token)
                .json(&json!({ "chat_id": ctx.target, "text": content }))
                .send()
                .await?
                .error_for_status()?;
        }
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

async fn run_max_polling(platform: Arc<MaxPlatform>, mut shutdown_rx: oneshot::Receiver<()>) {
    let mut marker: Option<String> = None;
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => return,
            result = fetch_max_updates(&platform, marker.clone()) => {
                match result {
                    Ok((updates, next)) => {
                        marker = next;
                        for update in updates {
                            if let Ok(Some(message)) = max_message_from_update(&platform, &update) {
                                dispatch_webhook(Arc::clone(&platform) as Arc<dyn Platform>, &platform.handler, message).await;
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "max poll failed");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

async fn run_max_webhook(platform: Arc<MaxPlatform>, shutdown_rx: oneshot::Receiver<()>) {
    if !platform.config.dry_run {
        if let Err(err) = max_subscribe_webhook(&platform).await {
            tracing::warn!(error = %err, "max webhook subscribe failed");
        }
    }
    let listener = match TcpListener::bind(&platform.config.webhook_listen).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::warn!(error = %err, "max webhook bind failed");
            return;
        }
    };
    if let Ok(addr) = listener.local_addr() {
        *platform.local_addr.lock().await = Some(addr);
    }
    let app = Router::new()
        .route(
            &normalize_path(platform.config.webhook_path.clone()),
            post(max_webhook),
        )
        .with_state(platform);
    let _ = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        })
        .await;
}

async fn max_subscribe_webhook(platform: &MaxPlatform) -> Result<()> {
    let mut body = json!({
        "url": platform.config.webhook_url.as_deref().unwrap_or_default(),
        "update_types": ["message_created", "message_callback"]
    });
    if let Some(secret) = &platform.config.webhook_secret {
        body["secret"] = json!(secret);
    }
    platform
        .client
        .post(format!(
            "{}/subscriptions",
            platform.config.api_base.trim_end_matches('/')
        ))
        .bearer_auth(&platform.config.token)
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn max_webhook(
    State(platform): State<Arc<MaxPlatform>>,
    Query(params): Query<BTreeMap<String, String>>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Response {
    if let Some(expected) = &platform.config.webhook_secret {
        let actual = headers
            .get("x-max-bot-api-secret")
            .and_then(|v| v.to_str().ok())
            .or_else(|| params.get("s").map(String::as_str));
        if actual != Some(expected.as_str()) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    match max_message_from_update(&platform, &payload) {
        Ok(Some(message)) => {
            dispatch_webhook(
                Arc::clone(&platform) as Arc<dyn Platform>,
                &platform.handler,
                message,
            )
            .await;
            Json(json!({ "ok": true })).into_response()
        }
        Ok(None) => Json(json!({ "ok": true, "ignored": true })).into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    }
}

async fn fetch_max_updates(
    platform: &MaxPlatform,
    marker: Option<String>,
) -> Result<(Vec<Value>, Option<String>)> {
    let mut req = platform
        .client
        .get(format!(
            "{}/updates",
            platform.config.api_base.trim_end_matches('/')
        ))
        .bearer_auth(&platform.config.token)
        .query(&[
            ("timeout", platform.config.poll_timeout_secs.to_string()),
            ("limit", "20".to_string()),
            ("types", "message_created".to_string()),
        ]);
    if let Some(marker) = marker {
        req = req.query(&[("marker", marker)]);
    }
    let value: Value = req.send().await?.error_for_status()?.json().await?;
    let updates = value
        .get("updates")
        .or_else(|| value.get("result"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let marker = value.get("marker").map(value_to_string);
    Ok((updates, marker))
}

fn max_message_from_update(platform: &MaxPlatform, update: &Value) -> Result<Option<Message>> {
    let msg = update.get("message").unwrap_or(update);
    let text = msg
        .pointer("/body/text")
        .or_else(|| msg.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Ok(None);
    }
    let chat_id = msg
        .pointer("/recipient/chat_id")
        .or_else(|| msg.get("chat_id"))
        .map(value_to_string)
        .ok_or_else(|| anyhow!("max message missing chat_id"))?;
    let user_id = msg
        .pointer("/sender/user_id")
        .or_else(|| msg.get("user_id"))
        .map(value_to_string)
        .unwrap_or_else(|| "unknown".to_string());
    let mid = msg
        .pointer("/body/mid")
        .or_else(|| msg.get("mid"))
        .map(value_to_string);
    let session_key = if platform.config.share_session_in_channel {
        format!("{}:{}", platform.config.name, chat_id)
    } else {
        format!("{}:{}:{}", platform.config.name, chat_id, user_id)
    };
    simple_message(
        &platform.config.name,
        session_key,
        user_id,
        mid.clone().as_deref(),
        text,
        SimpleReplyContext {
            channel: platform.config.name.clone(),
            target: chat_id,
            message_id: mid,
            extra: Value::Null,
        },
    )
}

#[async_trait]
impl Platform for WeixinPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }
    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let (tx, rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(tx);
        tokio::spawn(run_weixin_polling(self.clone_for_task(), rx));
        Ok(())
    }
    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: SimpleReplyContext = serde_json::from_str(&reply_ctx.value)?;
        emit_outbound(self, &ctx, &content).await;
        if !self.config.dry_run {
            self.client
                .post(format!(
                    "{}/ilink/bot/sendmessage",
                    self.config.api_base.trim_end_matches('/')
                ))
                .bearer_auth(&self.config.token)
                .json(&json!({
                    "msg": {
                        "to_user_id": ctx.target,
                        "client_id": ctx.extra.get("client_id").and_then(Value::as_str).unwrap_or(""),
                        "message_type": 2,
                        "message_state": 2,
                        "context_token": ctx.extra.get("context_token").and_then(Value::as_str).unwrap_or(""),
                        "item_list": [{ "type": 1, "text_item": { "text": content }}]
                    },
                    "base_info": { "channel_version": "agentlink-weixin/1.0" }
                }))
                .send()
                .await?
                .error_for_status()?;
        }
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

async fn run_weixin_polling(platform: Arc<WeixinPlatform>, mut shutdown_rx: oneshot::Receiver<()>) {
    let mut buf = String::new();
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => return,
            result = fetch_weixin_updates(&platform, &buf) => {
                match result {
                    Ok((updates, next)) => {
                        if let Some(next) = next { buf = next; }
                        for item in updates {
                            if let Ok(Some(message)) = weixin_message_from_item(&platform, &item) {
                                dispatch_webhook(Arc::clone(&platform) as Arc<dyn Platform>, &platform.handler, message).await;
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "weixin poll failed");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

async fn fetch_weixin_updates(
    platform: &WeixinPlatform,
    buf: &str,
) -> Result<(Vec<Value>, Option<String>)> {
    let value: Value = platform
        .client
        .post(format!(
            "{}/ilink/bot/getupdates",
            platform.config.api_base.trim_end_matches('/')
        ))
        .bearer_auth(&platform.config.token)
        .json(&json!({
            "get_updates_buf": buf,
            "base_info": { "channel_version": "agentlink-weixin/1.0" }
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok((
        value
            .get("msgs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        value
            .get("get_updates_buf")
            .and_then(Value::as_str)
            .map(str::to_string),
    ))
}

fn weixin_message_from_item(platform: &WeixinPlatform, item: &Value) -> Result<Option<Message>> {
    if item.get("message_type").and_then(Value::as_i64) == Some(2) {
        return Ok(None);
    }
    let from = item
        .get("from_user_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let text = item
        .get("item_list")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find_map(|v| {
                v.pointer("/text_item/text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        })
        .unwrap_or_default();
    if text.trim().is_empty() {
        return Ok(None);
    }
    let msg_id = item.get("message_id").map(value_to_string);
    simple_message(
        &platform.config.name,
        format!("{}:{}", platform.config.name, from),
        from.clone(),
        msg_id.clone().as_deref(),
        text,
        SimpleReplyContext {
            channel: platform.config.name.clone(),
            target: from,
            message_id: msg_id,
            extra: json!({
                "context_token": item.get("context_token").and_then(Value::as_str).unwrap_or(""),
                "client_id": item.get("client_id").and_then(Value::as_str).unwrap_or("")
            }),
        },
    )
}

#[derive(Debug, Clone)]
pub struct WsPlatformConfig {
    pub name: String,
    pub app_id: String,
    pub app_secret: String,
    pub token_endpoint: String,
    pub ws_endpoint: String,
    pub api_base: String,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

impl Default for WsPlatformConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            app_id: String::new(),
            app_secret: String::new(),
            token_endpoint: String::new(),
            ws_endpoint: String::new(),
            api_base: String::new(),
            share_session_in_channel: false,
            dry_run: false,
        }
    }
}

polling_platform!(QqBotPlatform);
polling_platform!(WeiboPlatform);

// This v1 stores WsPlatformConfig fields inside PollPlatformConfig for reuse:
// token = app token, api_base = HTTP API, name = platform name.
#[async_trait]
impl Platform for QqBotPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }
    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let (tx, rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(tx);
        tokio::spawn(run_simple_ws(self.clone_for_task(), rx, true));
        Ok(())
    }
    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: SimpleReplyContext = serde_json::from_str(&reply_ctx.value)?;
        emit_outbound(self, &ctx, &content).await;
        if !self.config.dry_run {
            let token = resolve_platform_token(&self.config).await?;
            let kind = ctx
                .extra
                .get("target_kind")
                .and_then(Value::as_str)
                .unwrap_or("group");
            let base = self.config.api_base.trim_end_matches('/');
            let endpoint = match kind {
                "user" => format!("{base}/v2/users/{}/messages", ctx.target),
                "channel" => format!("{base}/channels/{}/messages", ctx.target),
                _ => format!("{base}/v2/groups/{}/messages", ctx.target),
            };
            self.client
                .post(endpoint)
                .bearer_auth(token)
                .json(&json!({ "content": content, "msg_id": ctx.message_id }))
                .send()
                .await?
                .error_for_status()?;
        }
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

#[async_trait]
impl Platform for WeiboPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }
    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let (tx, rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(tx);
        tokio::spawn(run_simple_ws(self.clone_for_task(), rx, false));
        Ok(())
    }
    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: SimpleReplyContext = serde_json::from_str(&reply_ctx.value)?;
        emit_outbound(self, &ctx, &content).await;
        if !self.config.dry_run {
            let token = resolve_platform_token(&self.config).await?;
            self.client
                .post(format!(
                    "{}/messages",
                    self.config.api_base.trim_end_matches('/')
                ))
                .bearer_auth(token)
                .json(&json!({ "receiver_id": ctx.target, "text": content }))
                .send()
                .await?
                .error_for_status()?;
        }
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

async fn run_simple_ws(
    platform: Arc<impl SimpleWsPlatform + 'static>,
    mut shutdown_rx: oneshot::Receiver<()>,
    identify: bool,
) {
    loop {
        match run_simple_ws_once(Arc::clone(&platform), &mut shutdown_rx, identify).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => tracing::warn!(error = %err, "websocket channel disconnected"),
        }
        tokio::select! {
            _ = &mut shutdown_rx => return,
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
}

#[async_trait]
trait SimpleWsPlatform: Platform {
    fn cfg(&self) -> &PollPlatformConfig;
    fn handler_ref(&self) -> &Arc<Mutex<Option<MessageHandler>>>;
}

#[async_trait]
impl SimpleWsPlatform for QqBotPlatform {
    fn cfg(&self) -> &PollPlatformConfig {
        &self.config
    }
    fn handler_ref(&self) -> &Arc<Mutex<Option<MessageHandler>>> {
        &self.handler
    }
}

#[async_trait]
impl SimpleWsPlatform for WeiboPlatform {
    fn cfg(&self) -> &PollPlatformConfig {
        &self.config
    }
    fn handler_ref(&self) -> &Arc<Mutex<Option<MessageHandler>>> {
        &self.handler
    }
}

async fn run_simple_ws_once<P: SimpleWsPlatform + 'static>(
    platform: Arc<P>,
    shutdown_rx: &mut oneshot::Receiver<()>,
    identify: bool,
) -> Result<bool> {
    let token = resolve_platform_token(platform.cfg()).await?;
    let url = websocket_url(platform.cfg(), identify, &token);
    let mut request = url.into_client_request()?;
    if !token.is_empty() {
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {token}").parse()?);
    }
    let (stream, _) = connect_async(request).await?;
    let (mut write, mut read) = stream.split();
    if identify {
        write
            .send(WsMessage::Text(
                json!({ "op": 2, "d": { "token": token }}).to_string(),
            ))
            .await?;
    }
    loop {
        tokio::select! {
            _ = &mut *shutdown_rx => {
                let _ = write.close().await;
                return Ok(true);
            }
            message = read.next() => {
                let Some(message) = message else { return Ok(false); };
                match message? {
                    WsMessage::Text(text) => {
                        let value: Value = serde_json::from_str(&text)?;
                        let parsed = if identify {
                            qqbot_message_from_value(platform.cfg(), &value)
                        } else {
                            weibo_message_from_value(platform.cfg(), &value)
                        };
                        if let Ok(Some(message)) = parsed {
                            let handler = platform.handler_ref().lock().await.clone();
                            if let Some(handler) = handler {
                                let dyn_platform = platform.clone() as Arc<dyn Platform>;
                                tokio::spawn(async move {
                                    handler(dyn_platform, message).await;
                                });
                            }
                        }
                    }
                    WsMessage::Ping(bytes) => {
                        write.send(WsMessage::Pong(bytes)).await?;
                    }
                    WsMessage::Close(_) => return Ok(false),
                    _ => {}
                }
            }
        }
    }
}

fn qqbot_message_from_value(cfg: &PollPlatformConfig, value: &Value) -> Result<Option<Message>> {
    let event = value.get("d").unwrap_or(value);
    let text = event
        .get("content")
        .or_else(|| event.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Ok(None);
    }
    let target = event
        .get("group_openid")
        .or_else(|| event.get("channel_id"))
        .or_else(|| event.get("user_openid"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let user_id = event
        .get("member_openid")
        .or_else(|| event.get("user_openid"))
        .or_else(|| event.get("author_id"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let msg_id = event
        .get("id")
        .or_else(|| event.get("msg_id"))
        .map(value_to_string);
    let target_kind = if event.get("group_openid").is_some() {
        "group"
    } else if event.get("channel_id").is_some() {
        "channel"
    } else {
        "user"
    };
    simple_message(
        &cfg.name,
        format!("{}:{}:{}", cfg.name, target, user_id),
        user_id,
        msg_id.clone().as_deref(),
        text,
        SimpleReplyContext {
            channel: cfg.name.clone(),
            target,
            message_id: msg_id,
            extra: json!({ "target_kind": target_kind }),
        },
    )
}

fn weibo_message_from_value(cfg: &PollPlatformConfig, value: &Value) -> Result<Option<Message>> {
    let text = value
        .get("text")
        .or_else(|| value.pointer("/message/text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Ok(None);
    }
    let user_id = value
        .get("from_user_id")
        .or_else(|| value.get("sender_id"))
        .or_else(|| value.pointer("/message/from_user_id"))
        .map(value_to_string)
        .unwrap_or_else(|| "unknown".to_string());
    let msg_id = value
        .get("id")
        .or_else(|| value.get("mid"))
        .map(value_to_string);
    simple_message(
        &cfg.name,
        format!("{}:{}", cfg.name, user_id),
        user_id.clone(),
        msg_id.clone().as_deref(),
        text,
        SimpleReplyContext {
            channel: cfg.name.clone(),
            target: user_id,
            message_id: msg_id,
            extra: Value::Null,
        },
    )
}

async fn resolve_platform_token(cfg: &PollPlatformConfig) -> Result<String> {
    if !cfg.token.trim().is_empty() {
        return Ok(cfg.token.clone());
    }
    match cfg.name.as_str() {
        "qqbot" => fetch_qqbot_access_token(cfg).await,
        "weibo" => fetch_weibo_ws_token(cfg).await,
        _ => Ok(String::new()),
    }
}

async fn fetch_qqbot_access_token(cfg: &PollPlatformConfig) -> Result<String> {
    let app_id = cfg
        .app_id
        .as_deref()
        .ok_or_else(|| anyhow!("qqbot app_id is required when token is empty"))?;
    let app_secret = cfg
        .app_secret
        .as_deref()
        .ok_or_else(|| anyhow!("qqbot app_secret is required when token is empty"))?;
    let endpoint = cfg
        .token_endpoint
        .as_deref()
        .unwrap_or("https://bots.qq.com/app/getAppAccessToken");
    let value: Value = reqwest::Client::new()
        .post(endpoint)
        .json(&json!({
            "appId": app_id,
            "clientSecret": app_secret
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    value
        .get("access_token")
        .or_else(|| value.get("accessToken"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("qqbot token response missing access_token"))
}

async fn fetch_weibo_ws_token(cfg: &PollPlatformConfig) -> Result<String> {
    let app_id = cfg
        .app_id
        .as_deref()
        .ok_or_else(|| anyhow!("weibo app_id is required when token is empty"))?;
    let app_secret = cfg
        .app_secret
        .as_deref()
        .ok_or_else(|| anyhow!("weibo app_secret is required when token is empty"))?;
    let endpoint = cfg
        .token_endpoint
        .as_deref()
        .unwrap_or("https://open-im.api.weibo.com/open/auth/ws_token");
    let value: Value = reqwest::Client::new()
        .post(endpoint)
        .json(&json!({
            "app_id": app_id,
            "app_secret": app_secret
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    value
        .pointer("/data/token")
        .or_else(|| value.get("token"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("weibo token response missing data.token"))
}

fn websocket_url(cfg: &PollPlatformConfig, identify: bool, token: &str) -> String {
    let base = cfg
        .ws_endpoint
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&cfg.api_base);
    if identify {
        return base.to_string();
    }
    let app_id = cfg.app_id.as_deref().unwrap_or("");
    append_query_params(base, &[("app_id", app_id), ("token", token)])
}

fn append_query_params(url: &str, pairs: &[(&str, &str)]) -> String {
    let query = pairs
        .iter()
        .filter(|(_, value)| !value.trim().is_empty())
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    if query.is_empty() {
        return url.to_string();
    }
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}{query}")
}

async fn dispatch_webhook(
    platform: Arc<dyn Platform>,
    handler_ref: &Arc<Mutex<Option<MessageHandler>>>,
    message: Message,
) {
    let handler = handler_ref.lock().await.clone();
    if let Some(handler) = handler {
        tokio::spawn(async move {
            handler(platform, message).await;
        });
    }
}

fn simple_message(
    platform: &str,
    session_key: String,
    user_id: String,
    message_id: Option<&str>,
    content: String,
    reply_ctx: SimpleReplyContext,
) -> Result<Option<Message>> {
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.to_string()),
        message_id: message_id.map(str::to_string),
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

async fn emit_outbound<P>(platform: &P, ctx: &SimpleReplyContext, content: &str)
where
    P: HasOutbox + ?Sized,
{
    let record = SimpleOutboundRecord {
        channel: ctx.channel.clone(),
        target: ctx.target.clone(),
        message_id: ctx.message_id.clone(),
        content: content.to_string(),
    };
    platform.outbox().lock().await.push(record.clone());
    let _ = platform.out_tx().send(record);
}

trait HasOutbox {
    fn outbox(&self) -> &Arc<Mutex<Vec<SimpleOutboundRecord>>>;
    fn out_tx(&self) -> &mpsc::UnboundedSender<SimpleOutboundRecord>;
}

macro_rules! has_outbox {
    ($platform:ty) => {
        impl HasOutbox for $platform {
            fn outbox(&self) -> &Arc<Mutex<Vec<SimpleOutboundRecord>>> {
                &self.outbox
            }
            fn out_tx(&self) -> &mpsc::UnboundedSender<SimpleOutboundRecord> {
                &self.out_tx
            }
        }
    };
}

has_outbox!(LinePlatform);
has_outbox!(WeComPlatform);
has_outbox!(MaxPlatform);
has_outbox!(WeixinPlatform);
has_outbox!(QqBotPlatform);
has_outbox!(WeiboPlatform);

fn verify_line_signature(secret: &str, headers: &HeaderMap, body: &[u8]) -> bool {
    let Some(actual) = headers
        .get("x-line-signature")
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let mut mac = match Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
        Ok(mac) => mac,
        Err(_) => return false,
    };
    mac.update(body);
    let expected = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    actual == expected
}

fn xml_tag(input: &str, tag: &str) -> Option<String> {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    let (_, rest) = input.split_once(&start)?;
    let (value, _) = rest.split_once(&end)?;
    Some(
        value
            .trim()
            .trim_start_matches("<![CDATA[")
            .trim_end_matches("]]>")
            .to_string(),
    )
}

fn value_to_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
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

fn u64_option(opts: &toml::value::Table, key: &str) -> Option<u64> {
    opts.get(key).and_then(|v| v.as_integer()).map(|v| v as u64)
}

macro_rules! webhook_config_try_from {
    ($config:ident, $default_name:literal, $default_base:literal) => {
        impl TryFrom<toml::value::Table> for $config {
            type Error = anyhow::Error;
            fn try_from(opts: toml::value::Table) -> Result<Self> {
                Ok(Self {
                    name: string_option(&opts, "name").unwrap_or_else(|| $default_name.to_string()),
                    listen: string_option(&opts, "listen")
                        .unwrap_or_else(|| Self::default().listen),
                    callback_path: string_option(&opts, "callback_path")
                        .unwrap_or_else(|| Self::default().callback_path),
                    secret: string_option(&opts, "secret")
                        .or_else(|| string_option(&opts, "channel_secret"))
                        .or_else(|| string_option(&opts, "corp_id")),
                    token: string_option(&opts, "token")
                        .or_else(|| string_option(&opts, "channel_token"))
                        .or_else(|| string_option(&opts, "corp_secret"))
                        .or_else(|| string_option(&opts, "agent_id")),
                    corp_id: string_option(&opts, "corp_id"),
                    corp_secret: string_option(&opts, "corp_secret"),
                    agent_id: string_option(&opts, "agent_id"),
                    callback_token: string_option(&opts, "callback_token"),
                    callback_aes_key: string_option(&opts, "callback_aes_key"),
                    api_base: string_option(&opts, "api_base")
                        .or_else(|| string_option(&opts, "api_base_url"))
                        .unwrap_or_else(|| $default_base.to_string()),
                    share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                        .unwrap_or(false),
                    dry_run: bool_option(&opts, "dry_run").unwrap_or(false),
                })
            }
        }
    };
}

webhook_config_try_from!(LinePlatformConfig, "line", "https://api.line.me");
webhook_config_try_from!(WeComPlatformConfig, "wecom", "https://qyapi.weixin.qq.com");

impl TryFrom<toml::value::Table> for PollPlatformConfig {
    type Error = anyhow::Error;
    fn try_from(opts: toml::value::Table) -> Result<Self> {
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "poll".to_string()),
            token: string_option(&opts, "token")
                .or_else(|| string_option(&opts, "app_token"))
                .unwrap_or_default(),
            app_id: string_option(&opts, "app_id"),
            app_secret: string_option(&opts, "app_secret"),
            token_endpoint: string_option(&opts, "token_endpoint"),
            api_base: string_option(&opts, "api_base")
                .or_else(|| string_option(&opts, "base_url"))
                .unwrap_or_default(),
            ws_endpoint: string_option(&opts, "ws_endpoint")
                .or_else(|| string_option(&opts, "gateway_url")),
            poll_timeout_secs: u64_option(&opts, "poll_timeout_secs").unwrap_or(30),
            webhook_url: string_option(&opts, "webhook_url"),
            webhook_listen: string_option(&opts, "webhook_listen")
                .unwrap_or_else(|| "127.0.0.1:18600".to_string()),
            webhook_path: string_option(&opts, "webhook_path")
                .unwrap_or_else(|| "/max/webhook".to_string()),
            webhook_secret: string_option(&opts, "webhook_secret"),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run: bool_option(&opts, "dry_run").unwrap_or(false),
        })
    }
}

pub fn max_config_from_options(opts: toml::value::Table) -> Result<PollPlatformConfig> {
    let mut cfg = PollPlatformConfig::try_from(opts)?;
    if cfg.name == "poll" {
        cfg.name = "max".to_string();
    }
    if cfg.api_base.is_empty() {
        cfg.api_base = "https://platform-api.max.ru".to_string();
    }
    Ok(cfg)
}

pub fn weixin_config_from_options(opts: toml::value::Table) -> Result<PollPlatformConfig> {
    let mut cfg = PollPlatformConfig::try_from(opts)?;
    if cfg.name == "poll" {
        cfg.name = "weixin".to_string();
    }
    if cfg.api_base.is_empty() {
        cfg.api_base = "https://ilinkai.weixin.qq.com".to_string();
    }
    Ok(cfg)
}

pub fn qqbot_config_from_options(opts: toml::value::Table) -> Result<PollPlatformConfig> {
    let mut cfg = PollPlatformConfig::try_from(opts.clone())?;
    if cfg.name == "poll" {
        cfg.name = "qqbot".to_string();
    }
    if cfg.api_base.is_empty() {
        cfg.api_base = "https://api.sgroup.qq.com".to_string();
    }
    if cfg.ws_endpoint.is_none() {
        cfg.ws_endpoint = string_option(&opts, "gateway_url");
    }
    Ok(cfg)
}

pub fn weibo_config_from_options(opts: toml::value::Table) -> Result<PollPlatformConfig> {
    let mut cfg = PollPlatformConfig::try_from(opts.clone())?;
    if cfg.name == "poll" {
        cfg.name = "weibo".to_string();
    }
    if cfg.api_base.is_empty() {
        cfg.api_base = "https://open-im.api.weibo.com".to_string();
    }
    if cfg.ws_endpoint.is_none() {
        cfg.ws_endpoint = string_option(&opts, "ws_endpoint");
    }
    Ok(cfg)
}
