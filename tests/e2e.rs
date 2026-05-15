use agentlink::core::{Message, ReplyContext};
use agentlink::mock::{MockAgent, MockPlatform};
use agentlink::{Engine, SessionStore};
use std::sync::Arc;
use tokio::time::{timeout, Duration};

async fn test_engine() -> (Arc<Engine>, Arc<MockPlatform>, SessionStore) {
    let store = SessionStore::in_memory().unwrap();
    let agent = Arc::new(MockAgent::new());
    let platform = MockPlatform::new("mock");
    let engine = Engine::new(
        "test",
        agent,
        vec![Arc::clone(&platform) as Arc<dyn agentlink::core::Platform>],
        store.clone(),
    );
    engine.start().await.unwrap();
    (engine, platform, store)
}

#[tokio::test]
async fn mock_platform_mock_agent_final_reply_e2e() {
    let (_engine, platform, store) = test_engine().await;
    let mut msg = Message::text("chat-1", "user-1", "hello");
    msg.reply_ctx = ReplyContext::from("reply-1");
    platform.inject(msg).await.unwrap();

    let outbound = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outbound.kind, "send");
    assert_eq!(outbound.content, "mock: hello");

    let session = store
        .get_session("test", "mock", "chat-1")
        .unwrap()
        .expect("session should persist");
    assert!(session.agent_session_id.starts_with("mock-session-"));
}

#[tokio::test]
async fn approval_allow_e2e() {
    let (_engine, platform, _store) = test_engine().await;
    let mut msg = Message::text("chat-approval", "user-1", "please trigger approval");
    msg.reply_ctx = ReplyContext::from("reply-approval");
    platform.inject(msg).await.unwrap();

    let approval_prompt = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert!(approval_prompt.content.contains("审批编号："));
    let approval_id = approval_prompt
        .content
        .lines()
        .find_map(|line| line.strip_prefix("审批编号："))
        .expect("approval id")
        .trim()
        .to_string();

    let mut allow = Message::text("chat-approval", "user-1", format!("/allow {approval_id}"));
    allow.reply_ctx = ReplyContext::from("reply-approval");
    platform.inject(allow).await.unwrap();

    let ack = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert!(ack.content.contains("审批已提交"));

    let final_msg = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .unwrap()
        .unwrap();
    assert!(final_msg.content.contains("mock approved"));
}
