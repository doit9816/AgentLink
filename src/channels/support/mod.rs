mod common;
mod config;
pub(crate) mod ws;

pub(crate) use common::*;
pub use config::{
    max_config_from_options, qqbot_config_from_options, weibo_config_from_options,
    weixin_config_from_options, LinePlatformConfig, PollPlatformConfig, WeComPlatformConfig,
};

pub(crate) use crate::core::{Message, MessageHandler, Platform, ReplyContext};
pub(crate) use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
pub(crate) use anyhow::{anyhow, Result};
pub(crate) use async_trait::async_trait;
pub(crate) use axum::body::Bytes;
pub(crate) use axum::extract::{Query, State};
pub(crate) use axum::http::{HeaderMap, StatusCode};
pub(crate) use axum::response::{IntoResponse, Response};
pub(crate) use axum::routing::{get, post};
pub(crate) use axum::{Json, Router};
pub(crate) use base64::Engine;
pub(crate) use futures_util::{SinkExt, StreamExt};
pub(crate) use rand::Rng;
pub(crate) use serde_json::{json, Value};
pub(crate) use sha1::{Digest as Sha1Digest, Sha1};
pub(crate) use std::collections::BTreeMap;
use std::net::SocketAddr;
pub(crate) use std::sync::Arc;
pub(crate) use std::time::Duration;
pub(crate) use tokio::net::TcpListener;
use tokio::sync::mpsc;
pub(crate) use tokio::sync::{oneshot, Mutex};
pub(crate) use tokio_tungstenite::connect_async;
pub(crate) use tokio_tungstenite::tungstenite::client::IntoClientRequest;
pub(crate) use tokio_tungstenite::tungstenite::Message as WsMessage;

macro_rules! webhook_platform_struct {
    ($config:ident, $platform:ident, $default_name:literal, $default_listen:literal, $default_path:literal) => {
        pub struct $platform {
            pub(crate) config: $config,
            pub(crate) handler: Arc<Mutex<Option<MessageHandler>>>,
            pub(crate) outbox: Arc<Mutex<Vec<SimpleOutboundRecord>>>,
            pub(crate) out_tx: mpsc::UnboundedSender<SimpleOutboundRecord>,
            pub(crate) out_rx: Arc<Mutex<mpsc::UnboundedReceiver<SimpleOutboundRecord>>>,
            pub(crate) local_addr: Arc<Mutex<Option<SocketAddr>>>,
            pub(crate) shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
            pub(crate) client: reqwest::Client,
        }

        impl $platform {
            pub fn new(config: $config) -> Arc<Self> {
                let (out_tx, out_rx) = mpsc::unbounded_channel();
                Arc::new(Self {
                    config: $config {
                        name: if config.name.trim().is_empty() {
                            $default_name.to_string()
                        } else {
                            config.name
                        },
                        listen: if config.listen.trim().is_empty() {
                            $default_listen.to_string()
                        } else {
                            config.listen
                        },
                        callback_path: normalize_path(if config.callback_path.trim().is_empty() {
                            $default_path.to_string()
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
                    client: self.client.clone(),
                }
            }
        }
    };
}

webhook_platform_struct!(
    LinePlatformConfig,
    LinePlatform,
    "line",
    "127.0.0.1:18400",
    "/line/webhook"
);
macro_rules! polling_platform {
    ($platform:ident) => {
        pub struct $platform {
            pub(crate) config: PollPlatformConfig,
            pub(crate) handler: Arc<Mutex<Option<MessageHandler>>>,
            pub(crate) outbox: Arc<Mutex<Vec<SimpleOutboundRecord>>>,
            pub(crate) out_tx: mpsc::UnboundedSender<SimpleOutboundRecord>,
            pub(crate) out_rx: Arc<Mutex<mpsc::UnboundedReceiver<SimpleOutboundRecord>>>,
            pub(crate) shutdown: Arc<Mutex<Option<oneshot::Sender<()>>>>,
            pub(crate) local_addr: Arc<Mutex<Option<SocketAddr>>>,
            pub(crate) client: reqwest::Client,
        }

        impl $platform {
            pub fn new(config: PollPlatformConfig) -> Arc<Self> {
                let (out_tx, out_rx) = mpsc::unbounded_channel();
                Arc::new(Self {
                    config,
                    handler: Arc::new(Mutex::new(None)),
                    outbox: Arc::new(Mutex::new(Vec::new())),
                    out_tx,
                    out_rx: Arc::new(Mutex::new(out_rx)),
                    shutdown: Arc::new(Mutex::new(None)),
                    local_addr: Arc::new(Mutex::new(None)),
                    client: reqwest::Client::new(),
                })
            }

            pub async fn wait_for_outbound(&self) -> Option<SimpleOutboundRecord> {
                self.out_rx.lock().await.recv().await
            }

            pub async fn local_addr(&self) -> Option<SocketAddr> {
                *self.local_addr.lock().await
            }

            pub(crate) fn clone_for_task(&self) -> Arc<Self> {
                Arc::new(Self {
                    config: self.config.clone(),
                    handler: Arc::clone(&self.handler),
                    outbox: Arc::clone(&self.outbox),
                    out_tx: self.out_tx.clone(),
                    out_rx: Arc::clone(&self.out_rx),
                    shutdown: Arc::clone(&self.shutdown),
                    local_addr: Arc::clone(&self.local_addr),
                    client: self.client.clone(),
                })
            }
        }
    };
}

polling_platform!(MaxPlatform);
polling_platform!(WeixinPlatform);
polling_platform!(QqBotPlatform);
polling_platform!(WeiboPlatform);

macro_rules! has_outbox {
    ($platform:ty) => {
        impl HasOutbox for $platform {
            fn outbox(&self) -> &Arc<Mutex<Vec<SimpleOutboundRecord>>> {
                &self.outbox
            }

            fn out_tx(&self) -> &mpsc::UnboundedSender<SimpleOutboundRecord> {
                &self.out_tx
            }
        }
    };
}

has_outbox!(LinePlatform);
has_outbox!(MaxPlatform);
has_outbox!(WeixinPlatform);
has_outbox!(QqBotPlatform);
has_outbox!(WeiboPlatform);
