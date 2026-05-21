mod config;
mod websocket;

use crate::core::{MessageHandler, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::sync::{mpsc, oneshot, Mutex};

pub use config::QqPlatformConfig;
use websocket::run_qq_websocket;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct QqOutboundRecord {
    pub message_type: String,
    pub user_id: Option<i64>,
    pub group_id: Option<i64>,
    pub message_id: Option<i64>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct QqReplyContext {
    pub(crate) message_type: String,
    pub(crate) user_id: Option<i64>,
    pub(crate) group_id: Option<i64>,
    pub(crate) message_id: Option<i64>,
}

pub struct QqPlatform {
    pub(crate) config: QqPlatformConfig,
    pub(crate) handler: Arc<Mutex<Option<MessageHandler>>>,
    pub(crate) outbox: Arc<Mutex<Vec<QqOutboundRecord>>>,
    pub(crate) out_tx: mpsc::UnboundedSender<QqOutboundRecord>,
    pub(crate) out_rx: Arc<Mutex<mpsc::UnboundedReceiver<QqOutboundRecord>>>,
    pub(crate) shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    pub(crate) write_tx: Arc<Mutex<Option<mpsc::UnboundedSender<Value>>>>,
    pub(crate) echo_seq: AtomicU64,
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

    pub(crate) fn clone_for_task(&self) -> Arc<Self> {
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
