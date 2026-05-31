use agentlink::agents::acp::AcpAgent;
use agentlink::core::{EventType, Message, ReplyContext};
use agentlink::mock::MockPlatform;
use agentlink::{Agent, Engine, SessionStartRequest, SessionStore};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{timeout, Duration};

const E2E_TIMEOUT: Duration = Duration::from_secs(25);

fn acp_mock_bin() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_acp-mock-agent")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/acp-mock-agent")
        })
}

async fn acp_engine(work_dir: &std::path::Path) -> (Arc<Engine>, Arc<MockPlatform>, SessionStore) {
    let bin = acp_mock_bin();
    assert!(
        bin.is_file(),
        "missing acp mock binary at {}",
        bin.display()
    );

    let mut opts = toml::value::Table::new();
    opts.insert(
        "command".to_string(),
        toml::Value::String(bin.to_string_lossy().into_owned()),
    );
    opts.insert(
        "work_dir".to_string(),
        toml::Value::String(work_dir.to_string_lossy().into_owned()),
    );
    opts.insert("name".to_string(), toml::Value::String("acp".to_string()));

    let agent = Arc::new(AcpAgent::new_from_options(opts).expect("acp options"));
    let store = SessionStore::in_memory().expect("store");
    let platform = MockPlatform::new("mock");
    let engine = Engine::new(
        "test",
        work_dir.to_string_lossy().into_owned(),
        agent,
        vec![Arc::clone(&platform) as Arc<dyn agentlink::core::Platform>],
        store.clone(),
    );
    engine.start().await.expect("engine start");
    (engine, platform, store)
}

#[tokio::test]
async fn acp_session_direct_prompt() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = acp_mock_bin();
    let mut opts = toml::value::Table::new();
    opts.insert(
        "command".to_string(),
        toml::Value::String(bin.to_string_lossy().into_owned()),
    );
    opts.insert(
        "work_dir".to_string(),
        toml::Value::String(dir.path().to_string_lossy().into_owned()),
    );
    let agent = Arc::new(AcpAgent::new_from_options(opts).expect("options"));
    let session = agent
        .start_session(SessionStartRequest::fresh(Some(
            dir.path().to_string_lossy().into_owned(),
        )))
        .await
        .expect("start session");
    session
        .send("hello acp".to_string(), vec![], vec![])
        .await
        .expect("send");
    let mut text = String::new();
    while let Some(event) = session.recv_event().await {
        match event.event_type {
            EventType::Text => text.push_str(&event.content),
            EventType::Result => break,
            EventType::Error => panic!("error: {:?}", event.error),
            _ => {}
        }
    }
    assert_eq!(text, "mock-acp: hello acp");
}

#[tokio::test]
async fn acp_mock_agent_streams_reply_e2e() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_engine, platform, store) = acp_engine(dir.path()).await;

    let mut msg = Message::text("chat-acp", "user-1", "hello acp");
    msg.reply_ctx = ReplyContext::from("reply-acp");
    platform.inject(msg).await.expect("inject");

    let outbound = timeout(E2E_TIMEOUT, platform.wait_for_outbound())
        .await
        .expect("timeout")
        .expect("outbound");
    assert_eq!(outbound.kind, "send");
    assert_eq!(outbound.content, "mock-acp: hello acp");

    let session = store
        .get_session("test", "mock", "chat-acp")
        .expect("get session")
        .expect("session row");
    assert_eq!(session.agent_session_id, "mock-acp-session-1");
}

#[tokio::test]
async fn acp_mock_agent_permission_allow_e2e() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_engine, platform, _store) = acp_engine(dir.path()).await;

    let mut msg = Message::text("chat-acp-approval", "user-1", "please approval");
    msg.reply_ctx = ReplyContext::from("reply-acp-approval");
    platform.inject(msg).await.expect("inject");

    let approval_prompt = timeout(E2E_TIMEOUT, platform.wait_for_outbound())
        .await
        .expect("timeout")
        .expect("approval prompt");
    assert!(approval_prompt.content.contains("审批编号："));
    let approval_id = approval_prompt
        .content
        .lines()
        .find_map(|line| line.strip_prefix("审批编号："))
        .expect("approval id")
        .trim()
        .to_string();

    let mut allow = Message::text(
        "chat-acp-approval",
        "user-1",
        format!("/allow {approval_id}"),
    );
    allow.reply_ctx = ReplyContext::from("reply-acp-approval");
    platform.inject(allow).await.expect("inject allow");

    let ack = timeout(E2E_TIMEOUT, platform.wait_for_outbound())
        .await
        .expect("timeout")
        .expect("ack");
    assert!(ack.content.contains("审批已提交"));

    let final_msg = timeout(E2E_TIMEOUT, platform.wait_for_outbound())
        .await
        .expect("timeout")
        .expect("final");
    assert_eq!(final_msg.content, "mock-acp: approved");
}
