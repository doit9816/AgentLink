use super::BridgeReplyContext;
use crate::core::{
    Attachment, FileAttachment, ImageAttachment, Message, MessageType, ReplyContext,
};
use anyhow::{anyhow, Result};
use axum::http::HeaderMap;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::net::SocketAddr;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisterMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    pub platform: String,
    pub project: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InboundMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    pub project: Option<String>,
    pub msg_id: Option<String>,
    pub session_key: String,
    pub user_id: String,
    pub user_name: Option<String>,
    #[serde(default, rename = "message_type")]
    pub content_type: MessageType,
    pub content: String,
    pub reply_ctx: Option<String>,
    #[serde(default)]
    pub attachments: Vec<BridgeAttachment>,
    #[serde(default)]
    pub images: Vec<BridgeBinaryData>,
    #[serde(default)]
    pub files: Vec<BridgeBinaryData>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeBinaryData {
    pub mime_type: String,
    pub data: String,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BridgeAttachment {
    pub kind: crate::core::AttachmentKind,
    pub mime_type: Option<String>,
    pub data: Option<String>,
    pub url: Option<String>,
    pub path: Option<String>,
    pub file_name: Option<String>,
    pub text: Option<String>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplyMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    pub project: Option<String>,
    pub session_key: String,
    pub reply_ctx: Option<String>,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegisterAckMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    pub ok: bool,
    pub error: String,
}

impl InboundMessage {
    pub fn into_core(self, platform: &str, adapter: &str) -> Result<Message> {
        let reply_ctx = BridgeReplyContext {
            platform: adapter.to_string(),
            session_key: self.session_key.clone(),
            reply_ctx: self.reply_ctx.unwrap_or_default(),
        };
        Ok(Message {
            project: self.project,
            session_key: self.session_key,
            platform: Some(platform.to_string()),
            message_id: self.msg_id,
            message_type: self.content_type,
            user_id: self.user_id,
            user_name: self.user_name,
            content: self.content,
            attachments: decode_attachments(self.attachments)?,
            images: decode_images(self.images)?,
            files: decode_files(self.files)?,
            reply_ctx: ReplyContext {
                value: serde_json::to_string(&reply_ctx)?,
            },
            created_at: std::time::SystemTime::now(),
        })
    }
}

fn decode_attachments(values: Vec<BridgeAttachment>) -> Result<Vec<Attachment>> {
    values
        .into_iter()
        .map(|value| {
            Ok(Attachment {
                kind: value.kind,
                mime_type: value.mime_type,
                data: value.data.map(|data| STANDARD.decode(data)).transpose()?,
                url: value.url,
                path: value.path,
                file_name: value.file_name,
                text: value.text,
                metadata: value.metadata,
            })
        })
        .collect()
}

fn decode_images(values: Vec<BridgeBinaryData>) -> Result<Vec<ImageAttachment>> {
    values
        .into_iter()
        .map(|value| {
            Ok(ImageAttachment {
                mime_type: value.mime_type,
                data: STANDARD.decode(value.data)?,
                file_name: value.file_name,
            })
        })
        .collect()
}

fn decode_files(values: Vec<BridgeBinaryData>) -> Result<Vec<FileAttachment>> {
    values
        .into_iter()
        .map(|value| {
            Ok(FileAttachment {
                mime_type: value.mime_type,
                data: STANDARD.decode(value.data)?,
                file_name: value.file_name,
            })
        })
        .collect()
}

pub(crate) fn parse_reply_context(value: &str) -> Result<BridgeReplyContext> {
    if let Ok(ctx) = serde_json::from_str::<BridgeReplyContext>(value) {
        if !ctx.platform.trim().is_empty() {
            return Ok(ctx);
        }
    }
    Err(anyhow!(
        "bridge reply context is invalid; expected JSON with platform/session_key/reply_ctx"
    ))
}

pub(crate) fn send_ws_json<T: Serialize>(
    tx: &tokio::sync::mpsc::UnboundedSender<String>,
    value: &T,
) -> Result<()> {
    tx.send(serde_json::to_string(value)?)
        .map_err(|_| anyhow!("websocket writer is closed"))
}

pub(crate) fn normalize_path(path: String) -> String {
    if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    }
}

fn string_option(opts: &toml::value::Table, key: &str) -> Option<String> {
    opts.get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

fn bool_option(opts: &toml::value::Table, key: &str) -> Option<bool> {
    opts.get(key).and_then(|value| value.as_bool())
}

pub(crate) fn header_matches(headers: &HeaderMap, name: &str, expected: &str) -> bool {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| constant_time_eq(value, expected))
}

pub(crate) fn constant_time_eq(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .as_bytes()
            .iter()
            .zip(right.as_bytes())
            .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

impl TryFrom<toml::value::Table> for super::BridgePlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let listen = string_option(&opts, "listen").unwrap_or_else(|| "127.0.0.1:9810".to_string());
        if listen.parse::<SocketAddr>().is_err() {
            return Err(anyhow!(
                "bridge platform listen must be a socket address, got `{listen}`"
            ));
        }
        let path = normalize_path(
            string_option(&opts, "path").unwrap_or_else(|| "/bridge/ws".to_string()),
        );
        let insecure = bool_option(&opts, "insecure").unwrap_or(false);
        let token = string_option(&opts, "token");
        if token.as_deref().unwrap_or_default().is_empty() && !insecure {
            return Err(anyhow!(
                "bridge platform requires options.token, or options.insecure = true for local development"
            ));
        }
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "bridge".to_string()),
            listen,
            path,
            token,
            insecure,
        })
    }
}
