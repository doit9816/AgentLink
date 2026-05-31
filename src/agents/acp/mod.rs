mod list_sessions;
mod mapping;
mod options;
mod rpc;
mod session;
mod state;

pub use options::AcpAgent;

use crate::core::{Agent, AgentCapabilities, AgentSession, AgentSessionInfo, SessionStartRequest};
use anyhow::Result;
use async_trait::async_trait;
use session::AcpSession;
use std::sync::Arc;

#[async_trait]
impl Agent for AcpAgent {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            images: true,
            files: true,
            approvals: true,
            streaming_output: true,
            ..AgentCapabilities::default()
        }
    }

    async fn start_session(&self, request: SessionStartRequest) -> Result<Arc<dyn AgentSession>> {
        let mut agent = self.clone();
        if let Some(dir) = request.work_dir {
            agent.work_dir = dir;
        }
        Ok(AcpSession::start(agent, request.resume_session_id).await? as Arc<dyn AgentSession>)
    }

    async fn list_sessions(&self) -> Result<Vec<AgentSessionInfo>> {
        list_sessions::list_sessions(self).await
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::AcpAgent;
    use crate::core::{Agent, AgentSession, SessionStartRequest};

    #[test]
    fn acp_agent_requires_command() {
        let opts = toml::value::Table::new();
        let err = AcpAgent::new_from_options(opts).unwrap_err();
        assert!(err.to_string().contains("command"));
    }

    fn acp_mock_bin() -> std::path::PathBuf {
        std::env::var_os("CARGO_BIN_EXE_acp-mock-agent")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("target/debug/acp-mock-agent")
            })
    }

    #[tokio::test]
    async fn acp_mock_lists_sessions_when_supported() {
        let bin = acp_mock_bin();
        if !bin.is_file() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let mut opts = toml::value::Table::new();
        opts.insert(
            "command".to_string(),
            toml::Value::String(bin.to_string_lossy().into_owned()),
        );
        opts.insert(
            "work_dir".to_string(),
            toml::Value::String(dir.path().to_string_lossy().into_owned()),
        );
        let agent = AcpAgent::new_from_options(opts).expect("options");
        let sessions = agent.list_sessions().await.expect("list");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "mock-acp-session-1");
        assert_eq!(sessions[0].summary.as_deref(), Some("mock session"));
    }

    #[tokio::test]
    async fn acp_mock_session_streams_text_and_result() {
        let bin = acp_mock_bin();
        if !bin.is_file() {
            eprintln!(
                "skip acp_mock_session_streams_text_and_result: no binary at {}",
                bin.display()
            );
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let mut opts = toml::value::Table::new();
        opts.insert(
            "command".to_string(),
            toml::Value::String(bin.to_string_lossy().into_owned()),
        );
        opts.insert(
            "work_dir".to_string(),
            toml::Value::String(dir.path().to_string_lossy().into_owned()),
        );
        let agent = AcpAgent::new_from_options(opts).expect("options");
        let session = agent
            .start_session(SessionStartRequest::fresh(None))
            .await
            .expect("start");
        session
            .send("hello acp".into(), vec![], vec![])
            .await
            .expect("send");
        let mut text = String::new();
        let mut saw_result = false;
        for _ in 0..20 {
            let Some(event) = session.recv_event().await else {
                break;
            };
            if event.event_type == crate::core::EventType::Text {
                text.push_str(&event.content);
            }
            if event.event_type == crate::core::EventType::Result {
                saw_result = true;
                break;
            }
        }
        assert!(saw_result, "expected Result event");
        assert_eq!(text, "mock-acp: hello acp");
        assert_eq!(session.current_session_id(), "mock-acp-session-1");
    }

    #[test]
    fn acp_agent_parses_options() {
        let mut opts = toml::value::Table::new();
        opts.insert(
            "command".to_string(),
            toml::Value::String("agent".to_string()),
        );
        opts.insert(
            "args".to_string(),
            toml::Value::Array(vec![toml::Value::String("acp".to_string())]),
        );
        opts.insert(
            "auth_method".to_string(),
            toml::Value::String("cursor_login".to_string()),
        );
        let agent = AcpAgent::new_from_options(opts).expect("options");
        assert_eq!(agent.command, "agent");
        assert_eq!(agent.args, vec!["acp"]);
        assert_eq!(agent.auth_method.as_deref(), Some("cursor_login"));
    }
}
