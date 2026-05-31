use agentlink::core::{Message, ReplyContext};
use agentlink::mock::MockPlatform;
use agentlink::{Engine, SessionStore};
use std::sync::Arc;
use tokio::time::{timeout, Duration};

async fn test_engine() -> (Arc<Engine>, Arc<MockPlatform>) {
    let store = SessionStore::in_memory().expect("store");
    let platform = MockPlatform::new("mock");
    let engine = Engine::new(
        "test",
        ".",
        Arc::new(agentlink::mock::MockAgent::new()),
        vec![Arc::clone(&platform) as Arc<dyn agentlink::core::Platform>],
        store,
    );
    engine.start().await.expect("start");
    (engine, platform)
}

#[tokio::test]
async fn session_new_list_current_e2e() {
    let (_engine, platform) = test_engine().await;
    let mut msg = Message::text("chat-sess", "user-1", "/new demo");
    msg.reply_ctx = ReplyContext::from("reply-sess");
    platform.inject(msg).await.expect("inject");
    let reply = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .expect("timeout")
        .expect("reply");
    assert!(reply.content.contains("已创建会话 #1"));
    assert!(reply.content.contains("demo"));

    let mut list = Message::text("chat-sess", "user-1", "/list");
    list.reply_ctx = ReplyContext::from("reply-sess");
    platform.inject(list).await.expect("inject");
    let list_reply = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .expect("timeout")
        .expect("list");
    assert!(list_reply.content.contains("#1"));
    assert!(list_reply.content.contains("*"));

    let mut current = Message::text("chat-sess", "user-1", "/current");
    current.reply_ctx = ReplyContext::from("reply-sess");
    platform.inject(current).await.expect("inject");
    let current_reply = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .expect("timeout")
        .expect("current");
    assert!(current_reply.content.contains("当前会话：#1"));
}

#[tokio::test]
async fn session_dir_show_e2e() {
    let (_engine, platform) = test_engine().await;
    let mut msg = Message::text("chat-dir", "user-1", "/dir");
    msg.reply_ctx = ReplyContext::from("reply-dir");
    platform.inject(msg).await.expect("inject");
    let reply = timeout(Duration::from_secs(2), platform.wait_for_outbound())
        .await
        .expect("timeout")
        .expect("reply");
    assert!(reply.content.contains("当前工作目录"));
}
