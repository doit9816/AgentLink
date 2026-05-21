mod codec;
mod ws;

use crate::core::{MessageHandler, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex};

pub use codec::{
    BridgeAttachment, BridgeBinaryData, InboundMessage, RegisterAckMessage, RegisterMessage,
    ReplyMessage,
};

pub const MESSAGE_TYPE_REGISTER: &str = "register";
pub const MESSAGE_TYPE_REGISTER_ACK: &str = "register_ack";
pub const MESSAGE_TYPE_MESSAGE: &str = "message";
pub const MESSAGE_TYPE_REPLY: &str = "reply";
pub const MESSAGE_TYPE_PING: &str = "ping";
pub const MESSAGE_TYPE_PONG: &str = "pong";
pub const MESSAGE_TYPE_ERROR: &str = "error";

#[derive(Debug, Clone, Default)]
pub struct BridgePlatformConfig {
    pub name: String,
    pub listen: String,
    pub path: String,
    pub token: Option<String>,
    pub insecure: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ErrorMessage {
    #[serde(rename = "type")]
    message_type: String,
    code: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PongMessage {
    #[serde(rename = "type")]
    message_type: String,
    ts: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct BridgeReplyContext {
    pub(crate) platform: String,
    pub(crate) session_key: String,
    pub(crate) reply_ctx: String,
}

#[derive(Debug, Clone)]
pub(crate) struct BridgeAdapter {
    pub(crate) platform: String,
    pub(crate) capabilities: Vec<String>,
    pub(crate) metadata: Option<Value>,
    pub(crate) tx: mpsc::UnboundedSender<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeAdapterInfo {
    pub platform: String,
    pub capabilities: Vec<String>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct HealthResponse {
    ok: bool,
    platform: String,
    adapters: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeOutboundRecord {
    pub adapter: String,
    pub message: ReplyMessage,
}

pub struct BridgePlatform {
    pub(crate) name: String,
    pub(crate) listen: String,
    pub(crate) path: String,
    pub(crate) token: Option<String>,
    pub(crate) insecure: bool,
    pub(crate) handler: Arc<Mutex<Option<MessageHandler>>>,
    pub(crate) adapters: Arc<Mutex<HashMap<String, BridgeAdapter>>>,
    pub(crate) outbox: Arc<Mutex<Vec<BridgeOutboundRecord>>>,
    pub(crate) local_addr: Arc<Mutex<Option<SocketAddr>>>,
    pub(crate) shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl BridgePlatform {
    pub fn new(config: BridgePlatformConfig) -> Arc<Self> {
        Arc::new(Self {
            name: if config.name.trim().is_empty() {
                "bridge".to_string()
            } else {
                config.name
            },
            listen: if config.listen.trim().is_empty() {
                "127.0.0.1:9810".to_string()
            } else {
                config.listen
            },
            path: codec::normalize_path(if config.path.trim().is_empty() {
                "/bridge/ws".to_string()
            } else {
                config.path
            }),
            token: config.token.filter(|value| !value.trim().is_empty()),
            insecure: config.insecure,
            handler: Arc::new(Mutex::new(None)),
            adapters: Arc::new(Mutex::new(HashMap::new())),
            outbox: Arc::new(Mutex::new(Vec::new())),
            local_addr: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn local_addr(&self) -> Option<SocketAddr> {
        *self.local_addr.lock().await
    }

    pub async fn outbox(&self) -> Vec<BridgeOutboundRecord> {
        self.outbox.lock().await.clone()
    }

    pub async fn adapter_infos(&self) -> Vec<BridgeAdapterInfo> {
        self.adapters
            .lock()
            .await
            .values()
            .map(|adapter| BridgeAdapterInfo {
                platform: adapter.platform.clone(),
                capabilities: adapter.capabilities.clone(),
                metadata: adapter.metadata.clone(),
            })
            .collect()
    }

    async fn send_to_adapter(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx = codec::parse_reply_context(&reply_ctx.value)?;
        let message = ReplyMessage {
            message_type: MESSAGE_TYPE_REPLY.to_string(),
            project: None,
            session_key: ctx.session_key,
            reply_ctx: Some(ctx.reply_ctx),
            content,
            format: Some("text".to_string()),
        };
        let payload = serde_json::to_string(&message)?;
        let adapters = self.adapters.lock().await;
        let Some(adapter) = adapters.get(&ctx.platform) else {
            return Err(anyhow!(
                "bridge adapter `{}` is not connected",
                ctx.platform
            ));
        };
        adapter
            .tx
            .send(payload)
            .map_err(|_| anyhow!("bridge adapter `{}` is disconnected", ctx.platform))?;
        drop(adapters);
        self.outbox.lock().await.push(BridgeOutboundRecord {
            adapter: ctx.platform,
            message,
        });
        Ok(())
    }

    fn clone_for_server(&self) -> Self {
        Self {
            name: self.name.clone(),
            listen: self.listen.clone(),
            path: self.path.clone(),
            token: self.token.clone(),
            insecure: self.insecure,
            handler: Arc::clone(&self.handler),
            adapters: Arc::clone(&self.adapters),
            outbox: Arc::clone(&self.outbox),
            local_addr: Arc::clone(&self.local_addr),
            shutdown: Arc::clone(&self.shutdown),
        }
    }
}

#[async_trait]
impl Platform for BridgePlatform {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        if self.token.is_none() && !self.insecure {
            return Err(anyhow!(
                "bridge platform requires token, or set insecure = true for local development"
            ));
        }
        *self.handler.lock().await = Some(handler);

        let listener = TcpListener::bind(&self.listen).await?;
        let addr = listener.local_addr()?;
        *self.local_addr.lock().await = Some(addr);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(shutdown_tx);

        let state = Arc::new(self.clone_for_server());
        let app = Router::new()
            .route(&self.path, get(ws::ws_handler))
            .route("/bridge/healthz", get(ws::health))
            .route("/bridge/adapters", get(ws::adapters))
            .with_state(state);

        tokio::spawn(async move {
            if let Err(err) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
            {
                tracing::error!(error = %err, "bridge websocket server stopped with error");
            }
        });
        tracing::info!(addr = %addr, path = %self.path, "bridge platform started");
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        self.send_to_adapter(reply_ctx, content).await
    }

    async fn send(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        self.send_to_adapter(reply_ctx, content).await
    }

    async fn stop(&self) -> Result<()> {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

impl std::fmt::Debug for BridgePlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgePlatform")
            .field("name", &self.name)
            .field("listen", &self.listen)
            .field("path", &self.path)
            .field("token", &self.token.as_ref().map(|_| "***"))
            .field("insecure", &self.insecure)
            .finish()
    }
}
