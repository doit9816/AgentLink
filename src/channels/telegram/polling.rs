use super::{TelegramPlatform, TelegramReplyContext};
use crate::core::{Message, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;

pub(crate) async fn run_telegram_polling(
    platform: Arc<TelegramPlatform>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let mut offset: Option<i64> = None;
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => return,
            result = fetch_telegram_updates(&platform, offset) => {
                match result {
                    Ok(updates) => {
                        for update in updates {
                            if let Some(update_id) = update.get("update_id").and_then(Value::as_i64) {
                                offset = Some(update_id + 1);
                            }
                            match telegram_message_from_update(&platform, &update) {
                                Ok(Some(message)) => dispatch(platform.clone(), message).await,
                                Ok(None) => {}
                                Err(err) => tracing::warn!(error = %err, "telegram update parse failed"),
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "telegram getUpdates failed");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

async fn fetch_telegram_updates(
    platform: &TelegramPlatform,
    offset: Option<i64>,
) -> Result<Vec<Value>> {
    let mut body = json!({
        "timeout": platform.config.poll_timeout_secs,
        "allowed_updates": ["message"]
    });
    if let Some(offset) = offset {
        body["offset"] = json!(offset);
    }
    let value: Value = platform
        .client
        .post(platform.method_url("getUpdates"))
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(anyhow!("telegram getUpdates returned ok=false"));
    }
    Ok(value
        .get("result")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

fn telegram_message_from_update(
    platform: &TelegramPlatform,
    update: &Value,
) -> Result<Option<Message>> {
    let message = update
        .get("message")
        .or_else(|| update.get("edited_message"))
        .ok_or_else(|| anyhow!("missing message"))?;
    let content = message
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Ok(None);
    }
    let chat = message
        .get("chat")
        .ok_or_else(|| anyhow!("missing message.chat"))?;
    let chat_id = chat
        .get("id")
        .map(value_to_string)
        .ok_or_else(|| anyhow!("missing chat.id"))?;
    let user = message
        .get("from")
        .ok_or_else(|| anyhow!("missing message.from"))?;
    if user.get("is_bot").and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let user_id = user
        .get("id")
        .map(value_to_string)
        .ok_or_else(|| anyhow!("missing from.id"))?;
    let message_id = message.get("message_id").and_then(Value::as_i64);
    let session_key = if platform.config.share_session_in_channel {
        format!("{}:{}", platform.config.name, chat_id)
    } else {
        format!("{}:{}:{}", platform.config.name, chat_id, user_id)
    };
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.config.name.clone()),
        message_id: message_id.map(|v| v.to_string()),
        message_type: crate::core::MessageType::Text,
        user_id,
        user_name: user
            .get("username")
            .or_else(|| user.get("first_name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        content,
        attachments: Vec::new(),
        images: Vec::new(),
        files: Vec::new(),
        reply_ctx: ReplyContext {
            value: serde_json::to_string(&TelegramReplyContext {
                chat_id,
                message_id,
            })?,
        },
        created_at: std::time::SystemTime::now(),
    }))
}

async fn dispatch(platform: Arc<TelegramPlatform>, message: Message) {
    let handler = platform.handler.lock().await.clone();
    if let Some(handler) = handler {
        let platform_dyn = platform as Arc<dyn Platform>;
        tokio::spawn(async move {
            handler(platform_dyn, message).await;
        });
    }
}

fn value_to_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}
