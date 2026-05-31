use agentlink::core::Platform;
use agentlink::core::{Attachment, AttachmentKind, MessageType};
use agentlink::http_channel::{HttpInboundMessage, HttpPlatform, HttpPlatformConfig};
use agentlink::mock::MockAgent;
use agentlink::{Engine, SessionStore};
use reqwest::StatusCode;
use std::sync::Arc;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn http_platform_receives_webhook_and_captures_final_reply() {
    let platform = HttpPlatform::new(HttpPlatformConfig {
        name: "http-test".to_string(),
        listen: "127.0.0.1:0".to_string(),
        bearer_token: None,
        outbound_url: None,
    });
    let engine = Engine::new(
        "test",
        ".",
        Arc::new(MockAgent::new()),
        vec![Arc::clone(&platform) as Arc<dyn Platform>],
        SessionStore::in_memory().unwrap(),
    );
    engine.start().await.unwrap();

    let addr = platform.local_addr().await.expect("http platform address");
    let client = reqwest::Client::new();
    let inbound = HttpInboundMessage {
        project: None,
        session_key: "chat-http".to_string(),
        user_id: "user-1".to_string(),
        user_name: Some("tester".to_string()),
        message_type: MessageType::Text,
        content: "hello http".to_string(),
        reply_ctx: Some("reply-http".to_string()),
        message_id: Some("msg-1".to_string()),
        attachments: Vec::new(),
        images: Vec::new(),
        files: Vec::new(),
    };
    let response = client
        .post(format!("http://{addr}/webhook"))
        .json(&inbound)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.kind, "send");
    assert_eq!(outbound.reply_ctx.value, "reply-http");
    assert_eq!(outbound.content, "mock: hello http");

    let outbox: Vec<serde_json::Value> = client
        .get(format!("http://{addr}/outbox"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(outbox.len(), 1);

    engine.stop().await.unwrap();
}

#[tokio::test]
async fn http_platform_accepts_rich_message_attachments() {
    let platform = HttpPlatform::new(HttpPlatformConfig {
        name: "http-rich".to_string(),
        listen: "127.0.0.1:0".to_string(),
        bearer_token: None,
        outbound_url: None,
    });
    let engine = Engine::new(
        "test",
        ".",
        Arc::new(MockAgent::new()),
        vec![Arc::clone(&platform) as Arc<dyn Platform>],
        SessionStore::in_memory().unwrap(),
    );
    engine.start().await.unwrap();

    let addr = platform.local_addr().await.expect("http platform address");
    let inbound = HttpInboundMessage {
        project: None,
        session_key: "chat-rich".to_string(),
        user_id: "user-1".to_string(),
        user_name: None,
        message_type: MessageType::Mixed,
        content: "handle this".to_string(),
        reply_ctx: Some("reply-rich".to_string()),
        message_id: Some("msg-rich".to_string()),
        attachments: vec![Attachment {
            kind: AttachmentKind::Location,
            text: Some("office".to_string()),
            metadata: Some(serde_json::json!({"lat": 31.2, "lng": 121.5})),
            ..Attachment::default()
        }],
        images: Vec::new(),
        files: Vec::new(),
    };
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/webhook"))
        .json(&inbound)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert!(outbound.content.contains("[attachments]"));
    assert!(outbound.content.contains("type=location"));
    engine.stop().await.unwrap();
}

#[tokio::test]
async fn http_platform_optional_bearer_token_protects_webhook() {
    let platform = HttpPlatform::new(HttpPlatformConfig {
        name: "http-auth".to_string(),
        listen: "127.0.0.1:0".to_string(),
        bearer_token: Some("secret".to_string()),
        outbound_url: None,
    });
    let engine = Engine::new(
        "test",
        ".",
        Arc::new(MockAgent::new()),
        vec![Arc::clone(&platform) as Arc<dyn Platform>],
        SessionStore::in_memory().unwrap(),
    );
    engine.start().await.unwrap();

    let addr = platform.local_addr().await.expect("http platform address");
    let inbound = serde_json::json!({
        "session_key": "chat-auth",
        "user_id": "user-1",
        "content": "hello"
    });
    let client = reqwest::Client::new();
    let denied = client
        .post(format!("http://{addr}/webhook"))
        .json(&inbound)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

    let accepted = client
        .post(format!("http://{addr}/webhook"))
        .bearer_auth("secret")
        .json(&inbound)
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);

    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.content, "mock: hello");

    engine.stop().await.unwrap();
}
