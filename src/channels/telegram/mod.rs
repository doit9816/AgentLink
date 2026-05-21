mod config;
mod polling;

use crate::core::{MessageHandler, Platform, ReplyContext};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

pub use config::TelegramPlatformConfig;
use polling::run_telegram_polling;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TelegramOutboundRecord {
    pub chat_id: String,
    pub message_id: Option<i64>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TelegramReplyContext {
    pub(crate) chat_id: String,
    pub(crate) message_id: Option<i64>,
}

pub struct TelegramPlatform {
    pub(crate) config: TelegramPlatformConfig,
    pub(crate) handler: Arc<Mutex<Option<MessageHandler>>>,
    pub(crate) outbox: Arc<Mutex<Vec<TelegramOutboundRecord>>>,
    pub(crate) out_tx: mpsc::UnboundedSender<TelegramOutboundRecord>,
    pub(crate) out_rx: Arc<Mutex<mpsc::UnboundedReceiver<TelegramOutboundRecord>>>,
    pub(crate) shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    pub(crate) client: reqwest::Client,
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

    pub(crate) fn method_url(&self, method: &str) -> String {
        format!(
            "{}/bot{}/{}",
            self.config.api_base, self.config.token, method
        )
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
