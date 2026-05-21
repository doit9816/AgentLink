mod config;
mod socket_mode;

use crate::core::{MessageHandler, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

pub use config::SlackPlatformConfig;
use socket_mode::run_slack_socket_mode;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SlackOutboundRecord {
    pub channel: String,
    pub thread_ts: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SlackReplyContext {
    pub(crate) channel: String,
    pub(crate) thread_ts: Option<String>,
}

pub struct SlackPlatform {
    pub(crate) config: SlackPlatformConfig,
    pub(crate) handler: Arc<Mutex<Option<MessageHandler>>>,
    pub(crate) outbox: Arc<Mutex<Vec<SlackOutboundRecord>>>,
    pub(crate) out_tx: mpsc::UnboundedSender<SlackOutboundRecord>,
    pub(crate) out_rx: Arc<Mutex<mpsc::UnboundedReceiver<SlackOutboundRecord>>>,
    pub(crate) shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    pub(crate) client: reqwest::Client,
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

    pub(crate) fn clone_for_task(&self) -> Arc<Self> {
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
