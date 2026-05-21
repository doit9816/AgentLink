mod auth;
mod config;
mod payload;
mod webhook;
mod websocket;

use crate::core::{MessageHandler, Platform, ReplyContext};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

pub(crate) const FEISHU_WS_ENDPOINT_PATH: &str = "/callback/ws/endpoint";

pub use config::FeishuPlatformConfig;
use webhook::normalize_path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeishuOutboundRecord {
    pub chat_id: String,
    pub message_id: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FeishuReplyContext {
    pub(crate) message_id: Option<String>,
    pub(crate) chat_id: String,
}

pub struct FeishuPlatform {
    pub(crate) config: FeishuPlatformConfig,
    pub(crate) handler: Arc<Mutex<Option<MessageHandler>>>,
    pub(crate) outbox: Arc<Mutex<Vec<FeishuOutboundRecord>>>,
    pub(crate) out_tx: mpsc::UnboundedSender<FeishuOutboundRecord>,
    pub(crate) out_rx: Arc<Mutex<mpsc::UnboundedReceiver<FeishuOutboundRecord>>>,
    pub(crate) local_addr: Arc<Mutex<Option<SocketAddr>>>,
    pub(crate) shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    pub(crate) client: reqwest::Client,
    pub(crate) token: Arc<Mutex<Option<auth::CachedToken>>>,
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

    pub(crate) fn should_use_websocket(&self) -> bool {
        self.config.connection_mode == "websocket"
            && !(self.config.dry_run && self.config.app_id.trim().is_empty())
    }

    pub(crate) fn clone_for_server(&self) -> Self {
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
                websocket::run(platform, shutdown_rx).await;
            });
            tracing::info!(mode = "websocket", api_base = %self.config.api_base, "feishu platform started");
            return Ok(());
        }

        webhook::start_http_server(self.clone_for_server(), shutdown_rx).await?;
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
        let token = auth::tenant_access_token(self).await?;
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
