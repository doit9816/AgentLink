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
use hmac::{Hmac, Mac};
pub(crate) use rand::Rng;
use serde::{Deserialize, Serialize};
pub(crate) use serde_json::{json, Value};
pub(crate) use sha1::{Digest as Sha1Digest, Sha1};
use sha2::Sha256;
use std::net::SocketAddr;
pub(crate) use std::collections::BTreeMap;
pub(crate) use std::sync::Arc;
pub(crate) use std::time::Duration;
pub(crate) use tokio::net::TcpListener;
use tokio::sync::mpsc;
pub(crate) use tokio::sync::{oneshot, Mutex};
pub(crate) use tokio_tungstenite::connect_async;
pub(crate) use tokio_tungstenite::tungstenite::client::IntoClientRequest;
pub(crate) use tokio_tungstenite::tungstenite::Message as WsMessage;

pub(crate) mod ws;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimpleOutboundRecord {
    pub channel: String,
    pub target: String,
    pub message_id: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SimpleReplyContext {
    pub(crate) channel: String,
    pub(crate) target: String,
    pub(crate) message_id: Option<String>,
    pub(crate) extra: Value,
}

macro_rules! webhook_platform_struct {
    ($config:ident, $platform:ident, $default_name:literal, $default_listen:literal, $default_path:literal) => {
        #[derive(Debug, Clone)]
        pub struct $config {
            pub name: String,
            pub listen: String,
            pub callback_path: String,
            pub secret: Option<String>,
            pub token: Option<String>,
            pub corp_id: Option<String>,
            pub corp_secret: Option<String>,
            pub agent_id: Option<String>,
            pub callback_token: Option<String>,
            pub callback_aes_key: Option<String>,
            pub api_base: String,
            pub share_session_in_channel: bool,
            pub dry_run: bool,
        }

        impl Default for $config {
            fn default() -> Self {
                Self {
                    name: $default_name.to_string(),
                    listen: $default_listen.to_string(),
                    callback_path: $default_path.to_string(),
                    secret: None,
                    token: None,
                    corp_id: None,
                    corp_secret: None,
                    agent_id: None,
                    callback_token: None,
                    callback_aes_key: None,
                    api_base: String::new(),
                    share_session_in_channel: false,
                    dry_run: false,
                }
            }
        }

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
webhook_platform_struct!(
    WeComPlatformConfig,
    WeComPlatform,
    "wecom",
    "127.0.0.1:18500",
    "/wecom/callback"
);

#[derive(Debug, Clone)]
pub struct PollPlatformConfig {
    pub name: String,
    pub app_id: Option<String>,
    pub app_secret: Option<String>,
    pub token_endpoint: Option<String>,
    pub token: String,
    pub api_base: String,
    pub ws_endpoint: Option<String>,
    pub poll_timeout_secs: u64,
    pub webhook_url: Option<String>,
    pub webhook_listen: String,
    pub webhook_path: String,
    pub webhook_secret: Option<String>,
    pub route_tag: Option<String>,
    pub account_id: Option<String>,
    pub allow_from: Option<String>,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

impl Default for PollPlatformConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            app_id: None,
            app_secret: None,
            token_endpoint: None,
            token: String::new(),
            api_base: String::new(),
            ws_endpoint: None,
            poll_timeout_secs: 30,
            webhook_url: None,
            webhook_listen: "127.0.0.1:18600".to_string(),
            webhook_path: "/max/webhook".to_string(),
            webhook_secret: None,
            route_tag: None,
            account_id: None,
            allow_from: None,
            share_session_in_channel: false,
            dry_run: false,
        }
    }
}

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

pub(crate) trait HasOutbox {
    fn outbox(&self) -> &Arc<Mutex<Vec<SimpleOutboundRecord>>>;
    fn out_tx(&self) -> &mpsc::UnboundedSender<SimpleOutboundRecord>;
}

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
has_outbox!(WeComPlatform);
has_outbox!(MaxPlatform);
has_outbox!(WeixinPlatform);
has_outbox!(QqBotPlatform);
has_outbox!(WeiboPlatform);

pub(crate) async fn dispatch_webhook(
    platform: Arc<dyn Platform>,
    handler_ref: &Arc<Mutex<Option<MessageHandler>>>,
    message: Message,
) {
    let handler = handler_ref.lock().await.clone();
    if let Some(handler) = handler {
        tokio::spawn(async move {
            handler(platform, message).await;
        });
    }
}

pub(crate) fn simple_message(
    platform: &str,
    session_key: String,
    user_id: String,
    message_id: Option<&str>,
    content: String,
    reply_ctx: SimpleReplyContext,
) -> Result<Option<Message>> {
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.to_string()),
        message_id: message_id.map(str::to_string),
        message_type: crate::core::MessageType::Text,
        user_id,
        user_name: None,
        content,
        attachments: Vec::new(),
        images: Vec::new(),
        files: Vec::new(),
        reply_ctx: ReplyContext {
            value: serde_json::to_string(&reply_ctx)?,
        },
        created_at: std::time::SystemTime::now(),
    }))
}

