mod approval;
mod prompt;
mod session;

use super::{ApprovalRuntime, Engine};
use crate::core::{EventType, Message, Platform};
use anyhow::{anyhow, Result};
use std::sync::Arc;
use tokio::time::timeout;

pub(crate) use prompt::log_preview;
use prompt::prompt_with_attachments;

impl Engine {
    pub(super) async fn process_user_message(
        &self,
        platform: Arc<dyn Platform>,
        message: Message,
    ) -> Result<()> {
        let platform_name = message
            .platform
            .clone()
            .unwrap_or_else(|| platform.name().to_string());
        let session = self
            .running_session(&platform_name, &message.session_key)
            .await?;
        let prompt = prompt_with_attachments(&message);
        tracing::info!(
            project = %self.project,
            platform = %platform_name,
            session_key = %message.session_key,
            agent = %self.agent.name(),
            agent_session_id = %session.current_session_id(),
            prompt_len = prompt.len(),
            images = message.images.len(),
            files = message.files.len(),
            "agent prompt send start"
        );
        session
            .send(prompt, message.images.clone(), message.files.clone())
            .await?;
        tracing::info!(
            project = %self.project,
            platform = %platform_name,
            session_key = %message.session_key,
            agent = %self.agent.name(),
            agent_session_id = %session.current_session_id(),
            "agent prompt sent"
        );

        let mut fallback = String::new();
        loop {
            let event = timeout(self.turn_timeout, session.recv_event())
                .await
                .map_err(|_| anyhow!("agent turn timed out"))?
                .ok_or_else(|| anyhow!("agent session event stream closed"))?;
            tracing::info!(
                project = %self.project,
                platform = %platform_name,
                session_key = %message.session_key,
                agent = %self.agent.name(),
                agent_session_id = %session.current_session_id(),
                event_type = ?event.event_type,
                content_len = event.content.len(),
                tool_name = %event.tool_name.as_deref().unwrap_or(""),
                done = event.done,
                "agent event received"
            );
            match event.event_type {
                EventType::PermissionRequest => {
                    self.handle_permission_request(
                        Arc::clone(&platform),
                        &message,
                        &platform_name,
                        Arc::clone(&session),
                        event,
                    )
                    .await?;
                }
                EventType::Text => fallback.push_str(&event.content),
                EventType::Result => {
                    self.finish_agent_turn(
                        Arc::clone(&platform),
                        &message,
                        &platform_name,
                        Arc::clone(&session),
                        event.content,
                        fallback.trim().to_string(),
                    )
                    .await?;
                    return Ok(());
                }
                EventType::Error => {
                    self.send_agent_error(
                        platform,
                        &message,
                        &platform_name,
                        Arc::clone(&session),
                        event,
                    )
                    .await?;
                    return Ok(());
                }
                _ => {}
            }
        }
    }
}
