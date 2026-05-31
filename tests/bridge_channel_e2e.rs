use agentlink::bridge::{BridgePlatform, BridgePlatformConfig};
use agentlink::core::Platform;
use agentlink::mock::MockAgent;
use agentlink::{Engine, SessionStore};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[tokio::test]
async fn bridge_websocket_adapter_round_trip_e2e() {
    let platform = BridgePlatform::new(BridgePlatformConfig {
        name: "bridge".to_string(),
        listen: "127.0.0.1:0".to_string(),
        path: "/bridge/ws".to_string(),
        token: Some("secret".to_string()),
        insecure: false,
    });
    let engine = Engine::new(
        "test",
        ".",
        Arc::new(MockAgent::new()),
        vec![Arc::clone(&platform) as Arc<dyn Platform>],
        SessionStore::in_memory().unwrap(),
    );
    engine.start().await.unwrap();

    let addr = platform.local_addr().await.expect("bridge address");
    let (mut ws, _) = connect_async(format!("ws://{addr}/bridge/ws?token=secret"))
        .await
        .unwrap();

    ws.send(WsMessage::Text(
        json!({
            "type": "register",
            "platform": "test-channel",
            "capabilities": ["text"]
        })
        .to_string(),
    ))
    .await
    .unwrap();
    let ack = next_json(&mut ws).await;
    assert_eq!(ack["type"], "register_ack");
    assert_eq!(ack["ok"], true);

    ws.send(WsMessage::Text(
        json!({
            "type": "message",
            "msg_id": "msg-1",
            "session_key": "test-channel:room-1:user-1",
            "user_id": "user-1",
            "user_name": "tester",
            "content": "hello bridge",
            "reply_ctx": "opaque-room-1"
        })
        .to_string(),
    ))
    .await
    .unwrap();

    let reply = next_json(&mut ws).await;
    assert_eq!(reply["type"], "reply");
    assert_eq!(reply["session_key"], "test-channel:room-1:user-1");
    assert_eq!(reply["reply_ctx"], "opaque-room-1");
    assert_eq!(reply["content"], "mock: hello bridge");

    let outbox = platform.outbox().await;
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].adapter, "test-channel");

    engine.stop().await.unwrap();
}

#[tokio::test]
async fn bridge_websocket_rejects_missing_token() {
    let platform = BridgePlatform::new(BridgePlatformConfig {
        name: "bridge".to_string(),
        listen: "127.0.0.1:0".to_string(),
        path: "/bridge/ws".to_string(),
        token: Some("secret".to_string()),
        insecure: false,
    });
    let engine = Engine::new(
        "test",
        ".",
        Arc::new(MockAgent::new()),
        vec![Arc::clone(&platform) as Arc<dyn Platform>],
        SessionStore::in_memory().unwrap(),
    );
    engine.start().await.unwrap();

    let addr = platform.local_addr().await.expect("bridge address");
    let err = connect_async(format!("ws://{addr}/bridge/ws"))
        .await
        .expect_err("missing token should fail");
    assert!(err.to_string().contains("401"));

    engine.stop().await.unwrap();
}

async fn next_json<S>(ws: &mut S) -> Value
where
    S: StreamExt<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let frame = timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = frame.into_text().unwrap();
    serde_json::from_str(&text).unwrap()
}
