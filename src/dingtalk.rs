use crate::core::{Message, MessageHandler, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex};

#[derive(Debug, Clone, Default)]
pub struct DingTalkPlatformConfig {
    pub name: String,
    pub client_id: String,
    pub client_secret: String,
    pub robot_code: String,
    pub listen: String,
    pub callback_path: String,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DingTalkOutboundRecord {
    pub session_webhook: Option<String>,
    pub conversation_id: String,
    pub sender_staff_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DingTalkReplyContext {
    session_webhook: Option<String>,
    conversation_id: String,
    sender_staff_id: String,
}

pub struct DingTalkPlatform {
    config: DingTalkPlatformConfig,
    handler: Arc<Mutex<Option<MessageHandler>>>,
    outbox: Arc<Mutex<Vec<DingTalkOutboundRecord>>>,
    out_tx: mpsc::UnboundedSender<DingTalkOutboundRecord>,
    out_rx: Arc<Mutex<mpsc::UnboundedReceiver<DingTalkOutboundRecord>>>,
    local_addr: Arc<Mutex<Option<SocketAddr>>>,
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    client: reqwest::Client,
}

impl DingTalkPlatform {
    pub fn new(config: DingTalkPlatformConfig) -> Arc<Self> {
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            config: DingTalkPlatformConfig {
                name: if config.name.trim().is_empty() {
                    "dingtalk".to_string()
                } else {
                    config.name
                },
                robot_code: if config.robot_code.trim().is_empty() {
                    config.client_id.clone()
                } else {
                    config.robot_code
                },
                listen: if config.listen.trim().is_empty() {
                    "127.0.0.1:18300".to_string()
                } else {
                    config.listen
                },
                callback_path: normalize_path(if config.callback_path.trim().is_empty() {
                    "/dingtalk/webhook".to_string()
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

    pub async fn wait_for_outbound(&self) -> Option<DingTalkOutboundRecord> {
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

#[async_trait]
impl Platform for DingTalkPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let listener = TcpListener::bind(&self.config.listen).await?;
        let addr = listener.local_addr()?;
        *self.local_addr.lock().await = Some(addr);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);
        let app = Router::new()
            .route(&self.config.callback_path, post(dingtalk_webhook))
            .route("/dingtalk/healthz", get(dingtalk_health))
            .with_state(Arc::new(self.clone_for_server()));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
        tracing::info!(addr = %addr, path = %self.config.callback_path, "dingtalk platform started");
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: DingTalkReplyContext = serde_json::from_str(&reply_ctx.value)?;
        let record = DingTalkOutboundRecord {
            session_webhook: ctx.session_webhook.clone(),
            conversation_id: ctx.conversation_id.clone(),
            sender_staff_id: ctx.sender_staff_id.clone(),
            content: content.clone(),
        };
        self.outbox.lock().await.push(record.clone());
        let _ = self.out_tx.send(record);

        if self.config.dry_run {
            return Ok(());
        }
        let Some(session_webhook) = ctx.session_webhook else {
            return Err(anyhow!(
                "dingtalk reply requires sessionWebhook in callback payload"
            ));
        };
        self.client
            .post(session_webhook)
            .json(&json!({
                "msgtype": "markdown",
                "markdown": {
                    "title": "reply",
                    "text": preprocess_markdown(&content)
                }
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

async fn dingtalk_health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

async fn dingtalk_webhook(
    State(platform): State<Arc<DingTalkPlatform>>,
    Json(payload): Json<Value>,
) -> Response {
    match dingtalk_message_from_payload(&platform, &payload) {
        Ok(Some(message)) => {
            let handler = platform.handler.lock().await.clone();
            if let Some(handler) = handler {
                let platform_dyn = Arc::clone(&platform) as Arc<dyn Platform>;
                tokio::spawn(async move {
                    handler(platform_dyn, message).await;
                });
            }
            Json(json!({ "success": true })).into_response()
        }
        Ok(None) => Json(json!({ "success": true, "ignored": true })).into_response(),
        Err(err) => {
            tracing::warn!(error = %err, "dingtalk webhook parse failed");
            (StatusCode::BAD_REQUEST, err.to_string()).into_response()
        }
    }
}

fn dingtalk_message_from_payload(
    platform: &DingTalkPlatform,
    payload: &Value,
) -> Result<Option<Message>> {
    let msg_type = payload
        .get("msgtype")
        .or_else(|| payload.get("msgType"))
        .and_then(Value::as_str)
        .unwrap_or("text");
    if msg_type != "text" && msg_type != "richText" {
        return Ok(None);
    }
    let content = if msg_type == "richText" {
        extract_rich_text(payload.get("content").unwrap_or(&Value::Null))
    } else {
        payload
            .pointer("/text/content")
            .or_else(|| payload.pointer("/text/text"))
            .or_else(|| payload.get("content"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    if content.is_empty() {
        return Ok(None);
    }
    let conversation_id = payload
        .get("conversationId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing conversationId"))?
        .to_string();
    let sender_staff_id = payload
        .get("senderStaffId")
        .or_else(|| payload.get("senderId"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let conv_type = if payload
        .get("conversationType")
        .and_then(Value::as_str)
        .unwrap_or("1")
        == "2"
    {
        "g"
    } else {
        "d"
    };
    let session_key = if platform.config.share_session_in_channel {
        format!("{}:{}:{}", platform.config.name, conv_type, conversation_id)
    } else {
        format!(
            "{}:{}:{}:{}",
            platform.config.name, conv_type, conversation_id, sender_staff_id
        )
    };
    let reply_ctx = DingTalkReplyContext {
        session_webhook: payload
            .get("sessionWebhook")
            .and_then(Value::as_str)
            .map(str::to_string),
        conversation_id: conversation_id.clone(),
        sender_staff_id: sender_staff_id.clone(),
    };
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.config.name.clone()),
        message_id: payload
            .get("msgId")
            .or_else(|| payload.get("msgid"))
            .and_then(Value::as_str)
            .map(str::to_string),
        message_type: crate::core::MessageType::Text,
        user_id: sender_staff_id,
        user_name: payload
            .get("senderNick")
            .and_then(Value::as_str)
            .map(str::to_string),
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

fn extract_rich_text(value: &Value) -> String {
    value
        .get("richText")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn preprocess_markdown(content: &str) -> String {
    content.replace("\n\n", "\n\n")
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

impl TryFrom<toml::value::Table> for DingTalkPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let client_id = string_option(&opts, "client_id").unwrap_or_default();
        let client_secret = string_option(&opts, "client_secret").unwrap_or_default();
        let dry_run = bool_option(&opts, "dry_run").unwrap_or(false);
        if !dry_run && (client_id.is_empty() || client_secret.is_empty()) {
            return Err(anyhow!(
                "dingtalk requires client_id and client_secret unless dry_run = true"
            ));
        }
        let listen =
            string_option(&opts, "listen").unwrap_or_else(|| "127.0.0.1:18300".to_string());
        if listen.parse::<SocketAddr>().is_err() {
            return Err(anyhow!(
                "dingtalk listen must be a socket address, got `{listen}`"
            ));
        }
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "dingtalk".to_string()),
            client_id: client_id.clone(),
            client_secret,
            robot_code: string_option(&opts, "robot_code").unwrap_or(client_id),
            listen,
            callback_path: string_option(&opts, "callback_path")
                .unwrap_or_else(|| "/dingtalk/webhook".to_string()),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run,
        })
    }
}
