use super::{SlackPlatform, SlackReplyContext};
use crate::core::{Message, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

pub(crate) async fn run_slack_socket_mode(
    platform: Arc<SlackPlatform>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    loop {
        match run_slack_socket_once(Arc::clone(&platform), &mut shutdown_rx).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => tracing::warn!(error = %err, "slack socket mode disconnected"),
        }
        tokio::select! {
            _ = &mut shutdown_rx => return,
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
}

async fn run_slack_socket_once(
    platform: Arc<SlackPlatform>,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> Result<bool> {
    let socket_url = open_slack_socket(&platform).await?;
    let (stream, _) = connect_async(&socket_url).await?;
    tracing::info!("slack socket mode connected");
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
                match message? {
                    WsMessage::Text(text) => {
                        let value: Value = serde_json::from_str(&text)?;
                        if let Some(envelope_id) = value.get("envelope_id").and_then(Value::as_str) {
                            write.send(WsMessage::Text(json!({ "envelope_id": envelope_id }).to_string())).await?;
                        }
                        match slack_message_from_envelope(&platform, &value) {
                            Ok(Some(message)) => dispatch(platform.clone(), message).await,
                            Ok(None) => {}
                            Err(err) => tracing::warn!(error = %err, "slack event parse failed"),
                        }
                    }
                    WsMessage::Ping(bytes) => {
                        write.send(WsMessage::Pong(bytes)).await?;
                    }
                    WsMessage::Close(_) => return Ok(false),
                    _ => {}
                }
            }
        }
    }
}

async fn open_slack_socket(platform: &SlackPlatform) -> Result<String> {
    let value: Value = platform
        .client
        .post(format!(
            "{}/apps.connections.open",
            platform.config.api_base
        ))
        .bearer_auth(&platform.config.app_token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(anyhow!(
            "slack apps.connections.open failed: {}",
            value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ));
    }
    value
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("slack apps.connections.open missing url"))
}

fn slack_message_from_envelope(
    platform: &SlackPlatform,
    envelope: &Value,
) -> Result<Option<Message>> {
    if envelope.get("type").and_then(Value::as_str) != Some("events_api") {
        return Ok(None);
    }
    let payload = envelope
        .get("payload")
        .ok_or_else(|| anyhow!("missing payload"))?;
    let event = payload
        .get("event")
        .ok_or_else(|| anyhow!("missing event"))?;
    let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
    if event_type != "app_mention" && event_type != "message" {
        return Ok(None);
    }
    if event.get("bot_id").is_some() || event.get("subtype").is_some() {
        return Ok(None);
    }
    let content = event
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Ok(None);
    }
    let channel = event
        .get("channel")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing channel"))?
        .to_string();
    let user_id = event
        .get("user")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing user"))?
        .to_string();
    let ts = event
        .get("ts")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let thread_ts = event
        .get("thread_ts")
        .and_then(Value::as_str)
        .unwrap_or(&ts)
        .to_string();
    let session_key = if platform.config.share_session_in_channel {
        format!("{}:{}", platform.config.name, channel)
    } else {
        format!("{}:{}:{}", platform.config.name, channel, user_id)
    };
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.config.name.clone()),
        message_id: Some(ts.clone()),
        message_type: crate::core::MessageType::Text,
        user_id,
        user_name: None,
        content,
        attachments: Vec::new(),
        images: Vec::new(),
        files: Vec::new(),
        reply_ctx: ReplyContext {
            value: serde_json::to_string(&SlackReplyContext {
                channel,
                thread_ts: Some(thread_ts),
            })?,
        },
        created_at: std::time::SystemTime::now(),
    }))
}

async fn dispatch(platform: Arc<SlackPlatform>, message: Message) {
    let handler = platform.handler.lock().await.clone();
    if let Some(handler) = handler {
        let platform_dyn = platform as Arc<dyn Platform>;
        tokio::spawn(async move {
            handler(platform_dyn, message).await;
        });
    }
}
