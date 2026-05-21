use super::{DiscordPlatform, DiscordReplyContext};
use crate::core::{Message, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

pub(crate) async fn run_discord_gateway(
    platform: Arc<DiscordPlatform>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    loop {
        match run_discord_gateway_once(Arc::clone(&platform), &mut shutdown_rx).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => tracing::warn!(error = %err, "discord gateway disconnected"),
        }
        tokio::select! {
            _ = &mut shutdown_rx => return,
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
}

async fn run_discord_gateway_once(
    platform: Arc<DiscordPlatform>,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> Result<bool> {
    let gateway_url = discord_gateway_url(&platform).await?;
    let url = if gateway_url.contains('?') {
        gateway_url
    } else {
        format!("{gateway_url}?v=10&encoding=json")
    };
    let (stream, _) = connect_async(&url).await?;
    tracing::info!("discord gateway connected");
    let (mut write, mut read) = stream.split();
    let mut heartbeat_interval: Option<tokio::time::Interval> = None;
    let mut last_sequence: Option<i64> = None;

    loop {
        tokio::select! {
            _ = &mut *shutdown_rx => {
                let _ = write.close().await;
                return Ok(true);
            }
            _ = async {
                if let Some(interval) = &mut heartbeat_interval {
                    interval.tick().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                write.send(WsMessage::Text(json!({ "op": 1, "d": last_sequence }).to_string())).await?;
            }
            message = read.next() => {
                let Some(message) = message else {
                    return Ok(false);
                };
                match message? {
                    WsMessage::Text(text) => {
                        let value: Value = serde_json::from_str(&text)?;
                        if let Some(seq) = value.get("s").and_then(Value::as_i64) {
                            last_sequence = Some(seq);
                        }
                        match value.get("op").and_then(Value::as_i64) {
                            Some(10) => {
                                let interval_ms = value.pointer("/d/heartbeat_interval").and_then(Value::as_u64).unwrap_or(45_000);
                                heartbeat_interval = Some(tokio::time::interval(Duration::from_millis(interval_ms)));
                                let identify = json!({
                                    "op": 2,
                                    "d": {
                                        "token": platform.config.token,
                                        "intents": 33280,
                                        "properties": {
                                            "os": std::env::consts::OS,
                                            "browser": "agentlink",
                                            "device": "agentlink"
                                        }
                                    }
                                });
                                write.send(WsMessage::Text(identify.to_string())).await?;
                            }
                            Some(0) => {
                                if value.get("t").and_then(Value::as_str) == Some("MESSAGE_CREATE") {
                                    match discord_message_from_dispatch(&platform, value.get("d").unwrap_or(&Value::Null)) {
                                        Ok(Some(message)) => dispatch(platform.clone(), message).await,
                                        Ok(None) => {}
                                        Err(err) => tracing::warn!(error = %err, "discord message parse failed"),
                                    }
                                }
                            }
                            _ => {}
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

async fn discord_gateway_url(platform: &DiscordPlatform) -> Result<String> {
    if let Some(url) = &platform.config.gateway_url {
        return Ok(url.clone());
    }
    let value: Value = platform
        .client
        .get(format!("{}/gateway/bot", platform.config.api_base))
        .bearer_auth(&platform.config.token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    value
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("discord gateway response missing url"))
}

fn discord_message_from_dispatch(
    platform: &DiscordPlatform,
    event: &Value,
) -> Result<Option<Message>> {
    let author = event
        .get("author")
        .ok_or_else(|| anyhow!("missing author"))?;
    if author.get("bot").and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let content = event
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Ok(None);
    }
    let channel_id = event
        .get("channel_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing channel_id"))?
        .to_string();
    let user_id = author
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing author.id"))?
        .to_string();
    let message_id = event.get("id").and_then(Value::as_str).map(str::to_string);
    let session_key = if platform.config.share_session_in_channel {
        format!("{}:{}", platform.config.name, channel_id)
    } else {
        format!("{}:{}:{}", platform.config.name, channel_id, user_id)
    };
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.config.name.clone()),
        message_id: message_id.clone(),
        message_type: crate::core::MessageType::Text,
        user_id,
        user_name: author
            .get("username")
            .and_then(Value::as_str)
            .map(str::to_string),
        content,
        attachments: Vec::new(),
        images: Vec::new(),
        files: Vec::new(),
        reply_ctx: ReplyContext {
            value: serde_json::to_string(&DiscordReplyContext {
                channel_id,
                message_id,
            })?,
        },
        created_at: std::time::SystemTime::now(),
    }))
}

async fn dispatch(platform: Arc<DiscordPlatform>, message: Message) {
    let handler = platform.handler.lock().await.clone();
    if let Some(handler) = handler {
        let platform_dyn = platform as Arc<dyn Platform>;
        tokio::spawn(async move {
            handler(platform_dyn, message).await;
        });
    }
}
