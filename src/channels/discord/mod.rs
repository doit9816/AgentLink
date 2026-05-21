mod config;
mod gateway;

use crate::core::{MessageHandler, Platform, ReplyContext};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

pub use config::DiscordPlatformConfig;
use gateway::run_discord_gateway;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscordOutboundRecord {
    pub channel_id: String,
    pub message_id: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DiscordReplyContext {
    pub(crate) channel_id: String,
    pub(crate) message_id: Option<String>,
}

pub struct DiscordPlatform {
    pub(crate) config: DiscordPlatformConfig,
    pub(crate) handler: Arc<Mutex<Option<MessageHandler>>>,
    pub(crate) outbox: Arc<Mutex<Vec<DiscordOutboundRecord>>>,
    pub(crate) out_tx: mpsc::UnboundedSender<DiscordOutboundRecord>,
    pub(crate) out_rx: Arc<Mutex<mpsc::UnboundedReceiver<DiscordOutboundRecord>>>,
    pub(crate) shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    pub(crate) client: reqwest::Client,
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
