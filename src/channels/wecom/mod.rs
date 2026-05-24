mod reply;
mod stream;
mod webhook;

pub use crate::channels::support::WeComPlatformConfig;
use crate::channels::support::{
    emit_outbound, normalize_path, HasOutbox, SimpleOutboundRecord, SimpleReplyContext,
};
use crate::core::{MessageHandler, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex};

use reply::wecom_access_token;
use stream::{build_respond_msg_frame, run as run_wecom_stream};
use webhook::{wecom_verify, wecom_webhook};

fn uses_websocket(config: &WeComPlatformConfig) -> bool {
    config.uses_websocket()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WeComBotReplyContext {
    pub(crate) req_id: String,
    pub(crate) user_id: String,
    pub(crate) chat_id: String,
    pub(crate) chat_type: String,
    pub(crate) msg_id: Option<String>,
}

pub struct WeComPlatform {
    pub(crate) config: WeComPlatformConfig,
    pub(crate) handler: Arc<Mutex<Option<MessageHandler>>>,
    pub(crate) outbox: Arc<Mutex<Vec<SimpleOutboundRecord>>>,
    pub(crate) out_tx: mpsc::UnboundedSender<SimpleOutboundRecord>,
    pub(crate) out_rx: Arc<Mutex<mpsc::UnboundedReceiver<SimpleOutboundRecord>>>,
    pub(crate) local_addr: Arc<Mutex<Option<SocketAddr>>>,
    pub(crate) shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    pub(crate) stream_shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    pub(crate) ws_out_tx: mpsc::UnboundedSender<String>,
    pub(crate) ws_out_rx: Arc<Mutex<Option<mpsc::UnboundedReceiver<String>>>>,
    pub(crate) client: reqwest::Client,
}

impl WeComPlatform {
    pub fn new(config: WeComPlatformConfig) -> Arc<Self> {
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        let (ws_out_tx, ws_out_rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            config: WeComPlatformConfig {
                name: if config.name.trim().is_empty() {
                    "wecom".to_string()
                } else {
                    config.name
                },
                listen: if config.listen.trim().is_empty() {
                    "127.0.0.1:18500".to_string()
                } else {
                    config.listen
                },
                callback_path: normalize_path(if config.callback_path.trim().is_empty() {
                    "/wecom/callback".to_string()
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
            stream_shutdown: Arc::new(Mutex::new(None)),
            ws_out_tx,
            ws_out_rx: Arc::new(Mutex::new(Some(ws_out_rx))),
            client: reqwest::Client::new(),
        })
    }

    pub async fn local_addr(&self) -> Option<SocketAddr> {
        *self.local_addr.lock().await
    }

    pub async fn wait_for_outbound(&self) -> Option<SimpleOutboundRecord> {
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
            stream_shutdown: Arc::clone(&self.stream_shutdown),
            ws_out_tx: self.ws_out_tx.clone(),
            ws_out_rx: Arc::clone(&self.ws_out_rx),
            client: self.client.clone(),
        }
    }

    fn queue_ws_frame(&self, frame: String) -> Result<()> {
        self.ws_out_tx
            .send(frame)
            .map_err(|_| anyhow!("wecom stream outbound channel closed"))
    }
}

impl HasOutbox for WeComPlatform {
    fn outbox(&self) -> &Arc<Mutex<Vec<SimpleOutboundRecord>>> {
        &self.outbox
    }

    fn out_tx(&self) -> &mpsc::UnboundedSender<SimpleOutboundRecord> {
        &self.out_tx
    }
}

#[async_trait]
impl Platform for WeComPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        if uses_websocket(&self.config) {
            let ws_out_rx = self
                .ws_out_rx
                .lock()
                .await
                .take()
                .ok_or_else(|| anyhow!("wecom stream outbound receiver already taken"))?;
            let (shutdown_tx, shutdown_rx) = oneshot::channel();
            *self.stream_shutdown.lock().await = Some(shutdown_tx);
            let platform = Arc::new(self.clone_for_server());
            tokio::spawn(async move {
                run_wecom_stream(platform, shutdown_rx, ws_out_rx).await;
            });
            tracing::info!(
                mode = %self.config.connection_mode,
                "wecom platform started (stream mode)"
            );
            return Ok(());
        }

        let listener = TcpListener::bind(&self.config.listen).await?;
        *self.local_addr.lock().await = Some(listener.local_addr()?);
        let (tx, rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(tx);
        let path = self.config.callback_path.clone();
        let app = Router::new()
            .route(&path, get(wecom_verify).post(wecom_webhook))
            .with_state(Arc::new(self.clone_for_server()));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });
        let addr = self.local_addr.lock().await;
        tracing::info!(
            addr = %addr.unwrap(),
            path = %self.config.callback_path,
            "wecom platform started (webhook mode)"
        );
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: SimpleReplyContext = serde_json::from_str(&reply_ctx.value)?;
        emit_outbound(self, &ctx, &content).await;
        if self.config.dry_run {
            return Ok(());
        }

        if uses_websocket(&self.config) {
            let bot: WeComBotReplyContext = if ctx.extra.is_object() {
                serde_json::from_value(ctx.extra.clone()).unwrap_or(WeComBotReplyContext {
                    req_id: String::new(),
                    user_id: ctx.target.clone(),
                    chat_id: ctx.target.clone(),
                    chat_type: "single".to_string(),
                    msg_id: ctx.message_id.clone(),
                })
            } else {
                WeComBotReplyContext {
                    req_id: String::new(),
                    user_id: ctx.target.clone(),
                    chat_id: ctx.target.clone(),
                    chat_type: "single".to_string(),
                    msg_id: ctx.message_id.clone(),
                }
            };
            let req_id = if bot.req_id.trim().is_empty() {
                return Err(anyhow!(
                    "wecom stream reply requires req_id from inbound aibot_msg_callback"
                ));
            } else {
                bot.req_id
            };
            return self.queue_ws_frame(build_respond_msg_frame(&req_id, &content));
        }

        let token = wecom_access_token(self).await?;
        let agent_id = ctx
            .extra
            .get("agent_id")
            .and_then(Value::as_str)
            .or_else(|| self.config.token.as_deref())
            .unwrap_or("0");
        let base = if self.config.api_base.trim().is_empty() {
            "https://qyapi.weixin.qq.com".to_string()
        } else {
            self.config.api_base.trim_end_matches('/').to_string()
        };
        self.client
            .post(format!("{base}/cgi-bin/message/send?access_token={token}"))
            .json(&json!({
                "touser": ctx.target,
                "msgtype": "text",
                "agentid": agent_id,
                "text": { "content": content }
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
        if let Some(tx) = self.stream_shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}
