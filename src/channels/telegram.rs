use crate::core::{Message, MessageHandler, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex};

#[derive(Debug, Clone)]
pub struct TelegramPlatformConfig {
    pub name: String,
    pub token: String,
    pub api_base: String,
    pub poll_timeout_secs: u64,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

impl Default for TelegramPlatformConfig {
    fn default() -> Self {
        Self {
            name: "telegram".to_string(),
            token: String::new(),
            api_base: "https://api.telegram.org".to_string(),
            poll_timeout_secs: 30,
            share_session_in_channel: false,
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramOutboundRecord {
    pub chat_id: String,
    pub message_id: Option<i64>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TelegramReplyContext {
    chat_id: String,
    message_id: Option<i64>,
}

pub struct TelegramPlatform {
    config: TelegramPlatformConfig,
    handler: Arc<Mutex<Option<MessageHandler>>>,
    outbox: Arc<Mutex<Vec<TelegramOutboundRecord>>>,
    out_tx: mpsc::UnboundedSender<TelegramOutboundRecord>,
    out_rx: Arc<Mutex<mpsc::UnboundedReceiver<TelegramOutboundRecord>>>,
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    client: reqwest::Client,
}

impl TelegramPlatform {
    pub fn new(config: TelegramPlatformConfig) -> Arc<Self> {
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            config: TelegramPlatformConfig {
                name: if config.name.trim().is_empty() {
                    "telegram".to_string()
                } else {
                    config.name
                },
                api_base: if config.api_base.trim().is_empty() {
                    "https://api.telegram.org".to_string()
                } else {
                    config.api_base.trim_end_matches('/').to_string()
                },
                poll_timeout_secs: if config.poll_timeout_secs == 0 {
                    30
                } else {
                    config.poll_timeout_secs
                },
                ..config
            },
            handler: Arc::new(Mutex::new(None)),
            outbox: Arc::new(Mutex::new(Vec::new())),
            out_tx,
            out_rx: Arc::new(Mutex::new(out_rx)),
            shutdown: Arc::new(Mutex::new(None)),
            client: reqwest::Client::new(),
        })
    }

    pub async fn wait_for_outbound(&self) -> Option<TelegramOutboundRecord> {
        self.out_rx.lock().await.recv().await
    }

    fn method_url(&self, method: &str) -> String {
        format!(
            "{}/bot{}/{}",
            self.config.api_base, self.config.token, method
        )
    }

    fn clone_for_task(&self) -> Arc<Self> {
        Arc::new(Self {
            config: self.config.clone(),
            handler: Arc::clone(&self.handler),
            outbox: Arc::clone(&self.outbox),
            out_tx: self.out_tx.clone(),
            out_rx: Arc::clone(&self.out_rx),
            shutdown: Arc::clone(&self.shutdown),
            client: self.client.clone(),
        })
    }
}

#[async_trait]
impl Platform for TelegramPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);
        let platform = self.clone_for_task();
        tokio::spawn(async move {
            run_telegram_polling(platform, shutdown_rx).await;
        });
        tracing::info!("telegram platform started");
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: TelegramReplyContext = serde_json::from_str(&reply_ctx.value)?;
        let record = TelegramOutboundRecord {
            chat_id: ctx.chat_id.clone(),
            message_id: ctx.message_id,
            content: content.clone(),
        };
        self.outbox.lock().await.push(record.clone());
        let _ = self.out_tx.send(record);

        if self.config.dry_run {
            return Ok(());
        }
        let mut body = json!({
            "chat_id": ctx.chat_id,
            "text": content,
            "disable_web_page_preview": true
        });
        if let Some(message_id) = ctx.message_id {
            body["reply_to_message_id"] = json!(message_id);
        }
        self.client
            .post(self.method_url("sendMessage"))
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

async fn run_telegram_polling(
    platform: Arc<TelegramPlatform>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let mut offset: Option<i64> = None;
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => return,
            result = fetch_telegram_updates(&platform, offset) => {
                match result {
                    Ok(updates) => {
                        for update in updates {
                            if let Some(update_id) = update.get("update_id").and_then(Value::as_i64) {
                                offset = Some(update_id + 1);
                            }
                            match telegram_message_from_update(&platform, &update) {
                                Ok(Some(message)) => dispatch(platform.clone(), message).await,
                                Ok(None) => {}
                                Err(err) => tracing::warn!(error = %err, "telegram update parse failed"),
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "telegram getUpdates failed");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

async fn fetch_telegram_updates(
    platform: &TelegramPlatform,
    offset: Option<i64>,
) -> Result<Vec<Value>> {
    let mut body = json!({
        "timeout": platform.config.poll_timeout_secs,
        "allowed_updates": ["message"]
    });
    if let Some(offset) = offset {
        body["offset"] = json!(offset);
    }
    let value: Value = platform
        .client
        .post(platform.method_url("getUpdates"))
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(anyhow!("telegram getUpdates returned ok=false"));
    }
    Ok(value
        .get("result")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

fn telegram_message_from_update(
    platform: &TelegramPlatform,
    update: &Value,
) -> Result<Option<Message>> {
    let message = update
        .get("message")
        .or_else(|| update.get("edited_message"))
        .ok_or_else(|| anyhow!("missing message"))?;
    let content = message
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Ok(None);
    }
    let chat = message
        .get("chat")
        .ok_or_else(|| anyhow!("missing message.chat"))?;
    let chat_id = chat
        .get("id")
        .map(value_to_string)
        .ok_or_else(|| anyhow!("missing chat.id"))?;
    let user = message
        .get("from")
        .ok_or_else(|| anyhow!("missing message.from"))?;
    if user.get("is_bot").and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let user_id = user
        .get("id")
        .map(value_to_string)
        .ok_or_else(|| anyhow!("missing from.id"))?;
    let message_id = message.get("message_id").and_then(Value::as_i64);
    let session_key = if platform.config.share_session_in_channel {
        format!("{}:{}", platform.config.name, chat_id)
    } else {
        format!("{}:{}:{}", platform.config.name, chat_id, user_id)
    };
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.config.name.clone()),
        message_id: message_id.map(|v| v.to_string()),
        message_type: crate::core::MessageType::Text,
        user_id,
        user_name: user
            .get("username")
            .or_else(|| user.get("first_name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        content,
        attachments: Vec::new(),
        images: Vec::new(),
        files: Vec::new(),
        reply_ctx: ReplyContext {
            value: serde_json::to_string(&TelegramReplyContext {
                chat_id,
                message_id,
            })?,
        },
        created_at: std::time::SystemTime::now(),
    }))
}

async fn dispatch(platform: Arc<TelegramPlatform>, message: Message) {
    let handler = platform.handler.lock().await.clone();
    if let Some(handler) = handler {
        let platform_dyn = platform as Arc<dyn Platform>;
        tokio::spawn(async move {
            handler(platform_dyn, message).await;
        });
    }
}

fn value_to_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
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

impl TryFrom<toml::value::Table> for TelegramPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let dry_run = bool_option(&opts, "dry_run").unwrap_or(false);
        let token = string_option(&opts, "token").unwrap_or_default();
        if token.trim().is_empty() && !dry_run {
            return Err(anyhow!("telegram requires token unless dry_run = true"));
        }
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "telegram".to_string()),
            token,
            api_base: string_option(&opts, "api_base")
                .unwrap_or_else(|| "https://api.telegram.org".to_string()),
            poll_timeout_secs: u64_option(&opts, "poll_timeout_secs").unwrap_or(30),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run,
        })
    }
}
