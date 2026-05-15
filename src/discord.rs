use crate::core::{Message, MessageHandler, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[derive(Debug, Clone)]
pub struct DiscordPlatformConfig {
    pub name: String,
    pub token: String,
    pub api_base: String,
    pub gateway_url: Option<String>,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

impl Default for DiscordPlatformConfig {
    fn default() -> Self {
        Self {
            name: "discord".to_string(),
            token: String::new(),
            api_base: "https://discord.com/api/v10".to_string(),
            gateway_url: None,
            share_session_in_channel: false,
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscordOutboundRecord {
    pub channel_id: String,
    pub message_id: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiscordReplyContext {
    channel_id: String,
    message_id: Option<String>,
}

pub struct DiscordPlatform {
    config: DiscordPlatformConfig,
    handler: Arc<Mutex<Option<MessageHandler>>>,
    outbox: Arc<Mutex<Vec<DiscordOutboundRecord>>>,
    out_tx: mpsc::UnboundedSender<DiscordOutboundRecord>,
    out_rx: Arc<Mutex<mpsc::UnboundedReceiver<DiscordOutboundRecord>>>,
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    client: reqwest::Client,
}

impl DiscordPlatform {
    pub fn new(config: DiscordPlatformConfig) -> Arc<Self> {
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            config: DiscordPlatformConfig {
                name: if config.name.trim().is_empty() {
                    "discord".to_string()
                } else {
                    config.name
                },
                api_base: if config.api_base.trim().is_empty() {
                    "https://discord.com/api/v10".to_string()
                } else {
                    config.api_base.trim_end_matches('/').to_string()
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

    pub async fn wait_for_outbound(&self) -> Option<DiscordOutboundRecord> {
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
            client: self.client.clone(),
        })
    }
}

#[async_trait]
impl Platform for DiscordPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);
        let platform = self.clone_for_task();
        tokio::spawn(async move {
            run_discord_gateway(platform, shutdown_rx).await;
        });
        tracing::info!("discord platform started");
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: DiscordReplyContext = serde_json::from_str(&reply_ctx.value)?;
        let record = DiscordOutboundRecord {
            channel_id: ctx.channel_id.clone(),
            message_id: ctx.message_id.clone(),
            content: content.clone(),
        };
        self.outbox.lock().await.push(record.clone());
        let _ = self.out_tx.send(record);

        if self.config.dry_run {
            return Ok(());
        }
        let mut body = json!({ "content": content });
        if let Some(message_id) = ctx.message_id {
            body["message_reference"] = json!({ "message_id": message_id });
        }
        self.client
            .post(format!(
                "{}/channels/{}/messages",
                self.config.api_base, ctx.channel_id
            ))
            .bearer_auth(&self.config.token)
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

async fn run_discord_gateway(
    platform: Arc<DiscordPlatform>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    loop {
        match run_discord_gateway_once(Arc::clone(&platform), &mut shutdown_rx).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => tracing::warn!(error = %err, "discord gateway disconnected"),
        }
        tokio::select! {
            _ = &mut shutdown_rx => return,
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
}

async fn run_discord_gateway_once(
    platform: Arc<DiscordPlatform>,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> Result<bool> {
    let gateway_url = discord_gateway_url(&platform).await?;
    let url = if gateway_url.contains('?') {
        gateway_url
    } else {
        format!("{gateway_url}?v=10&encoding=json")
    };
    let (stream, _) = connect_async(&url).await?;
    tracing::info!("discord gateway connected");
    let (mut write, mut read) = stream.split();
    let mut heartbeat_interval: Option<tokio::time::Interval> = None;
    let mut last_sequence: Option<i64> = None;

    loop {
        tokio::select! {
            _ = &mut *shutdown_rx => {
                let _ = write.close().await;
                return Ok(true);
            }
            _ = async {
                if let Some(interval) = &mut heartbeat_interval {
                    interval.tick().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                write.send(WsMessage::Text(json!({ "op": 1, "d": last_sequence }).to_string())).await?;
            }
            message = read.next() => {
                let Some(message) = message else {
                    return Ok(false);
                };
                match message? {
                    WsMessage::Text(text) => {
                        let value: Value = serde_json::from_str(&text)?;
                        if let Some(seq) = value.get("s").and_then(Value::as_i64) {
                            last_sequence = Some(seq);
                        }
                        match value.get("op").and_then(Value::as_i64) {
                            Some(10) => {
                                let interval_ms = value.pointer("/d/heartbeat_interval").and_then(Value::as_u64).unwrap_or(45_000);
                                heartbeat_interval = Some(tokio::time::interval(Duration::from_millis(interval_ms)));
                                let identify = json!({
                                    "op": 2,
                                    "d": {
                                        "token": platform.config.token,
                                        "intents": 33280,
                                        "properties": {
                                            "os": std::env::consts::OS,
                                            "browser": "agentlink",
                                            "device": "agentlink"
                                        }
                                    }
                                });
                                write.send(WsMessage::Text(identify.to_string())).await?;
                            }
                            Some(0) => {
                                if value.get("t").and_then(Value::as_str) == Some("MESSAGE_CREATE") {
                                    match discord_message_from_dispatch(&platform, value.get("d").unwrap_or(&Value::Null)) {
                                        Ok(Some(message)) => dispatch(platform.clone(), message).await,
                                        Ok(None) => {}
                                        Err(err) => tracing::warn!(error = %err, "discord message parse failed"),
                                    }
                                }
                            }
                            _ => {}
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

async fn discord_gateway_url(platform: &DiscordPlatform) -> Result<String> {
    if let Some(url) = &platform.config.gateway_url {
        return Ok(url.clone());
    }
    let value: Value = platform
        .client
        .get(format!("{}/gateway/bot", platform.config.api_base))
        .bearer_auth(&platform.config.token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    value
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("discord gateway response missing url"))
}

fn discord_message_from_dispatch(
    platform: &DiscordPlatform,
    event: &Value,
) -> Result<Option<Message>> {
    let author = event
        .get("author")
        .ok_or_else(|| anyhow!("missing author"))?;
    if author.get("bot").and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let content = event
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Ok(None);
    }
    let channel_id = event
        .get("channel_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing channel_id"))?
        .to_string();
    let user_id = author
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing author.id"))?
        .to_string();
    let message_id = event.get("id").and_then(Value::as_str).map(str::to_string);
    let session_key = if platform.config.share_session_in_channel {
        format!("{}:{}", platform.config.name, channel_id)
    } else {
        format!("{}:{}:{}", platform.config.name, channel_id, user_id)
    };
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.config.name.clone()),
        message_id: message_id.clone(),
        message_type: crate::core::MessageType::Text,
        user_id,
        user_name: author
            .get("username")
            .and_then(Value::as_str)
            .map(str::to_string),
        content,
        attachments: Vec::new(),
        images: Vec::new(),
        files: Vec::new(),
        reply_ctx: ReplyContext {
            value: serde_json::to_string(&DiscordReplyContext {
                channel_id,
                message_id,
            })?,
        },
        created_at: std::time::SystemTime::now(),
    }))
}

async fn dispatch(platform: Arc<DiscordPlatform>, message: Message) {
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

impl TryFrom<toml::value::Table> for DiscordPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let dry_run = bool_option(&opts, "dry_run").unwrap_or(false);
        let token = string_option(&opts, "token").unwrap_or_default();
        if token.trim().is_empty() && !dry_run {
            return Err(anyhow!("discord requires token unless dry_run = true"));
        }
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "discord".to_string()),
            token,
            api_base: string_option(&opts, "api_base")
                .unwrap_or_else(|| "https://discord.com/api/v10".to_string()),
            gateway_url: string_option(&opts, "gateway_url"),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run,
        })
    }
}