pub(crate) async fn emit_outbound<P>(platform: &P, ctx: &SimpleReplyContext, content: &str)
where
    P: HasOutbox + ?Sized,
{
    let record = SimpleOutboundRecord {
        channel: ctx.channel.clone(),
        target: ctx.target.clone(),
        message_id: ctx.message_id.clone(),
        content: content.to_string(),
    };
    platform.outbox().lock().await.push(record.clone());
    let _ = platform.out_tx().send(record);
}

pub(crate) fn verify_line_signature(secret: &str, headers: &HeaderMap, body: &[u8]) -> bool {
    let Some(actual) = headers
        .get("x-line-signature")
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let mut mac = match Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
        Ok(mac) => mac,
        Err(_) => return false,
    };
    mac.update(body);
    let expected = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    actual == expected
}

pub(crate) fn xml_tag(input: &str, tag: &str) -> Option<String> {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    let (_, rest) = input.split_once(&start)?;
    let (value, _) = rest.split_once(&end)?;
    Some(
        value
            .trim()
            .trim_start_matches("<![CDATA[")
            .trim_end_matches("]]>")
            .to_string(),
    )
}

pub(crate) fn value_to_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

pub(crate) fn normalize_path(path: String) -> String {
    if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    }
}

pub(crate) fn string_option(opts: &toml::value::Table, key: &str) -> Option<String> {
    opts.get(key)
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
}

pub(crate) fn bool_option(opts: &toml::value::Table, key: &str) -> Option<bool> {
    opts.get(key).and_then(|v| v.as_bool())
}

pub(crate) fn u64_option(opts: &toml::value::Table, key: &str) -> Option<u64> {
    opts.get(key).and_then(|v| v.as_integer()).map(|v| v as u64)
}

macro_rules! webhook_config_try_from {
    ($config:ident, $default_name:literal, $default_base:literal) => {
        impl TryFrom<toml::value::Table> for $config {
            type Error = anyhow::Error;

            fn try_from(opts: toml::value::Table) -> Result<Self> {
                Ok(Self {
                    name: string_option(&opts, "name").unwrap_or_else(|| $default_name.to_string()),
                    listen: string_option(&opts, "listen")
                        .unwrap_or_else(|| Self::default().listen),
                    callback_path: string_option(&opts, "callback_path")
                        .unwrap_or_else(|| Self::default().callback_path),
                    secret: string_option(&opts, "secret")
                        .or_else(|| string_option(&opts, "channel_secret"))
                        .or_else(|| string_option(&opts, "corp_id")),
                    token: string_option(&opts, "token")
                        .or_else(|| string_option(&opts, "channel_token"))
                        .or_else(|| string_option(&opts, "corp_secret"))
                        .or_else(|| string_option(&opts, "agent_id")),
                    corp_id: string_option(&opts, "corp_id"),
                    corp_secret: string_option(&opts, "corp_secret"),
                    agent_id: string_option(&opts, "agent_id"),
                    callback_token: string_option(&opts, "callback_token"),
                    callback_aes_key: string_option(&opts, "callback_aes_key"),
                    api_base: string_option(&opts, "api_base")
                        .or_else(|| string_option(&opts, "api_base_url"))
                        .unwrap_or_else(|| $default_base.to_string()),
                    share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                        .unwrap_or(false),
                    dry_run: bool_option(&opts, "dry_run").unwrap_or(false),
                })
            }
        }
    };
}

