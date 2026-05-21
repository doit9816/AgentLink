use super::{DingTalkPlatform, DingTalkReplyContext};
use crate::core::{Message, MessageType, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

pub(super) async fn dingtalk_health() -> Json<Value> {
    Json(json!({ "ok": true }))
}

pub(super) async fn dingtalk_webhook(
    State(platform): State<Arc<DingTalkPlatform>>,
    Json(payload): Json<Value>,
) -> Response {
    match dingtalk_message_from_payload(&platform, &payload) {
        Ok(Some(message)) => {
            let handler = platform.handler.lock().await.clone();
            if let Some(handler) = handler {
                let platform_dyn = Arc::clone(&platform) as Arc<dyn Platform>;
                tokio::spawn(async move {
                    handler(platform_dyn, message).await;
                });
            }
            Json(json!({ "success": true })).into_response()
        }
        Ok(None) => Json(json!({ "success": true, "ignored": true })).into_response(),
        Err(err) => {
            tracing::warn!(error = %err, "dingtalk webhook parse failed");
            (StatusCode::BAD_REQUEST, err.to_string()).into_response()
        }
    }
}

fn dingtalk_message_from_payload(
    platform: &DingTalkPlatform,
    payload: &Value,
) -> Result<Option<Message>> {
    let msg_type = payload
        .get("msgtype")
        .or_else(|| payload.get("msgType"))
        .and_then(Value::as_str)
        .unwrap_or("text");
    if msg_type != "text" && msg_type != "richText" {
        return Ok(None);
    }
    let content = if msg_type == "richText" {
        extract_rich_text(payload.get("content").unwrap_or(&Value::Null))
    } else {
        payload
            .pointer("/text/content")
            .or_else(|| payload.pointer("/text/text"))
            .or_else(|| payload.get("content"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    if content.is_empty() {
        return Ok(None);
    }
    let conversation_id = payload
        .get("conversationId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing conversationId"))?
        .to_string();
    let sender_staff_id = payload
        .get("senderStaffId")
        .or_else(|| payload.get("senderId"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let conv_type = if payload
        .get("conversationType")
        .and_then(Value::as_str)
        .unwrap_or("1")
        == "2"
    {
        "g"
    } else {
        "d"
    };
    let session_key = if platform.config.share_session_in_channel {
        format!("{}:{}:{}", platform.config.name, conv_type, conversation_id)
    } else {
        format!(
            "{}:{}:{}:{}",
            platform.config.name, conv_type, conversation_id, sender_staff_id
        )
    };
    let reply_ctx = DingTalkReplyContext {
        session_webhook: payload
            .get("sessionWebhook")
            .and_then(Value::as_str)
            .map(str::to_string),
        conversation_id: conversation_id.clone(),
        sender_staff_id: sender_staff_id.clone(),
    };
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.config.name.clone()),
        message_id: payload
            .get("msgId")
            .or_else(|| payload.get("msgid"))
            .and_then(Value::as_str)
            .map(str::to_string),
        message_type: MessageType::Text,
        user_id: sender_staff_id,
        user_name: payload
            .get("senderNick")
            .and_then(Value::as_str)
            .map(str::to_string),
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

fn extract_rich_text(value: &Value) -> String {
    value
        .get("richText")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<String>()
        })
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub(super) fn preprocess_markdown(content: &str) -> String {
    content.replace("\n\n", "\n\n")
}

pub(super) fn normalize_path(path: String) -> String {
    if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    }
}
