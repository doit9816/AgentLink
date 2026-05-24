use super::{WeComBotReplyContext, WeComPlatform};
use crate::channels::support::{dispatch_webhook, simple_message, SimpleReplyContext};
use crate::core::{MessageHandler, Platform};
use anyhow::{anyhow, Result};
use futures_util::stream::SplitStream;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;

const DEFAULT_WECOM_WS_URL: &str = "wss://openws.work.weixin.qq.com";

pub(super) async fn run(
    platform: Arc<WeComPlatform>,
    mut shutdown_rx: oneshot::Receiver<()>,
    mut ws_out_rx: tokio::sync::mpsc::UnboundedReceiver<String>,
) {
    loop {
        match run_once(Arc::clone(&platform), &mut shutdown_rx, &mut ws_out_rx).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(err) => tracing::warn!(error = %err, "wecom stream disconnected"),
        }
        tokio::select! {
            _ = &mut shutdown_rx => return,
            _ = tokio::time::sleep(Duration::from_secs(15)) => {}
        }
    }
}

async fn run_once(
    platform: Arc<WeComPlatform>,
    shutdown_rx: &mut oneshot::Receiver<()>,
    ws_out_rx: &mut tokio::sync::mpsc::UnboundedReceiver<String>,
) -> Result<bool> {
    let bot_id = platform
        .config
        .bot_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("wecom bot_id is required for websocket mode"))?;
    let bot_secret = platform
        .config
        .bot_secret
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("wecom bot_secret is required for websocket mode"))?;
    let ws_url = platform.config.websocket_url.trim().to_string();
    let ws_url = if ws_url.is_empty() {
        DEFAULT_WECOM_WS_URL.to_string()
    } else {
        ws_url
    };

    let (stream, _) = connect_async(&ws_url).await?;
    tracing::info!(url = %ws_url, "wecom stream connected");
    let (mut write, mut read) = stream.split();

    let subscribe = ws_frame(
        "aibot_subscribe",
        &new_req_id(),
        json!({
            "bot_id": bot_id,
            "secret": bot_secret,
        }),
    );
    write.send(WsMessage::Text(subscribe.into())).await?;
    wait_subscribe_ok(&mut read).await?;

    let mut ping = tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            _ = &mut *shutdown_rx => {
                let _ = write.close().await;
                return Ok(true);
            }
            _ = ping.tick() => {
                let ping = ws_frame("ping", &new_req_id(), Value::Null);
                write.send(WsMessage::Text(ping.into())).await?;
            }
            outbound = ws_out_rx.recv() => {
                if let Some(frame) = outbound {
                    write.send(WsMessage::Text(frame.into())).await?;
                }
            }
            message = read.next() => {
                let Some(message) = message else { return Ok(false); };
                let text = match message? {
                    WsMessage::Text(text) => text,
                    WsMessage::Ping(bytes) => {
                        write.send(WsMessage::Pong(bytes)).await?;
                        continue;
                    }
                    WsMessage::Close(_) => return Ok(false),
                    _ => continue,
                };
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    handle_inbound_frame(Arc::clone(&platform), &value).await;
                }
            }
        }
    }
}

async fn wait_subscribe_ok(
    read: &mut SplitStream<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>,
) -> Result<()> {
    for _ in 0..8 {
        let Some(message) = read.next().await else {
            return Err(anyhow!("wecom stream closed before subscribe ack"));
        };
        let text = match message? {
            WsMessage::Text(text) => text,
            _ => continue,
        };
        let value: Value = serde_json::from_str(&text)?;
        if value.get("errcode").and_then(Value::as_i64) == Some(0) {
            tracing::info!("wecom stream subscribed");
            return Ok(());
        }
        if value.get("cmd").and_then(Value::as_str) == Some("aibot_subscribe") {
            let code = value.get("errcode").and_then(Value::as_i64).unwrap_or(-1);
            let msg = value
                .get("errmsg")
                .and_then(Value::as_str)
                .unwrap_or("subscribe failed");
            return Err(anyhow!("wecom subscribe failed ({code}): {msg}"));
        }
    }
    Err(anyhow!("wecom subscribe ack timeout"))
}

