use super::{QqPlatform, QqReplyContext};
use crate::core::{Message, Platform, ReplyContext};
use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMessage;

pub(crate) async fn run_qq_websocket(
    platform: Arc<QqPlatform>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    loop {
        match run_qq_websocket_once(Arc::clone(&platform), &mut shutdown_rx).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => tracing::warn!(error = %err, "qq websocket disconnected"),
        }
        tokio::select! {
            _ = &mut shutdown_rx => return,
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
}

async fn run_qq_websocket_once(
    platform: Arc<QqPlatform>,
    shutdown_rx: &mut oneshot::Receiver<()>,
) -> Result<bool> {
    let mut request = platform.config.ws_url.clone().into_client_request()?;
    if let Some(token) = &platform.config.token {
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {token}").parse()?);
    }
    let (stream, _) = connect_async(request).await?;
    let (mut write, mut read) = stream.split();
    let (write_tx, mut write_rx) = mpsc::unbounded_channel::<Value>();
    *platform.write_tx.lock().await = Some(write_tx);
    tracing::info!("qq websocket connected");

    loop {
        tokio::select! {
            _ = &mut *shutdown_rx => {
                *platform.write_tx.lock().await = None;
                let _ = write.close().await;
                return Ok(true);
            }
            Some(payload) = write_rx.recv() => {
                write.send(WsMessage::Text(payload.to_string())).await?;
            }
            message = read.next() => {
                let Some(message) = message else {
                    *platform.write_tx.lock().await = None;
                    return Ok(false);
                };
                match message? {
                    WsMessage::Text(text) => {
                        let value: Value = serde_json::from_str(&text)?;
                        if value.get("echo").is_some() {
                            continue;
                        }
                        match qq_message_from_payload(&platform, &value) {
                            Ok(Some(message)) => dispatch(platform.clone(), message).await,
                            Ok(None) => {}
                            Err(err) => tracing::warn!(error = %err, "qq message parse failed"),
                        }
                    }
                    WsMessage::Ping(bytes) => {
                        write.send(WsMessage::Pong(bytes)).await?;
                    }
                    WsMessage::Close(_) => {
                        *platform.write_tx.lock().await = None;
                        return Ok(false);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn qq_message_from_payload(platform: &QqPlatform, payload: &Value) -> Result<Option<Message>> {
    if payload.get("post_type").and_then(Value::as_str) != Some("message") {
        return Ok(None);
    }
    let message_type = payload
        .get("message_type")
        .and_then(Value::as_str)
        .unwrap_or("private")
        .to_string();
    let user_id = payload.get("user_id").and_then(Value::as_i64);
    let group_id = payload.get("group_id").and_then(Value::as_i64);
    let message_id = payload.get("message_id").and_then(Value::as_i64);
    let content = payload
        .get("raw_message")
        .or_else(|| payload.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Ok(None);
    }
    let user_id = user_id.ok_or_else(|| anyhow!("qq message missing user_id"))?;
    let session_key = if message_type == "group" {
        let group_id = group_id.ok_or_else(|| anyhow!("qq group message missing group_id"))?;
        if platform.config.share_session_in_channel {
            format!("{}:g:{}", platform.config.name, group_id)
        } else {
            format!("{}:{}:{}", platform.config.name, group_id, user_id)
        }
    } else {
        format!("{}:{}", platform.config.name, user_id)
    };
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.config.name.clone()),
        message_id: message_id.map(|v| v.to_string()),
        message_type: crate::core::MessageType::Text,
        user_id: user_id.to_string(),
        user_name: payload
            .pointer("/sender/card")
            .or_else(|| payload.pointer("/sender/nickname"))
            .and_then(Value::as_str)
            .map(str::to_string),
        content,
        attachments: Vec::new(),
        images: Vec::new(),
        files: Vec::new(),
        reply_ctx: ReplyContext {
            value: serde_json::to_string(&QqReplyContext {
                message_type,
                user_id: Some(user_id),
                group_id,
                message_id,
            })?,
        },
        created_at: std::time::SystemTime::now(),
    }))
}

async fn dispatch(platform: Arc<QqPlatform>, message: Message) {
    let handler = platform.handler.lock().await.clone();
    if let Some(handler) = handler {
        let platform_dyn = platform as Arc<dyn Platform>;
        tokio::spawn(async move {
            handler(platform_dyn, message).await;
        });
    }
}
