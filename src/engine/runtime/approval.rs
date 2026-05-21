use super::{ApprovalRuntime, Engine};
use crate::core::{
    new_approval_id, AgentSession, Event, Message, PermissionBehavior, PermissionResult, Platform,
};
use crate::store::unix_now;
use anyhow::Result;
use std::sync::Arc;

impl Engine {
    pub(crate) async fn handle_approval_command(
        &self,
        platform: Arc<dyn Platform>,
        message: Message,
        approval_id: String,
        decision: PermissionBehavior,
    ) -> Result<()> {
        let platform_name = message
            .platform
            .clone()
            .unwrap_or_else(|| platform.name().to_string());
        let Some(record) = self.store.get_approval(&approval_id)? else {
            platform
                .reply(
                    message.reply_ctx,
                    format!("Unknown approval id: {approval_id}"),
                )
                .await?;
            return Ok(());
        };
        if record.project != self.project
            || record.platform != platform_name
            || record.session_key != message.session_key
        {
            platform
                .reply(
                    message.reply_ctx,
                    "Approval id does not belong to this session.".to_string(),
                )
                .await?;
            return Ok(());
        }
        if record.status != "pending" || unix_now() > record.expires_at {
            platform
                .reply(
                    message.reply_ctx,
                    format!("Approval already handled or expired: {approval_id}"),
                )
                .await?;
            return Ok(());
        }
        let Some(runtime) = self.approvals.lock().await.remove(&approval_id) else {
            platform
                .reply(
                    message.reply_ctx,
                    "Approval exists, but the agent session is no longer online.".to_string(),
                )
                .await?;
            return Ok(());
        };
        let decision_text = match decision {
            PermissionBehavior::Allow => "allow",
            PermissionBehavior::Deny => "deny",
        };
        tracing::info!(
            project = %self.project,
            platform = %platform_name,
            session_key = %message.session_key,
            approval_id = %approval_id,
            decision = %decision_text,
            "agent permission response received from channel"
        );
        self.store.resolve_approval(&approval_id, decision_text)?;
        platform
            .reply(
                message.reply_ctx.clone(),
                format!("\u{5ba1}\u{6279}\u{5df2}\u{63d0}\u{4ea4}\u{ff1a}{decision_text}"),
            )
            .await?;
        runtime
            .session
            .respond_permission(
                record.agent_request_id,
                PermissionResult {
                    behavior: decision,
                    message: None,
                    updated_input: None,
                },
            )
            .await?;
        tracing::info!(
            project = %self.project,
            platform = %platform_name,
            session_key = %message.session_key,
            approval_id = %approval_id,
            decision = %decision_text,
            "agent permission response sent"
        );
        Ok(())
    }

    pub(super) async fn handle_permission_request(
        &self,
        platform: Arc<dyn Platform>,
        message: &Message,
        platform_name: &str,
        session: Arc<dyn AgentSession>,
        event: Event,
    ) -> Result<()> {
        let request_id = event
            .request_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("permission request missing request_id"))?;
        let approval_id = new_approval_id();
        let summary = event
            .tool_input
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| event.content.clone());
        let record = crate::store::ApprovalRecord {
            approval_id: approval_id.clone(),
            project: self.project.clone(),
            platform: platform_name.to_string(),
            session_key: message.session_key.clone(),
            agent_session_id: session.current_session_id(),
            agent_request_id: request_id,
            status: "pending".to_string(),
            summary: summary.clone(),
            expires_at: unix_now() + 300,
            decision: None,
        };
        self.store.create_approval(&record)?;
        tracing::info!(
            project = %self.project,
            platform = %platform_name,
            session_key = %message.session_key,
            agent = %self.agent.name(),
            agent_session_id = %session.current_session_id(),
            approval_id = %approval_id,
            tool_name = %event.tool_name.as_deref().unwrap_or("tool"),
            summary_len = summary.len(),
            "agent permission requested"
        );
        self.approvals.lock().await.insert(
            approval_id.clone(),
            ApprovalRuntime {
                session: Arc::clone(&session),
            },
        );
        let prompt = format!(
            "Agent \u{8bf7}\u{6c42}\u{6267}\u{884c}\u{64cd}\u{4f5c}\u{ff0c}\u{9700}\u{8981}\u{5ba1}\u{6279}\u{ff1a}\n\n\u{5ba1}\u{6279}\u{7f16}\u{53f7}\u{ff1a}{}\n\u{7c7b}\u{578b}\u{ff1a}{}\n\u{5185}\u{5bb9}\u{ff1a}{}\n\n\u{56de}\u{590d}\u{ff1a}/allow {} \u{5141}\u{8bb8}\u{ff0c}\u{6216} /deny {} \u{62d2}\u{7edd}",
            approval_id,
            event.tool_name.unwrap_or_else(|| "tool".to_string()),
            summary,
            approval_id,
            approval_id
        );
        platform.send(message.reply_ctx.clone(), prompt).await?;
        Ok(())
    }
}
