mod config;
mod webhook;

use crate::core::{MessageHandler, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

pub use config::DingTalkPlatformConfig;
use webhook::{dingtalk_health, dingtalk_webhook, normalize_path, preprocess_markdown};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DingTalkOutboundRecord {
    pub session_webhook: Option<String>,
    pub conversation_id: String,
    pub sender_staff_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DingTalkReplyContext {
    pub(crate) session_webhook: Option<String>,
    pub(crate) conversation_id: String,
    pub(crate) sender_staff_id: String,
}

pub struct DingTalkPlatform {
    pub(crate) config: DingTalkPlatformConfig,
    pub(crate) handler: Arc<Mutex<Option<MessageHandler>>>,
    pub(crate) outbox: Arc<Mutex<Vec<DingTalkOutboundRecord>>>,
    pub(crate) out_tx: mpsc::UnboundedSender<DingTalkOutboundRecord>,
    pub(crate) out_rx: Arc<Mutex<mpsc::UnboundedReceiver<DingTalkOutboundRecord>>>,
    pub(crate) local_addr: Arc<Mutex<Option<SocketAddr>>>,
    pub(crate) shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    pub(crate) client: reqwest::Client,
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
        let listener = tokio::net::TcpListener::bind(&self.config.listen).await?;
        let addr = listener.local_addr()?;
        *self.local_addr.lock().await = Some(addr);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);
        let app = axum::Router::new()
            .route(
                &self.config.callback_path,
                axum::routing::post(dingtalk_webhook),
            )
            .route("/dingtalk/healthz", axum::routing::get(dingtalk_health))
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
