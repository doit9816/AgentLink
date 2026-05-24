use agentlink::core::Platform;
use agentlink::dingtalk::{DingTalkPlatform, DingTalkPlatformConfig};
use agentlink::feishu::{FeishuPlatform, FeishuPlatformConfig};
use agentlink::mock::MockAgent;
use agentlink::{Engine, SessionStore};
use axum::extract::ws::{Message as AxumWsMessage, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use prost::Message as ProstMessage;
use reqwest::StatusCode;
use serde_json::json;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};

#[derive(Clone, PartialEq, ProstMessage)]
struct TestFeishuWsHeader {
    #[prost(string, required, tag = "1")]
    key: String,
    #[prost(string, required, tag = "2")]
    value: String,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct TestFeishuWsFrame {
    #[prost(uint64, required, tag = "1")]
    seq_id: u64,
    #[prost(uint64, required, tag = "2")]
    log_id: u64,
    #[prost(int32, required, tag = "3")]
    service: i32,
    #[prost(int32, required, tag = "4")]
    method: i32,
    #[prost(message, repeated, tag = "5")]
    headers: Vec<TestFeishuWsHeader>,
    #[prost(string, optional, tag = "6")]
    payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    payload_type: Option<String>,
    #[prost(bytes, optional, tag = "8")]
    payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    log_id_new: Option<String>,
}

#[tokio::test]
async fn feishu_webhook_text_round_trip_e2e() {
    let platform = FeishuPlatform::new(FeishuPlatformConfig {
        name: "feishu".to_string(),
        listen: "127.0.0.1:0".to_string(),
        callback_path: "/feishu/webhook".to_string(),
        dry_run: true,
        ..FeishuPlatformConfig::default()
    });
    let engine = Engine::new(
        "test",
        Arc::new(MockAgent::new()),
        vec![Arc::clone(&platform) as Arc<dyn Platform>],
        SessionStore::in_memory().unwrap(),
    );
    engine.start().await.unwrap();
    let addr = platform.local_addr().await.unwrap();
    wait_for_json_health(format!("http://{addr}/feishu/healthz")).await;

    let response = reqwest::Client::new()
        .post(format!("http://{addr}/feishu/webhook"))
        .json(&json!({
            "schema": "2.0",
            "header": {
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_id": {
                        "open_id": "ou_user_1"
                    }
                },
                "message": {
                    "message_id": "om_msg_1",
                    "chat_id": "oc_chat_1",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"hello feishu\"}"
                }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.chat_id, "oc_chat_1");
    assert_eq!(outbound.message_id.as_deref(), Some("om_msg_1"));
    assert_eq!(outbound.content, "mock: hello feishu");
    engine.stop().await.unwrap();
}

#[tokio::test]
async fn feishu_url_verification_returns_challenge() {
    let platform = FeishuPlatform::new(FeishuPlatformConfig {
        name: "feishu".to_string(),
        listen: "127.0.0.1:0".to_string(),
        callback_path: "/feishu/webhook".to_string(),
        dry_run: true,
        ..FeishuPlatformConfig::default()
    });
    let engine = Engine::new(
        "test",
        Arc::new(MockAgent::new()),
        vec![Arc::clone(&platform) as Arc<dyn Platform>],
        SessionStore::in_memory().unwrap(),
    );
    engine.start().await.unwrap();
    let addr = platform.local_addr().await.unwrap();
    wait_for_json_health(format!("http://{addr}/feishu/healthz")).await;

    let value: serde_json::Value = reqwest::Client::new()
        .post(format!("http://{addr}/feishu/webhook"))
        .json(&json!({
            "type": "url_verification",
            "challenge": "challenge-code"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(value["challenge"], "challenge-code");
    engine.stop().await.unwrap();
}

#[tokio::test]
async fn feishu_webhook_verification_token_rejects_invalid_request() {
    let platform = FeishuPlatform::new(FeishuPlatformConfig {
        name: "feishu".to_string(),
        listen: "127.0.0.1:0".to_string(),
        callback_path: "/feishu/webhook".to_string(),
        verification_token: Some("expected-token".to_string()),
        dry_run: true,
        ..FeishuPlatformConfig::default()
    });
    let engine = Engine::new(
        "test",
        Arc::new(MockAgent::new()),
        vec![Arc::clone(&platform) as Arc<dyn Platform>],
        SessionStore::in_memory().unwrap(),
    );
    engine.start().await.unwrap();
    let addr = platform.local_addr().await.unwrap();
    wait_for_json_health(format!("http://{addr}/feishu/healthz")).await;

    let response = reqwest::Client::new()
        .post(format!("http://{addr}/feishu/webhook"))
        .json(&json!({
            "schema": "2.0",
            "header": {
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_id": {
                        "open_id": "ou_user_1"
                    }
                },
                "message": {
                    "message_id": "om_msg_1",
                    "chat_id": "oc_chat_1",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": "{\"text\":\"hello feishu\"}"
                }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    engine.stop().await.unwrap();
}

#[tokio::test]
async fn feishu_websocket_text_round_trip_e2e() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let ws_url = format!("ws://{addr}/ws?device_id=test-device&service_id=1");
    let endpoint_ws_url = ws_url.clone();
    let app = Router::new()
        .route(
            "/callback/ws/endpoint",
            post(move || {
                let endpoint_ws_url = endpoint_ws_url.clone();
                async move {
                    Json(json!({
                        "code": 0,
                        "msg": "ok",
                        "data": {
                            "URL": endpoint_ws_url,
                            "ClientConfig": {
                                "PingInterval": 30,
                                "ReconnectInterval": 1
                            }
                        }
                    }))
                }
            }),
        )
        .route("/ws", get(feishu_test_ws))
        .route("/healthz", get(|| async { Json(json!({ "ok": true })) }));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    wait_for_json_health(format!("http://{addr}/healthz")).await;

    let platform = FeishuPlatform::new(FeishuPlatformConfig {
        name: "feishu".to_string(),
        app_id: "cli_test".to_string(),
        app_secret: "sec_test".to_string(),
        api_base: format!("http://{addr}"),
        connection_mode: "websocket".to_string(),
        dry_run: true,
        ..FeishuPlatformConfig::default()
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
    assert_eq!(outbound.chat_id, "oc_ws_chat_1");
    assert_eq!(outbound.message_id.as_deref(), Some("om_ws_msg_1"));
    assert_eq!(outbound.content, "mock: hello feishu websocket");
    engine.stop().await.unwrap();
}

async fn feishu_test_ws(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|mut socket| async move {
        socket
            .send(AxumWsMessage::Binary(test_feishu_event_frame()))
            .await
            .unwrap();
        while let Some(Ok(message)) = socket.next().await {
            if let AxumWsMessage::Binary(bytes) = message {
                let frame = TestFeishuWsFrame::decode(bytes.as_slice()).unwrap();
                let payload = frame.payload.unwrap_or_default();
                let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
                if value.get("code").and_then(|v| v.as_u64()) == Some(200) {
                    break;
                }
            }
        }
    })
}

fn test_feishu_event_frame() -> Vec<u8> {
    let payload = serde_json::to_vec(&json!({
        "schema": "2.0",
        "header": {
            "event_type": "im.message.receive_v1"
        },
        "event": {
            "sender": {
                "sender_id": {
                    "open_id": "ou_ws_user_1"
                }
            },
            "message": {
                "message_id": "om_ws_msg_1",
                "chat_id": "oc_ws_chat_1",
                "chat_type": "group",
                "message_type": "text",
                "content": "{\"text\":\"hello feishu websocket\"}"
            }
        }
    }))
    .unwrap();
    let frame = TestFeishuWsFrame {
        seq_id: 1,
        log_id: 1,
        service: 1,
        method: 1,
        headers: vec![
            TestFeishuWsHeader {
                key: "type".to_string(),
                value: "event".to_string(),
            },
            TestFeishuWsHeader {
                key: "sum".to_string(),
                value: "1".to_string(),
            },
            TestFeishuWsHeader {
                key: "seq".to_string(),
                value: "0".to_string(),
            },
            TestFeishuWsHeader {
                key: "message_id".to_string(),
                value: "test-frame-1".to_string(),
            },
            TestFeishuWsHeader {
                key: "trace_id".to_string(),
                value: "trace-1".to_string(),
            },
        ],
        payload_encoding: None,
        payload_type: None,
        payload: Some(payload),
        log_id_new: None,
    };
    let mut bytes = Vec::new();
    frame.encode(&mut bytes).unwrap();
    bytes
}

#[tokio::test]
async fn dingtalk_webhook_text_round_trip_e2e() {
    let platform = DingTalkPlatform::new(DingTalkPlatformConfig {
        name: "dingtalk".to_string(),
        connection_mode: "webhook".to_string(),
        listen: "127.0.0.1:0".to_string(),
        callback_path: "/dingtalk/webhook".to_string(),
        dry_run: true,
        ..DingTalkPlatformConfig::default()
    });
    let engine = Engine::new(
        "test",
        Arc::new(MockAgent::new()),
        vec![Arc::clone(&platform) as Arc<dyn Platform>],
        SessionStore::in_memory().unwrap(),
    );
    engine.start().await.unwrap();
    let addr = platform.local_addr().await.unwrap();
    wait_for_json_health(format!("http://{addr}/dingtalk/healthz")).await;

    let response = reqwest::Client::new()
        .post(format!("http://{addr}/dingtalk/webhook"))
        .json(&json!({
            "msgId": "ding_msg_1",
            "msgtype": "text",
            "text": { "content": "hello dingtalk" },
            "senderStaffId": "staff_1",
            "senderNick": "Ding User",
            "conversationId": "cid_1",
            "conversationType": "2",
            "sessionWebhook": "https://example.invalid/session-webhook"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.conversation_id, "cid_1");
    assert_eq!(outbound.sender_staff_id, "staff_1");
    assert_eq!(outbound.content, "mock: hello dingtalk");
    engine.stop().await.unwrap();
}

async fn wait_for_json_health(url: String) {
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for _ in 0..40 {
        match client.get(&url).send().await {
            Ok(response) if response.status() == StatusCode::OK => return,
            _ => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
    panic!("health endpoint did not become ready: {url}");
}
