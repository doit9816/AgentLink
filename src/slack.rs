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
pub struct SlackPlatformConfig {
    pub name: String,
    pub bot_token: String,
    pub app_token: String,
    pub api_base: String,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

impl Default for SlackPlatformConfig {
    fn default() -> Self {
        Self {
            name: "slack".to_string(),
            bot_token: String::new(),
            app_token: String::new(),
            api_base: "https://slack.com/api".to_string(),
            share_session_in_channel: false,
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlackOutboundRecord {
    pub channel: String,
    pub thread_ts: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SlackReplyContext {
    channel: String,
    thread_ts: Option<String>,
}

pub struct SlackPlatform {
    config: SlackPlatformConfig,
    handler: Arc<Mutex<Option<MessageHandler>>>,
    outbox: Arc<Mutex<Vec<SlackOutboundRecord>>>,
    out_tx: mpsc::UnboundedSender<SlackOutboundRecord>,
    out_rx: Arc<Mutex<mpsc::UnboundedReceiver<SlackOutboundRecord>>>,
    shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    client: reqwest::Client,
}

impl SlackPlatform {
    pub fn new(config: SlackPlatformConfig) -> Arc<Self> {
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            config: SlackPlatformConfig {
                name: if config.name.trim().is_empty() {
                    "slack".to_string()
                } else {
                    config.name
                },
                api_base: if config.api_base.trim().is_empty() {
                    "https://slack.com/api".to_string()
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

    pub async fn wait_for_outbound(&self) -> Option<SlackOutboundRecord> {
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
impl Platform for SlackPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);
        let platform = self.clone_for_task();
        tokio::spawn(async move {
            run_slack_socket_mode(platform, shutdown_rx).await;
        });
        tracing::info!("slack platform started");
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: SlackReplyContext = serde_json::from_str(&reply_ctx.value)?;
        let record = SlackOutboundRecord {
            channel: ctx.channel.clone(),
            thread_ts: ctx.thread_ts.clone(),
            content: content.clone(),
        };
        self.outbox.lock().await.push(record.clone());
        let _ = self.out_tx.send(record);

        if self.config.dry_run {
            return Ok(());
        }
        let mut body = json!({
            "channel": ctx.channel,
            "text": content,
            "unfurl_links": false,
            "unfurl_media": false
        });
        if let Some(thread_ts) = ctx.thread_ts {
            body["thread_ts"] = json!(thread_ts);
        }
        let value: Value = self
            .client
            .post(format!("{}/chat.postMessage", self.config.api_base))
            .bearer_auth(&self.config.bot_token)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if value.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(anyhow!(
                "slack chat.postMessage failed: {}",
                value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            ));
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

async fn run_slack_socket_mode(
    platform: Arc<SlackPlatform>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    loop {
        match run_slack_socket_once(Arc::clone(&platform), &mut shutdown_rx).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => tracing::warn!(error = %err, "slack socket mode disconnected"),
        }
        tokio::select! {
            _ = &mut shutdown_rx => return,
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
}

async fn run_slack_socket_once(
    platform: Arc<SlackPlatform>,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> Result<bool> {
    let socket_url = open_slack_socket(&platform).await?;
    let (stream, _) = connect_async(&socket_url).await?;
    tracing::info!("slack socket mode connected");
    let (mut write, mut read) = stream.split();
    loop {
        tokio::select! {
            _ = &mut *shutdown_rx => {
                let _ = write.close().await;
                return Ok(true);
            }
            message = read.next() => {
                let Some(message) = message else {
                    return Ok(false);
                };
                match message? {
                    WsMessage::Text(text) => {
                        let value: Value = serde_json::from_str(&text)?;
                        if let Some(envelope_id) = value.get("envelope_id").and_then(Value::as_str) {
                            write.send(WsMessage::Text(json!({ "envelope_id": envelope_id }).to_string())).await?;
                        }
                        match slack_message_from_envelope(&platform, &value) {
                            Ok(Some(message)) => dispatch(platform.clone(), message).await,
                            Ok(None) => {}
                            Err(err) => tracing::warn!(error = %err, "slack event parse failed"),
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

async fn open_slack_socket(platform: &SlackPlatform) -> Result<String> {
    let value: Value = platform
        .client
        .post(format!(
            "{}/apps.connections.open",
            platform.config.api_base
        ))
        .bearer_auth(&platform.config.app_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(anyhow!(
            "slack apps.connections.open failed: {}",
            value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ));
    }
    value
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("slack apps.connections.open missing url"))
}

fn slack_message_from_envelope(
    platform: &SlackPlatform,
    envelope: &Value,
) -> Result<Option<Message>> {
    if envelope.get("type").and_then(Value::as_str) != Some("events_api") {
        return Ok(None);
    }
    let payload = envelope
        .get("payload")
        .ok_or_else(|| anyhow!("missing payload"))?;
    let event = payload
        .get("event")
        .ok_or_else(|| anyhow!("missing event"))?;
    let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
    if event_type != "app_mention" && event_type != "message" {
        return Ok(None);
    }
    if event.get("bot_id").is_some() || event.get("subtype").is_some() {
        return Ok(None);
    }
    let content = event
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Ok(None);
    }
    let channel = event
        .get("channel")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing channel"))?
        .to_string();
    let user_id = event
        .get("user")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing user"))?
        .to_string();
    let ts = event
        .get("ts")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let thread_ts = event
        .get("thread_ts")
        .and_then(Value::as_str)
        .unwrap_or(&ts)
        .to_string();
    let session_key = if platform.config.share_session_in_channel {
        format!("{}:{}", platform.config.name, channel)
    } else {
        format!("{}:{}:{}", platform.config.name, channel, user_id)
    };
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.config.name.clone()),
        message_id: Some(ts.clone()),
        message_type: crate::core::MessageType::Text,
        user_id,
        user_name: None,
        content,
        attachments: Vec::new(),
        images: Vec::new(),
        files: Vec::new(),
        reply_ctx: ReplyContext {
            value: serde_json::to_string(&SlackReplyContext {
                channel,
                thread_ts: Some(thread_ts),
            })?,
        },
        created_at: std::time::SystemTime::now(),
    }))
}

async fn dispatch(platform: Arc<SlackPlatform>, message: Message) {
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

impl TryFrom<toml::value::Table> for SlackPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let dry_run = bool_option(&opts, "dry_run").unwrap_or(false);
        let bot_token = string_option(&opts, "bot_token").unwrap_or_default();
        let app_token = string_option(&opts, "app_token").unwrap_or_default();
        if !dry_run && (bot_token.trim().is_empty() || app_token.trim().is_empty()) {
            return Err(anyhow!(
                "slack requires bot_token and app_token unless dry_run = true"
            ));
        }
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "slack".to_string()),
            bot_token,
            app_token,
            api_base: string_option(&opts, "api_base")
                .unwrap_or_else(|| "https://slack.com/api".to_string()),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run,
        })
    }
}
