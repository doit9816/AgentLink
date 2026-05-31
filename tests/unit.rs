use agentlink::acp::AcpAgent;
use agentlink::codex::extract_completed_agent_message;
use agentlink::core::{
    parse_approval_command, parse_session_command, PermissionBehavior, SessionCommand,
};
use agentlink::mock::MockAgent;
use agentlink::{Registry, SessionStore};
use serde_json::json;
use std::sync::Arc;

#[test]
fn parse_session_management_commands() {
    assert_eq!(
        parse_session_command("/new"),
        Some(SessionCommand::New { label: None })
    );
    assert_eq!(parse_session_command("/list"), Some(SessionCommand::List));
    assert_eq!(
        parse_session_command("/switch 2"),
        Some(SessionCommand::Switch {
            slot_id: "2".to_string()
        })
    );
}

#[test]
fn parse_approval_commands() {
    let allow = parse_approval_command("/allow apv_1").unwrap();
    assert_eq!(allow.decision, PermissionBehavior::Allow);
    assert_eq!(allow.approval_id, "apv_1");

    let deny = parse_approval_command("/deny apv_2").unwrap();
    assert_eq!(deny.decision, PermissionBehavior::Deny);
    assert_eq!(deny.approval_id, "apv_2");

    assert!(parse_approval_command("/allow").is_none());
}

#[test]
fn registry_creates_acp_agent() {
    let mut registry = Registry::new();
    registry.register_agent("acp", |opts| {
        Ok(std::sync::Arc::new(AcpAgent::new_from_options(opts)?))
    });
    let mut opts = toml::value::Table::new();
    opts.insert(
        "command".to_string(),
        toml::Value::String("agent".to_string()),
    );
    let agent = registry.create_agent("acp", opts).expect("acp agent");
    assert_eq!(agent.name(), "acp");
    assert!(agent.capabilities().approvals);
}

#[test]
fn registry_unknown_type_lists_available() {
    let mut registry = Registry::new();
    registry.register_agent("mock", |_| Ok(Arc::new(MockAgent::new())));
    let err = match registry.create_agent("missing", toml::value::Table::new()) {
        Ok(_) => panic!("expected missing agent to fail"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("mock"));
}

#[test]
fn sqlite_session_store_persists_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bridge.sqlite3");
    let store = SessionStore::open(&path).unwrap();
    store
        .upsert_session("p", "mock", "chat", "mock", "agent-session")
        .unwrap();
    drop(store);

    let reopened = SessionStore::open(&path).unwrap();
    let record = reopened.get_session("p", "mock", "chat").unwrap().unwrap();
    assert_eq!(record.agent_session_id, "agent-session");
}

#[test]
fn codex_completed_agent_message_extraction() {
    let value = json!({
        "item": {
            "type": "agentMessage",
            "content": [
                { "text": "hello " },
                { "text": "codex" }
            ]
        }
    });
    assert_eq!(
        extract_completed_agent_message(&value).unwrap(),
        "hello codex"
    );
}
