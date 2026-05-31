mod exec;
mod options;
mod preset;

use crate::core::{
    Agent, AgentCapabilities, AgentSession, AgentSessionInfo, Event, FileAttachment,
    ImageAttachment, PermissionResult, SessionStartRequest,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

pub use options::CliAgent;

#[async_trait]
impl Agent for CliAgent {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            images: true,
            files: true,
            model_switching: self.model.is_some(),
            streaming_output: true,
            ..AgentCapabilities::default()
        }
    }

    async fn start_session(&self, request: SessionStartRequest) -> Result<Arc<dyn AgentSession>> {
        let mut agent = self.clone();
        if let Some(dir) = request.work_dir {
            agent.work_dir = dir;
        }
        Ok(Arc::new(CliAgentSession::new(
            agent,
            request.resume_session_id,
        )))
    }

    async fn list_sessions(&self) -> Result<Vec<AgentSessionInfo>> {
        Ok(Vec::new())
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }
}

pub struct CliAgentSession {
    agent: CliAgent,
    id: String,
    alive: AtomicBool,
    tx: mpsc::UnboundedSender<Event>,
    rx: Mutex<mpsc::UnboundedReceiver<Event>>,
}

impl CliAgentSession {
    fn new(agent: CliAgent, session_id: Option<String>) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let (tx, rx) = mpsc::unbounded_channel();
        let id = session_id.unwrap_or_else(|| {
            format!(
                "{}-cli-session-{}",
                agent.name,
                NEXT_ID.fetch_add(1, Ordering::SeqCst)
            )
        });
        Self {
            agent,
            id,
            alive: AtomicBool::new(true),
            tx,
            rx: Mutex::new(rx),
        }
    }
}

#[async_trait]
impl AgentSession for CliAgentSession {
    async fn send(
        &self,
        prompt: String,
        images: Vec<ImageAttachment>,
        files: Vec<FileAttachment>,
    ) -> Result<()> {
        if !self.alive() {
            return Err(anyhow!("cli session is closed"));
        }
        let agent = self.agent.clone();
        let session_id = self.id.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = exec::run_cli_turn(&agent, &session_id, prompt, images, files).await;
            let event = match result {
                Ok(content) => Event::result(content, session_id),
                Err(err) => Event::error(err.to_string()),
            };
            let _ = tx.send(event);
        });
        Ok(())
    }

    async fn respond_permission(
        &self,
        _request_id: String,
        _result: PermissionResult,
    ) -> Result<()> {
        Err(anyhow!("cli agent does not support permission callbacks"))
    }

    async fn recv_event(&self) -> Option<Event> {
        self.rx.lock().await.recv().await
    }

    fn current_session_id(&self) -> String {
        self.id.clone()
    }

    fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    async fn close(&self) -> Result<()> {
        self.alive.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::exec::extract_cli_output;
    use super::CliAgent;

    #[test]
    fn cli_json_output_extracts_text() {
        assert_eq!(
            extract_cli_output("{\"delta\":\"hi\"}\n{\"delta\":\" there\"}").as_deref(),
            Some("hi there")
        );
        assert_eq!(
            extract_cli_output("{\"content\":\"final\"}").as_deref(),
            Some("final")
        );
    }

    #[test]
    fn cli_agent_preset_can_be_overridden() {
        let mut opts = toml::value::Table::new();
        opts.insert(
            "command".to_string(),
            toml::Value::String("echo".to_string()),
        );
        let agent = CliAgent::new_from_options("gemini", opts);
        assert_eq!(agent.command, "echo");
        assert_eq!(agent.name, "gemini");
    }
}
