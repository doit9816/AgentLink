mod config;
mod server;

use crate::core::{MessageHandler, Platform, ReplyContext};
use anyhow::Result;
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};

pub use config::HttpPlatformConfig;
pub use server::{HttpInboundMessage, HttpOutboundMessage};

pub struct HttpPlatform {
    pub(crate) name: String,
    pub(crate) listen: String,
    pub(crate) bearer_token: Option<String>,
    pub(crate) outbound_url: Option<String>,
    pub(crate) handler: Arc<Mutex<Option<MessageHandler>>>,
    pub(crate) outbox: Arc<Mutex<Vec<HttpOutboundMessage>>>,
    pub(crate) out_tx: mpsc::UnboundedSender<HttpOutboundMessage>,
    pub(crate) out_rx: Arc<Mutex<mpsc::UnboundedReceiver<HttpOutboundMessage>>>,
    pub(crate) local_addr: Arc<Mutex<Option<SocketAddr>>>,
    pub(crate) shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    pub(crate) client: reqwest::Client,
}

impl HttpPlatform {
    pub fn new(config: HttpPlatformConfig) -> Arc<Self> {
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            name: if config.name.trim().is_empty() {
                "http".to_string()
            } else {
                config.name
            },
            listen: if config.listen.trim().is_empty() {
                "127.0.0.1:18080".to_string()
            } else {
                config.listen
            },
            bearer_token: config.bearer_token.filter(|v| !v.trim().is_empty()),
            outbound_url: config.outbound_url.filter(|v| !v.trim().is_empty()),
            handler: Arc::new(Mutex::new(None)),
            outbox: Arc::new(Mutex::new(Vec::new())),
            out_tx,
            out_rx: Arc::new(Mutex::new(out_rx)),
            local_addr: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(Mutex::new(None)),
            client: reqwest::Client::new(),
        })
    }

    pub fn from_options(opts: toml::value::Table) -> Self {
        let config = HttpPlatformConfig {
            name: config::string_option(&opts, "name").unwrap_or_else(|| "http".to_string()),
            listen: config::string_option(&opts, "listen")
                .unwrap_or_else(|| "127.0.0.1:18080".to_string()),
            bearer_token: config::string_option(&opts, "bearer_token"),
            outbound_url: config::string_option(&opts, "outbound_url"),
        };
        let platform = Self::new(config);
        Arc::try_unwrap(platform).unwrap_or_else(|_| unreachable!("new platform has one owner"))
    }

    pub async fn local_addr(&self) -> Option<SocketAddr> {
        *self.local_addr.lock().await
    }

    pub async fn outbox(&self) -> Vec<HttpOutboundMessage> {
        self.outbox.lock().await.clone()
    }

    pub async fn wait_for_outbound(&self) -> Option<HttpOutboundMessage> {
        self.out_rx.lock().await.recv().await
    }

    pub(crate) async fn record_and_forward(
        &self,
        kind: &str,
        reply_ctx: ReplyContext,
        content: String,
    ) -> Result<()> {
        let msg = HttpOutboundMessage {
            kind: kind.to_string(),
            reply_ctx,
            content,
        };
        self.outbox.lock().await.push(msg.clone());
        let _ = self.out_tx.send(msg.clone());

        if let Some(url) = &self.outbound_url {
            self.client
                .post(url)
                .json(&msg)
                .send()
                .await?
                .error_for_status()?;
        }
        Ok(())
    }

    pub(crate) fn clone_for_server(&self) -> Self {
        Self {
            name: self.name.clone(),
            listen: self.listen.clone(),
            bearer_token: self.bearer_token.clone(),
            outbound_url: self.outbound_url.clone(),
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
impl Platform for HttpPlatform {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        server::start_http_server(self.clone_for_server()).await
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        self.record_and_forward("reply", reply_ctx, content).await
    }

    async fn send(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        self.record_and_forward("send", reply_ctx, content).await
    }

    async fn stop(&self) -> Result<()> {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

impl std::fmt::Debug for HttpPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpPlatform")
            .field("name", &self.name)
            .field("listen", &self.listen)
            .field("bearer_token", &self.bearer_token.as_ref().map(|_| "***"))
            .field("outbound_url", &self.outbound_url)
            .finish()
    }
}
