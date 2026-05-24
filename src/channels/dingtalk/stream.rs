use super::{DingTalkPlatform, DingTalkReplyContext};
use crate::core::{Message, MessageType, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

const DINGTALK_BOT_CALLBACK_TOPIC: &str = "/v1.0/im/bot/messages/get";
const GATEWAY_OPEN_URL: &str = "https://api.dingtalk.com/v1.0/gateway/connections/open";

#[derive(serde::Deserialize)]
struct GatewayResponse {
    endpoint: String,
    ticket: String,
}

pub(super) async fn run(platform: Arc<DingTalkPlatform>, mut shutdown_rx: oneshot::Receiver<()>) {
    loop {
        match run_once(Arc::clone(&platform), &mut shutdown_rx).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => {
                tracing::warn!(error = %err, "dingtalk stream disconnected");
            }
        }
        tokio::select! {
            _ = &mut shutdown_rx => return,
            _ = tokio::time::sleep(Duration::from_secs(15)) => {}
        }
    }
}

/// Returns `true` when shutdown was requested.
async fn run_once(
    platform: Arc<DingTalkPlatform>,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> Result<bool> {
    let gateway = register_connection(&platform).await?;
    let ws_url = format!("{}?ticket={}", gateway.endpoint, gateway.ticket);
    tracing::info!("dingtalk stream connecting");
    let (stream, _) = connect_async(&ws_url).await?;
    tracing::info!("dingtalk stream connected");
    let (mut write, mut read) = stream.split();

    loop {
        tokio::select! {
            _ = &mut *shutdown_rx => {
                let _ = write.close().await;
                return Ok(true);
            }
            message = read.next() => {
                let Some(message) = message else {
                    return Ok(false);
                };
                let text = match message? {
                    WsMessage::Text(text) => text,
                    WsMessage::Close(_) => return Ok(false),
                    _ => continue,
                };
                let frame: Value = match serde_json::from_str(text.as_ref()) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let frame_type = frame.get("type").and_then(Value::as_str).unwrap_or("");
                match frame_type {
                    "SYSTEM" => {
                        if let Some(pong) = stream_ack(&frame) {
                            write.send(WsMessage::Text(pong.into())).await?;
                        }
                    }
                    "EVENT" | "CALLBACK" => {
                        if let Some(data) = parse_stream_data(&frame) {
                            if let Some(message) = dingtalk_message_from_stream_data(&platform, &data)? {
                                let handler = platform.handler.lock().await.clone();
                                if let Some(handler) = handler {
                                    let platform_dyn = Arc::clone(&platform) as Arc<dyn Platform>;
                                    tokio::spawn(async move {
                                        handler(platform_dyn, message).await;
                                    });
                                }
                            }
                        }
                        if let Some(ack) = stream_ack(&frame) {
                            write.send(WsMessage::Text(ack.into())).await?;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn register_connection(platform: &DingTalkPlatform) -> Result<GatewayResponse> {
    let body = serde_json::json!({
        "clientId": platform.config.client_id,
        "clientSecret": platform.config.client_secret,
        "subscriptions": [{
            "type": "CALLBACK",
            "topic": DINGTALK_BOT_CALLBACK_TOPIC,
        }],
    });
    let response = platform
        .client
        .post(GATEWAY_OPEN_URL)
        .json(&body)
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "dingtalk gateway registration failed ({status}): {body}"
        ));
    }
    Ok(response.json().await?)
}

fn stream_ack(frame: &Value) -> Option<String> {
    let message_id = frame
        .get("headers")
        .and_then(|headers| headers.get("messageId"))
        .and_then(Value::as_str)
        .unwrap_or("");
    Some(
        serde_json::json!({
            "code": 200,
            "headers": {
                "contentType": "application/json",
                "messageId": message_id,
            },
            "message": "OK",
            "data": "",
        })
        .to_string(),
    )
}

pub(super) fn parse_stream_data(frame: &Value) -> Option<Value> {
    match frame.get("data") {
        Some(Value::String(raw)) => serde_json::from_str(raw).ok(),
        Some(value @ Value::Object(_)) => Some(value.clone()),
        _ => None,
    }
}

pub(super) fn dingtalk_message_from_stream_data(
    platform: &DingTalkPlatform,
    data: &Value,
) -> Result<Option<Message>> {
    let content = data
        .get("text")
        .and_then(|text| text.get("content"))
        .and_then(Value::as_str)
        .or_else(|| {
            data.pointer("/text/content")
                .or_else(|| data.get("content"))
                .and_then(Value::as_str)
        })
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Ok(None);
    }
    let conversation_id = data
        .get("conversationId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing conversationId"))?
        .to_string();
    let sender_staff_id = data
        .get("senderStaffId")
        .or_else(|| data.get("senderId"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let conv_type = if data
        .get("conversationType")
        .and_then(|value| {
            value
                .as_str()
                .map(|text| text == "2")
                .or_else(|| value.as_i64().map(|number| number == 2))
        })
        .unwrap_or(false)
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
        session_webhook: data
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
        message_id: data
            .get("msgId")
            .or_else(|| data.get("msgid"))
            .and_then(Value::as_str)
            .map(str::to_string),
        message_type: MessageType::Text,
        user_id: sender_staff_id,
        user_name: data
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::dingtalk::{DingTalkPlatform, DingTalkPlatformConfig};

    #[test]
    fn parse_stream_data_supports_string_payload() {
        let frame = serde_json::json!({
            "data": "{\"text\":{\"content\":\"hello\"},\"conversationId\":\"c1\",\"senderStaffId\":\"s1\"}"
        });
        let parsed = parse_stream_data(&frame).expect("payload");
        assert_eq!(
            parsed.pointer("/text/content").and_then(Value::as_str),
            Some("hello")
        );
    }

    #[test]
    fn stream_data_builds_message_with_session_webhook() {
        let platform = DingTalkPlatform::new(DingTalkPlatformConfig {
            name: "dingtalk".to_string(),
            dry_run: true,
            ..DingTalkPlatformConfig::default()
        });
        let data = serde_json::json!({
            "text": { "content": "hello stream" },
            "conversationId": "cid_1",
            "conversationType": 2,
            "senderStaffId": "staff_1",
            "senderNick": "Tester",
            "sessionWebhook": "https://example.invalid/session-webhook",
            "msgId": "m1"
        });
        let message = dingtalk_message_from_stream_data(&platform, &data)
            .expect("parse")
            .expect("message");
        assert_eq!(message.content, "hello stream");
        assert_eq!(message.user_id, "staff_1");
        assert!(message.reply_ctx.value.contains("session-webhook"));
    }
}
