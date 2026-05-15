use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::io::{self, BufRead};
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::var("BRIDGE_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:9810/bridge/ws?token=change-me".to_string());
    let platform = std::env::var("BRIDGE_PLATFORM").unwrap_or_else(|_| "console".to_string());
    let user_id = std::env::var("BRIDGE_USER").unwrap_or_else(|_| "local-user".to_string());
    let session_key = format!("{platform}:dm:{user_id}");
    let reply_ctx = format!("{platform}:dm");

    let (ws, _) = connect_async(url).await?;
    let (mut writer, mut reader) = ws.split();

    writer
        .send(WsMessage::Text(
            json!({
                "type": "register",
                "platform": platform,
                "capabilities": ["text"],
                "metadata": {
                    "example": "examples/agentlink_adapter.rs"
                }
            })
            .to_string(),
        ))
        .await?;

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });

    println!("connected. Type a message and press Enter.");
    let mut seq: u64 = 0;
    loop {
        tokio::select! {
            Some(line) = rx.recv() => {
                if line.trim().is_empty() {
                    continue;
                }
                seq += 1;
                writer.send(WsMessage::Text(json!({
                    "type": "message",
                    "msg_id": format!("console-{seq}"),
                    "session_key": session_key,
                    "user_id": user_id,
                    "user_name": "Console User",
                    "content": line,
                    "reply_ctx": reply_ctx
                }).to_string())).await?;
            }
            Some(frame) = reader.next() => {
                let frame = frame?;
                if !frame.is_text() {
                    continue;
                }
                let value: Value = serde_json::from_str(frame.to_text()?)?;
                match value.get("type").and_then(Value::as_str).unwrap_or("") {
                    "register_ack" => println!("registered: {}", value),
                    "reply" => println!("\nagent> {}\n", value["content"].as_str().unwrap_or("")),
                    "pong" => {}
                    "error" => eprintln!("bridge error: {}", value),
                    other => println!("bridge {other}: {}", value),
                }
            }
            else => break,
        }
    }
    Ok(())
}
