use crate::core::{Message, MessageHandler, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[derive(Debug, Clone)]
pub struct QqPlatformConfig {
    pub name: String,
    pub ws_url: String,
    pub token: Option<String>,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

impl Default for QqPlatformConfig {
    fn default() -> Self {
        Self {
            name: "qq".to_string(),
            ws_url: "ws://127.0.0.1:3001".to_string(),
            token: None,
            share_session_in_channel: false,
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QqOutboundRecord {
    pub message_type: String,
    pub user_id: Option<i64>,
    pub group_id: Option<i64>,
    pub message_id: Option<i64>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QqReplyContext {
    message_type: String,
    user_id: Option<i64>,
    group_id: Option<i64>,
    message_id: Option<i64>,
}

pub struct QqPlatform {
    config: QqPlatformConfig,
    handler: Arc<Mutex<Option<MessageHandler>>>,
    outbox: Arc<Mutex<Vec<QqOutboundRecord>>>,
    out_tx: mpsc::UnboundedSender<QqOutboundRecord>,
    out_rx: Arc<Mutex<mpsc::UnboundedReceiver<QqOutboundRecord>>>,
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    write_tx: Arc<Mutex<Option<mpsc::UnboundedSender<Value>>>>,
    echo_seq: AtomicU64,
}

impl QqPlatform {
    pub fn new(config: QqPlatformConfig) -> Arc<Self> {
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            config: QqPlatformConfig {
                name: if config.name.trim().is_empty() {
                    "qq".to_string()
                } else {
                    config.name
                },
                ws_url: if config.ws_url.trim().is_empty() {
                    "ws://127.0.0.1:3001".to_string()
                } else {
                    config.ws_url
                },
                ..config
            },
            handler: Arc::new(Mutex::new(None)),
            outbox: Arc::new(Mutex::new(Vec::new())),
            out_tx,
            out_rx: Arc::new(Mutex::new(out_rx)),
            shutdown: Arc::new(Mutex::new(None)),
            write_tx: Arc::new(Mutex::new(None)),
            echo_seq: AtomicU64::new(1),
        })
    }

    pub async fn wait_for_outbound(&self) -> Option<QqOutboundRecord> {
        self.out_rx.lock().await.recv().await
    }

    fn clone_for_task(&self) -> Arc<Self> {
        Arc::new(Self {
            config: self.config.clone(),
            handler: Arc::clone(&self.handler),
            outbox: Arc::clone(&self.outbox),
            out_tx: self.out_tx.clone(),
            out_rx: Arc::clone(&self.out_rx),
            shutdown: Arc::clone(&self.shutdown),
            write_tx: Arc::clone(&self.write_tx),
            echo_seq: AtomicU64::new(self.echo_seq.load(Ordering::SeqCst)),
        })
    }
}

#[async_trait]
impl Platform for QqPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);
        let platform = self.clone_for_task();
        tokio::spawn(async move {
            run_qq_websocket(platform, shutdown_rx).await;
        });
        tracing::info!(url = %self.config.ws_url, "qq platform started");
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: QqReplyContext = serde_json::from_str(&reply_ctx.value)?;
        let record = QqOutboundRecord {
            message_type: ctx.message_type.clone(),
            user_id: ctx.user_id,
            group_id: ctx.group_id,
            message_id: ctx.message_id,
            content: content.clone(),
        };
        self.outbox.lock().await.push(record.clone());
        let _ = self.out_tx.send(record);

        if self.config.dry_run {
            return Ok(());
        }
        let (action, params) = if ctx.message_type == "group" {
            let group_id = ctx
                .group_id
                .ok_or_else(|| anyhow!("qq group reply missing group_id"))?;
            (
                "send_group_msg",
                json!({ "group_id": group_id, "message": content }),
            )
        } else {
            let user_id = ctx
                .user_id
                .ok_or_else(|| anyhow!("qq private reply missing user_id"))?;
            (
                "send_private_msg",
                json!({ "user_id": user_id, "message": content }),
            )
        };
        let echo = self.echo_seq.fetch_add(1, Ordering::SeqCst).to_string();
        let payload = json!({
            "action": action,
            "params": params,
            "echo": echo
        });
        let tx = self.write_tx.lock().await.clone();
        let Some(tx) = tx else {
            return Err(anyhow!("qq websocket is not connected"));
        };
        tx.send(payload)?;
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

async fn run_qq_websocket(platform: Arc<QqPlatform>, mut shutdown_rx: oneshot::Receiver<()>) {
    loop {
        match run_qq_websocket_once(Arc::clone(&platform), &mut shutdown_rx).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => tracing::warn!(error = %err, "qq websocket disconnected"),
        }
        tokio::select! {
            _ = &mut shutdown_rx => return,
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
}

async fn run_qq_websocket_once(
    platform: Arc<QqPlatform>,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> Result<bool> {
    let mut request = platform.config.ws_url.clone().into_client_request()?;
    if let Some(token) = &platform.config.token {
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {token}").parse()?);
    }
    let (stream, _) = connect_async(request).await?;
    let (mut write, mut read) = stream.split();
    let (write_tx, mut write_rx) = mpsc::unbounded_channel::<Value>();
    *platform.write_tx.lock().await = Some(write_tx);
    tracing::info!("qq websocket connected");

    loop {
        tokio::select! {
            _ = &mut *shutdown_rx => {
                *platform.write_tx.lock().await = None;
                let _ = write.close().await;
                return Ok(true);
            }
            Some(payload) = write_rx.recv() => {
                write.send(WsMessage::Text(payload.to_string())).await?;
            }
            message = read.next() => {
                let Some(message) = message else {
                    *platform.write_tx.lock().await = None;
                    return Ok(false);
                };
                match message? {
                    WsMessage::Text(text) => {
                        let value: Value = serde_json::from_str(&text)?;
                        if value.get("echo").is_some() {
                            continue;
                        }
                        match qq_message_from_payload(&platform, &value) {
                            Ok(Some(message)) => dispatch(platform.clone(), message).await,
                            Ok(None) => {}
                            Err(err) => tracing::warn!(error = %err, "qq message parse failed"),
                        }
                    }
                    WsMessage::Ping(bytes) => {
                        write.send(WsMessage::Pong(bytes)).await?;
                    }
                    WsMessage::Close(_) => {
                        *platform.write_tx.lock().await = None;
                        return Ok(false);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn qq_message_from_payload(platform: &QqPlatform, payload: &Value) -> Result<Option<Message>> {
    if payload.get("post_type").and_then(Value::as_str) != Some("message") {
        return Ok(None);
    }
    let message_type = payload
        .get("message_type")
        .and_then(Value::as_str)
        .unwrap_or("private")
        .to_string();
    let user_id = payload.get("user_id").and_then(Value::as_i64);
    let group_id = payload.get("group_id").and_then(Value::as_i64);
    let message_id = payload.get("message_id").and_then(Value::as_i64);
    let content = payload
        .get("raw_message")
        .or_else(|| payload.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Ok(None);
    }
    let user_id = user_id.ok_or_else(|| anyhow!("qq message missing user_id"))?;
    let session_key = if message_type == "group" {
        let group_id = group_id.ok_or_else(|| anyhow!("qq group message missing group_id"))?;
        if platform.config.share_session_in_channel {
            format!("{}:g:{}", platform.config.name, group_id)
        } else {
            format!("{}:{}:{}", platform.config.name, group_id, user_id)
        }
    } else {
        format!("{}:{}", platform.config.name, user_id)
    };
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.config.name.clone()),
        message_id: message_id.map(|v| v.to_string()),
        message_type: crate::core::MessageType::Text,
        user_id: user_id.to_string(),
        user_name: payload
            .pointer("/sender/card")
            .or_else(|| payload.pointer("/sender/nickname"))
            .and_then(Value::as_str)
            .map(str::to_string),
        content,
        attachments: Vec::new(),
        images: Vec::new(),
        files: Vec::new(),
        reply_ctx: ReplyContext {
            value: serde_json::to_string(&QqReplyContext {
                message_type,
                user_id: Some(user_id),
                group_id,
                message_id,
            })?,
        },
        created_at: std::time::SystemTime::now(),
    }))
}

async fn dispatch(platform: Arc<QqPlatform>, message: Message) {
    let handler = platform.handler.lock().await.clone();
    if let Some(handler) = handler {
        let platform_dyn = platform as Arc<dyn Platform>;
        tokio::spawn(async move {
            handler(platform_dyn, message).await;
        });
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

impl TryFrom<toml::value::Table> for QqPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "qq".to_string()),
            ws_url: string_option(&opts, "ws_url")
                .unwrap_or_else(|| "ws://127.0.0.1:3001".to_string()),
            token: string_option(&opts, "token"),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run: bool_option(&opts, "dry_run").unwrap_or(false),
        })
    }
}