webhook_config_try_from!(LinePlatformConfig, "line", "https://api.line.me");
webhook_config_try_from!(WeComPlatformConfig, "wecom", "https://qyapi.weixin.qq.com");

impl TryFrom<toml::value::Table> for PollPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "poll".to_string()),
            token: string_option(&opts, "token")
                .or_else(|| string_option(&opts, "app_token"))
                .unwrap_or_default(),
            app_id: string_option(&opts, "app_id"),
            app_secret: string_option(&opts, "app_secret"),
            token_endpoint: string_option(&opts, "token_endpoint"),
            api_base: string_option(&opts, "api_base")
                .or_else(|| string_option(&opts, "base_url"))
                .unwrap_or_default(),
            ws_endpoint: string_option(&opts, "ws_endpoint")
                .or_else(|| string_option(&opts, "gateway_url")),
            poll_timeout_secs: u64_option(&opts, "poll_timeout_secs").unwrap_or(30),
            webhook_url: string_option(&opts, "webhook_url"),
            webhook_listen: string_option(&opts, "webhook_listen")
                .unwrap_or_else(|| "127.0.0.1:18600".to_string()),
            webhook_path: string_option(&opts, "webhook_path")
                .unwrap_or_else(|| "/max/webhook".to_string()),
            webhook_secret: string_option(&opts, "webhook_secret"),
            route_tag: string_option(&opts, "route_tag"),
            account_id: string_option(&opts, "account_id"),
            allow_from: string_option(&opts, "allow_from"),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run: bool_option(&opts, "dry_run").unwrap_or(false),
        })
    }
}

pub fn max_config_from_options(opts: toml::value::Table) -> Result<PollPlatformConfig> {
    let mut cfg = PollPlatformConfig::try_from(opts)?;
    if cfg.name == "poll" {
        cfg.name = "max".to_string();
    }
    if cfg.api_base.is_empty() {
        cfg.api_base = "https://platform-api.max.ru".to_string();
    }
    Ok(cfg)
}

pub fn weixin_config_from_options(opts: toml::value::Table) -> Result<PollPlatformConfig> {
    let mut cfg = PollPlatformConfig::try_from(opts)?;
    if cfg.name == "poll" {
        cfg.name = "weixin".to_string();
    }
    if cfg.api_base.is_empty() {
        cfg.api_base = "https://ilinkai.weixin.qq.com".to_string();
    }
    Ok(cfg)
}

pub fn qqbot_config_from_options(opts: toml::value::Table) -> Result<PollPlatformConfig> {
    let mut cfg = PollPlatformConfig::try_from(opts.clone())?;
    if cfg.name == "poll" {
        cfg.name = "qqbot".to_string();
    }
    if cfg.api_base.is_empty() {
        cfg.api_base = "https://api.sgroup.qq.com".to_string();
    }
    if cfg.ws_endpoint.is_none() {
        cfg.ws_endpoint = string_option(&opts, "gateway_url");
    }
    Ok(cfg)
}

pub fn weibo_config_from_options(opts: toml::value::Table) -> Result<PollPlatformConfig> {
    let mut cfg = PollPlatformConfig::try_from(opts.clone())?;
    if cfg.name == "poll" {
        cfg.name = "weibo".to_string();
    }
    if cfg.api_base.is_empty() {
        cfg.api_base = "https://open-im.api.weibo.com".to_string();
    }
    if cfg.ws_endpoint.is_none() {
        cfg.ws_endpoint = string_option(&opts, "ws_endpoint");
    }
    Ok(cfg)
}
