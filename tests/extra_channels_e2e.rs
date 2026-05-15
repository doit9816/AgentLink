use agentlink::core::Platform;
use agentlink::discord::{DiscordPlatform, DiscordPlatformConfig};
use agentlink::mock::MockAgent;
use agentlink::qq::{QqPlatform, QqPlatformConfig};
use agentlink::slack::{SlackPlatform, SlackPlatformConfig};
use agentlink::telegram::{TelegramPlatform, TelegramPlatformConfig};
use agentlink::{Engine, SessionStore};
use axum::extract::ws::{Message as AxumWsMessage, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn telegram_long_poll_text_round_trip_e2e() {
    let served_update = Arc::new(AtomicBool::new(false));
    let app = Router::new()
        .route(
            "/botTEST/getUpdates",
            post({
                let served_update = Arc::clone(&served_update);
                move || {
                    let served_update = Arc::clone(&served_update);
                    async move {
                        if served_update.swap(true, Ordering::SeqCst) {
                            Json(json!({ "ok": true, "result": [] }))
                        } else {
                            Json(json!({
                                "ok": true,
                                "result": [{
                                    "update_id": 1,
                                    "message": {
                                        "message_id": 11,
                                        "chat": { "id": 1001, "type": "private" },
                                        "from": { "id": 2002, "is_bot": false, "username": "alice" },
                                        "text": "hello telegram"
                                    }
                                }]
                            }))
                        }
                    }
                }
            }),
        )
        .route(
            "/botTEST/sendMessage",
            post(|| async { Json(json!({ "ok": true, "result": {} })) }),
        );
    let addr = serve(app).await;
    let platform = TelegramPlatform::new(TelegramPlatformConfig {
        token: "TEST".to_string(),
        api_base: format!("http://{addr}"),
        poll_timeout_secs: 1,
        dry_run: true,
        ..TelegramPlatformConfig::default()
    });
    let engine = Engine::new(
        "test",
        Arc::new(MockAgent::new()),
        vec![Arc::clone(&platform) as Arc<dyn Platform>],
        SessionStore::in_memory().unwrap(),
    );
    engine.start().await.unwrap();

    let outbound = timeout(Duration::from_secs(3), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.chat_id, "1001");
    assert_eq!(outbound.message_id, Some(11));
    assert_eq!(outbound.content, "mock: hello telegram");
    engine.stop().await.unwrap();
}

#[tokio::test]
async fn slack_socket_mode_text_round_trip_e2e() {
    let app = Router::new()
        .route(
            "/api/apps.connections.open",
            post(|| async {
                Json(json!({
                    "ok": true,
                    "url": "ws://127.0.0.1:0/not-ready"
                }))
            }),
        )
        .route("/slack/ws", get(slack_test_ws));
    let addr = serve(app).await;

    let app = Router::new()
        .route(
            "/api/apps.connections.open",
            post(move || async move {
                Json(json!({
                    "ok": true,
                    "url": format!("ws://{addr}/slack/ws")
                }))
            }),
        )
        .route("/slack/ws", get(slack_test_ws));
    let addr = serve(app).await;

    let platform = SlackPlatform::new(SlackPlatformConfig {
        bot_token: "xoxb-test".to_string(),
        app_token: "xapp-test".to_string(),
        api_base: format!("http://{addr}/api"),
        dry_run: true,
        ..SlackPlatformConfig::default()
    });
    let engine = Engine::new(
        "test",
        Arc::new(MockAgent::new()),
        vec![Arc::clone(&platform) as Arc<dyn Platform>],
        SessionStore::in_memory().unwrap(),
    );
    engine.start().await.unwrap();

    let outbound = timeout(Duration::from_secs(3), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.channel, "C1");
    assert_eq!(outbound.thread_ts.as_deref(), Some("123.45"));
    assert_eq!(outbound.content, "mock: hello slack");
    engine.stop().await.unwrap();
}

async fn slack_test_ws(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        socket
            .send(AxumWsMessage::Text(
                json!({
                    "type": "events_api",
                    "envelope_id": "env-1",
                    "payload": {
                        "type": "event_callback",
                        "event": {
                            "type": "message",
                            "user": "U1",
                            "channel": "C1",
                            "text": "hello slack",
                            "ts": "123.45"
                        }
                    }
                })
                .to_string(),
            ))
            .await
            .unwrap();
        while let Some(Ok(message)) = socket.next().await {
            if let AxumWsMessage::Text(text) = message {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value.get("envelope_id").and_then(Value::as_str) == Some("env-1") {
                    break;
                }
            }
        }
    })
}

#[tokio::test]
async fn discord_gateway_text_round_trip_e2e() {
    let app = Router::new().route("/gateway", get(discord_test_gateway));
    let addr = serve(app).await;

    let platform = DiscordPlatform::new(DiscordPlatformConfig {
        token: "discord-test".to_string(),
        api_base: format!("http://{addr}/api"),
        gateway_url: Some(format!("ws://{addr}/gateway")),
        dry_run: true,
        ..DiscordPlatformConfig::default()
    });
    let engine = Engine::new(
        "test",
        Arc::new(MockAgent::new()),
        vec![Arc::clone(&platform) as Arc<dyn Platform>],
        SessionStore::in_memory().unwrap(),
    );
    engine.start().await.unwrap();

    let outbound = timeout(Duration::from_secs(3), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.channel_id, "DCHAN1");
    assert_eq!(outbound.message_id.as_deref(), Some("DMSG1"));
    assert_eq!(outbound.content, "mock: hello discord");
    engine.stop().await.unwrap();
}

#[tokio::test]
async fn qq_onebot_websocket_text_round_trip_e2e() {
    let app = Router::new().route("/onebot", get(qq_test_ws));
    let addr = serve(app).await;
    let platform = QqPlatform::new(QqPlatformConfig {
        ws_url: format!("ws://{addr}/onebot"),
        dry_run: true,
        ..QqPlatformConfig::default()
    });
    let engine = Engine::new(
        "test",
        Arc::new(MockAgent::new()),
        vec![Arc::clone(&platform) as Arc<dyn Platform>],
        SessionStore::in_memory().unwrap(),
    );
    engine.start().await.unwrap();

    let outbound = timeout(Duration::from_secs(3), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.message_type, "group");
    assert_eq!(outbound.group_id, Some(7788));
    assert_eq!(outbound.user_id, Some(1234));
    assert_eq!(outbound.content, "mock: hello qq");
    engine.stop().await.unwrap();
}

async fn qq_test_ws(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        socket
            .send(AxumWsMessage::Text(
                json!({
                    "post_type": "message",
                    "message_type": "group",
                    "message_id": 99,
                    "group_id": 7788,
                    "user_id": 1234,
                    "raw_message": "hello qq",
                    "sender": {
                        "nickname": "alice"
                    }
                })
                .to_string(),
            ))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    })
}

async fn discord_test_gateway(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        socket
            .send(AxumWsMessage::Text(
                json!({ "op": 10, "d": { "heartbeat_interval": 30_000 } }).to_string(),
            ))
            .await
            .unwrap();
        socket
            .send(AxumWsMessage::Text(
                json!({
                    "op": 0,
                    "t": "MESSAGE_CREATE",
                    "s": 1,
                    "d": {
                        "id": "DMSG1",
                        "channel_id": "DCHAN1",
                        "content": "hello discord",
                        "author": {
                            "id": "DUSER1",
                            "username": "alice",
                            "bot": false
                        }
                    }
                })
                .to_string(),
            ))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    })
}

async fn serve(app: Router) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    addr
}