async fn handle_inbound_frame(platform: Arc<WeComPlatform>, frame: &Value) {
    let cmd = frame.get("cmd").and_then(Value::as_str).unwrap_or("");
    match cmd {
        "aibot_msg_callback" => {
            if let Some(body) = frame.get("body") {
                match message_from_callback_body(&platform, body, frame) {
                    Ok(Some(message)) => {
                        dispatch_webhook(
                            Arc::clone(&platform) as Arc<dyn Platform>,
                            &platform.handler,
                            message,
                        )
                        .await;
                    }
                    Ok(None) => {}
                    Err(err) => tracing::warn!(error = %err, "wecom stream message parse failed"),
                }
            }
        }
        "aibot_event_callback" => {
            tracing::debug!(body = ?frame.get("body"), "wecom stream event");
        }
        _ => {}
    }
}

pub(super) fn message_from_callback_body(
    platform: &WeComPlatform,
    body: &Value,
    frame: &Value,
) -> Result<Option<crate::core::Message>> {
    let msg_type = body
        .get("msgtype")
        .and_then(Value::as_str)
        .unwrap_or("text");
    if msg_type != "text" {
        return Ok(None);
    }
    let content = body
        .get("text")
        .and_then(|text| text.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Ok(None);
    }
    let user_id = body
        .get("from")
        .and_then(|from| from.get("userid"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let chat_id = body
        .get("chatid")
        .and_then(Value::as_str)
        .unwrap_or_else(|| user_id.as_str())
        .to_string();
    let chat_type = body
        .get("chattype")
        .and_then(Value::as_str)
        .unwrap_or("single")
        .to_string();
    let msg_id = body
        .get("msgid")
        .and_then(Value::as_str)
        .map(str::to_string);
    let req_id = frame
        .get("headers")
        .and_then(|headers| headers.get("req_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let session_key = if platform.config.share_session_in_channel && chat_type == "group" {
        format!("{}:group:{}", platform.config.name, chat_id)
    } else {
        format!("{}:single:{}", platform.config.name, user_id)
    };
    let bot_ctx = WeComBotReplyContext {
        req_id,
        user_id: user_id.clone(),
        chat_id,
        chat_type,
        msg_id: msg_id.clone(),
    };
    simple_message(
        &platform.config.name,
        session_key,
        user_id,
        msg_id.as_deref(),
        content,
        SimpleReplyContext {
            channel: platform.config.name.clone(),
            target: bot_ctx.user_id.clone(),
            message_id: msg_id.clone(),
            extra: serde_json::to_value(&bot_ctx)?,
        },
    )
}

pub(super) fn build_respond_msg_frame(req_id: &str, content: &str) -> String {
    ws_frame(
        "aibot_respond_msg",
        req_id,
        json!({
            "msgtype": "markdown",
            "markdown": { "content": content }
        }),
    )
}

pub(super) fn build_send_msg_frame(chat_id: &str, chat_type: &str, content: &str) -> String {
    let chat_type_num = if chat_type == "group" { 2 } else { 1 };
    ws_frame(
        "aibot_send_msg",
        &new_req_id(),
        json!({
            "chatid": chat_id,
            "chat_type": chat_type_num,
            "msgtype": "markdown",
            "markdown": { "content": content }
        }),
    )
}

fn ws_frame(cmd: &str, req_id: &str, body: Value) -> String {
    let mut frame = json!({
        "cmd": cmd,
        "headers": { "req_id": req_id },
    });
    if !body.is_null() {
        frame["body"] = body;
    }
    frame.to_string()
}

fn new_req_id() -> String {
    format!("agentlink-{}", rand::random::<u64>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::wecom::WeComPlatformConfig;

    #[test]
    fn parses_text_callback_into_message() {
        let platform = WeComPlatform::new(WeComPlatformConfig::default());
        let frame = serde_json::json!({
            "cmd": "aibot_msg_callback",
            "headers": { "req_id": "req-1" },
            "body": {
                "msgid": "m1",
                "chatid": "chat-1",
                "chattype": "single",
                "from": { "userid": "user-1" },
                "msgtype": "text",
                "text": { "content": "hello wecom" }
            }
        });
        let message = message_from_callback_body(&platform, frame.get("body").unwrap(), &frame)
            .expect("parse")
            .expect("message");
        assert_eq!(message.content, "hello wecom");
        assert_eq!(message.user_id, "user-1");
        assert!(message.reply_ctx.value.contains("req-1"));
    }
}
