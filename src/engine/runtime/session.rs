use super::Engine;
use crate::core::{AgentSession, Message, Platform};
use crate::store::{unix_now, TargetRecord};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Mutex;

impl Engine {
    pub(crate) fn record_target(&self, platform_name: &str, message: &Message) -> Result<()> {
        self.store.record_target(&TargetRecord {
            project: self.project.clone(),
            platform: platform_name.to_string(),
            session_key: message.session_key.clone(),
            user_id: message.user_id.clone(),
            user_name: message.user_name.clone(),
            message_id: message.message_id.clone(),
            reply_ctx: message.reply_ctx.value.clone(),
            content_preview: message.content.chars().take(160).collect(),
            updated_at: unix_now(),
        })?;
        tracing::info!(
            project = %self.project,
            platform = %platform_name,
            session_key = %message.session_key,
            user_id = %message.user_id,
            "channel target recorded"
        );
        Ok(())
    }

    pub(crate) async fn session_queue(&self, platform: &str, session_key: &str) -> Arc<Mutex<()>> {
        let key = format!("{platform}:{session_key}");
        let mut queues = self.session_queues.lock().await;
        queues
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub(super) async fn running_session(
        &self,
        platform: &str,
        session_key: &str,
    ) -> Result<Arc<dyn AgentSession>> {
        let key = format!("{platform}:{session_key}");
        if let Some(session) = self.sessions.lock().await.get(&key) {
            if session.alive() {
                tracing::info!(
                    project = %self.project,
                    platform = %platform,
                    session_key = %session_key,
                    agent = %self.agent.name(),
                    agent_session_id = %session.current_session_id(),
                    "agent session reused"
                );
                return Ok(Arc::clone(session));
            }
        }
        let resume_id = self
            .store
            .get_session(&self.project, platform, session_key)?
            .map(|r| r.agent_session_id);
        tracing::info!(
            project = %self.project,
            platform = %platform,
            session_key = %session_key,
            agent = %self.agent.name(),
            resume_agent_session_id = %resume_id.as_deref().unwrap_or(""),
            "agent session start requested"
        );
        let session = self.agent.start_session(resume_id).await?;
        tracing::info!(
            project = %self.project,
            platform = %platform,
            session_key = %session_key,
            agent = %self.agent.name(),
            agent_session_id = %session.current_session_id(),
            "agent session started"
        );
        self.sessions.lock().await.insert(key, Arc::clone(&session));
        Ok(session)
    }

    pub(super) async fn finish_agent_turn(
        &self,
        platform: Arc<dyn Platform>,
        message: &Message,
        platform_name: &str,
        session: Arc<dyn AgentSession>,
        result_content: String,
        fallback: String,
    ) -> Result<()> {
        self.store.upsert_session(
            &self.project,
            platform_name,
            &message.session_key,
            self.agent.name(),
            &session.current_session_id(),
        )?;
        let final_text = if result_content.trim().is_empty() {
            fallback.trim().to_string()
        } else {
            result_content.trim().to_string()
        };
        tracing::info!(
            project = %self.project,
            platform = %platform_name,
            session_key = %message.session_key,
            agent = %self.agent.name(),
            agent_session_id = %session.current_session_id(),
            final_len = final_text.len(),
            preview = %super::log_preview(&final_text, 100),
            "agent final result ready"
        );
        if !final_text.is_empty() {
            platform.send(message.reply_ctx.clone(), final_text).await?;
            tracing::info!(
                project = %self.project,
                platform = %platform_name,
                session_key = %message.session_key,
                "agent final result sent to channel"
            );
        }
        Ok(())
    }

    pub(super) async fn send_agent_error(
        &self,
        platform: Arc<dyn Platform>,
        message: &Message,
        platform_name: &str,
        session: Arc<dyn AgentSession>,
        event: crate::core::Event,
    ) -> Result<()> {
        let error = event.error.unwrap_or_else(|| event.content.clone());
        tracing::error!(
            project = %self.project,
            platform = %platform_name,
            session_key = %message.session_key,
            agent = %self.agent.name(),
            agent_session_id = %session.current_session_id(),
            error = %error,
            "agent returned error"
        );
        platform
            .send(message.reply_ctx.clone(), format!("Agent error: {error}"))
            .await?;
        Ok(())
    }
}
