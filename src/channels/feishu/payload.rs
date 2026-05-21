use super::{FeishuPlatform, FeishuReplyContext};
use crate::core::{Message, MessageType, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::sync::Arc;

pub(super) async fn dispatch(
    platform: Arc<FeishuPlatform>,
    payload: &Value,
) -> Result<Option<bool>> {
    match message_from_payload(&platform, payload).await? {
        Some(message) => {
            let handler = platform.handler.lock().await.clone();
            if let Some(handler) = handler {
                let platform_dyn = Arc::clone(&platform) as Arc<dyn Platform>;
                tokio::spawn(async move {
                    handler(platform_dyn, message).await;
                });
            }
            Ok(Some(true))
        }
        None => Ok(None),
    }
}

async fn message_from_payload(
    platform: &FeishuPlatform,
    payload: &Value,
) -> Result<Option<Message>> {
    let event = payload.get("event").unwrap_or(payload);
    let Some(message) = event.get("message") else {
        let event_type = payload
            .get("header")
            .and_then(|header| header.get("event_type"))
            .and_then(Value::as_str)
            .or_else(|| event.get("type").and_then(Value::as_str))
            .unwrap_or("unknown");
        tracing::debug!(event_type = %event_type, "feishu non-message event ignored");
        return Ok(None);
    };
    let msg_type = message
        .get("message_type")
        .and_then(Value::as_str)
        .unwrap_or("text");
    if msg_type != "text" {
        return Ok(None);
    }
    let content_raw = message
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing message.content"))?;
    let content_value: Value = serde_json::from_str(content_raw)?;
    let content = content_value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Ok(None);
    }

    let chat_id = message
        .get("chat_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing message.chat_id"))?
        .to_string();
    let message_id = message
        .get("message_id")
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let chat_type = message
        .get("chat_type")
        .and_then(Value::as_str)
        .unwrap_or("p2p");
    let sender = event.get("sender").unwrap_or(&Value::Null);
    let sender_id = sender.get("sender_id").unwrap_or(sender);
    let user_id = sender_id
        .get("open_id")
        .or_else(|| sender_id.get("user_id"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let scope = if chat_type == "group" { "g" } else { "p" };
    let session_key = if platform.config.share_session_in_channel {
        format!("{}:{}:{}", platform.config.name, scope, chat_id)
    } else {
        format!("{}:{}:{}:{}", platform.config.name, scope, chat_id, user_id)
    };
    let reply_ctx = FeishuReplyContext {
        message_id,
        chat_id,
    };
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.config.name.clone()),
        message_id: message
            .get("message_id")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        message_type: MessageType::Text,
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
