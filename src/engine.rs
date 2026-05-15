use crate::core::{
    new_approval_id, parse_approval_command, Agent, AgentSession, AttachmentKind, EventType,
    Message, MessageHandler, PermissionBehavior, PermissionResult, Platform, ReplyContext,
};
use crate::store::{unix_now, ApprovalRecord, SessionStore, TargetRecord};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

pub struct Engine {
    project: String,
    agent: Arc<dyn Agent>,
    platforms: Vec<Arc<dyn Platform>>,
    store: SessionStore,
    sessions: Mutex<HashMap<String, Arc<dyn AgentSession>>>,
    session_queues: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    approvals: Mutex<HashMap<String, ApprovalRuntime>>,
    turn_timeout: Duration,
}

struct ApprovalRuntime {
    session: Arc<dyn AgentSession>,
}

impl Engine {
    pub fn new(
        project: impl Into<String>,
        agent: Arc<dyn Agent>,
        platforms: Vec<Arc<dyn Platform>>,
        store: SessionStore,
    ) -> Arc<Self> {
        Arc::new(Self {
            project: project.into(),
            agent,
            platforms,
            store,
            sessions: Mutex::new(HashMap::new()),
            session_queues: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
            turn_timeout: Duration::from_secs(600),
        })
    }

    pub async fn start(self: &Arc<Self>) -> Result<()> {
        let handler: MessageHandler = {
            let engine = Arc::clone(self);
            Arc::new(move |platform, message| {
                let engine = Arc::clone(&engine);
                Box::pin(async move {
                    let reply_ctx = message.reply_ctx.clone();
                    let platform_for_error = Arc::clone(&platform);
                    if let Err(err) = engine.handle_message(platform, message).await {
                        tracing::error!(error = %err, "handle message failed");
                        let _ = platform_for_error
                            .reply(reply_ctx, format!("Bridge 处理消息失败：{err}"))
                            .await;
                    }
                })
            })
        };
        for platform in &self.platforms {
            platform.start(Arc::clone(&handler)).await?;
        }
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        for platform in &self.platforms {
            let _ = platform.stop().await;
        }
        for session in self.sessions.lock().await.values() {
            let _ = session.close().await;
        }
        self.agent.stop().await?;
        Ok(())
    }

    pub fn with_turn_timeout(self: Arc<Self>, turn_timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            project: self.project.clone(),
            agent: Arc::clone(&self.agent),
            platforms: self.platforms.clone(),
            store: self.store.clone(),
            sessions: Mutex::new(HashMap::new()),
            session_queues: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
            turn_timeout,
        })
    }

    pub async fn handle_message(
        &self,
        platform: Arc<dyn Platform>,
        mut message: Message,
    ) -> Result<()> {
        if message.platform.is_none() {
            message.platform = Some(platform.name().to_string());
        }
        tracing::info!(
            project = %self.project,
            platform = %message.platform.as_deref().unwrap_or_else(|| platform.name()),
            session_key = %message.session_key,
            user_id = %message.user_id,
            message_type = ?message.message_type,
            content_len = message.content.len(),
            attachments = message.attachments.len(),
            images = message.images.len(),
            files = message.files.len(),
            preview = %log_preview(&message.content, 80),
            "chat message received"
        );
        if let Some(command) = parse_approval_command(&message.content) {
            return self
                .handle_approval_command(platform, message, command.approval_id, command.decision)
                .await;
        }
        if message.content.trim().is_empty()
            && message.images.is_empty()
            && message.files.is_empty()
            && message.attachments.is_empty()
        {
            return Ok(());
        }
        let platform_name = message
            .platform
            .clone()
            .unwrap_or_else(|| platform.name().to_string());
        self.store.record_target(&TargetRecord {
            project: self.project.clone(),
            platform: platform_name.clone(),
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
        let queue = self
            .session_queue(&platform_name, &message.session_key)
            .await;
        let _guard = queue.lock().await;
        self.process_user_message(platform, message).await
    }

    async fn session_queue(&self, platform: &str, session_key: &str) -> Arc<Mutex<()>> {
        let key = format!("{platform}:{session_key}");
        let mut queues = self.session_queues.lock().await;
        queues
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    async fn process_user_message(
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
                    let request_id = event
                        .request_id
                        .clone()
                        .ok_or_else(|| anyhow!("permission request missing request_id"))?;
                    let approval_id = new_approval_id();
                    let summary = event
                        .tool_input
                        .clone()
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| event.content.clone());
                    let record = ApprovalRecord {
                        approval_id: approval_id.clone(),
                        project: self.project.clone(),
                        platform: platform_name.clone(),
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
                        "Agent 请求执行操作，需要审批：\n\n审批编号：{}\n类型：{}\n内容：{}\n\n回复：/allow {} 允许，或 /deny {} 拒绝",
                        approval_id,
                        event.tool_name.unwrap_or_else(|| "tool".to_string()),
                        summary,
                        approval_id,
                        approval_id
                    );
                    platform.send(message.reply_ctx.clone(), prompt).await?;
                }
                EventType::Text => fallback.push_str(&event.content),
                EventType::Result => {
                    self.store.upsert_session(
                        &self.project,
                        &platform_name,
                        &message.session_key,
                        self.agent.name(),
                        &session.current_session_id(),
                    )?;
                    let final_text = if event.content.trim().is_empty() {
                        fallback.trim().to_string()
                    } else {
                        event.content.trim().to_string()
                    };
                    tracing::info!(
                        project = %self.project,
                        platform = %platform_name,
                        session_key = %message.session_key,
                        agent = %self.agent.name(),
                        agent_session_id = %session.current_session_id(),
                        final_len = final_text.len(),
                        preview = %log_preview(&final_text, 100),
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
                    return Ok(());
                }
                EventType::Error => {
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
                        .send(message.reply_ctx.clone(), format!("Agent 错误：{error}"))
                        .await?;
                    return Ok(());
                }
                _ => {}
            }
        }
    }

    async fn running_session(
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

    async fn handle_approval_command(
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
                .reply(message.reply_ctx, format!("审批编号不存在：{approval_id}"))
                .await?;
            return Ok(());
        };
        if record.project != self.project
            || record.platform != platform_name
            || record.session_key != message.session_key
        {
            platform
                .reply(message.reply_ctx, "审批编号不属于当前会话。".to_string())
                .await?;
            return Ok(());
        }
        if record.status != "pending" || unix_now() > record.expires_at {
            platform
                .reply(
                    message.reply_ctx,
                    format!("审批已处理或已过期：{approval_id}"),
                )
                .await?;
            return Ok(());
        }
        let Some(runtime) = self.approvals.lock().await.remove(&approval_id) else {
            platform
                .reply(
                    message.reply_ctx,
                    "审批对应的 Agent 会话不在线。".to_string(),
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
                format!("审批已提交：{decision_text}"),
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
}

#[allow(dead_code)]
fn _reply_ctx(_: ReplyContext) {}

fn log_preview(value: &str, max_chars: usize) -> String {
    let mut preview = value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if preview.chars().count() > max_chars {
        preview = preview.chars().take(max_chars).collect::<String>();
        preview.push_str("...");
    }
    preview
}

fn prompt_with_attachments(message: &Message) -> String {
    let mut prompt = if message.content.trim().is_empty() {
        format!("[{:?} message]", message.message_type)
    } else {
        message.content.clone()
    };
    let mut lines = Vec::new();
    for (index, attachment) in message.attachments.iter().enumerate() {
        let kind = match attachment.kind {
            AttachmentKind::File => "file",
            AttachmentKind::Image => "image",
            AttachmentKind::Audio => "audio",
            AttachmentKind::Video => "video",
            AttachmentKind::Location => "location",
            AttachmentKind::Card => "card",
            AttachmentKind::Sticker => "sticker",
            AttachmentKind::Raw => "raw",
        };
        let name = attachment
            .file_name
            .as_deref()
            .or(attachment.path.as_deref())
            .or(attachment.url.as_deref())
            .unwrap_or("");
        let metadata_text = attachment.metadata.as_ref().map(|v| v.to_string());
        let detail = attachment
            .text
            .as_deref()
            .or(metadata_text.as_deref())
            .unwrap_or("");
        lines.push(format!(
            "{}. type={}, mime={}, name={}, detail={}",
            index + 1,
            kind,
            attachment.mime_type.as_deref().unwrap_or(""),
            name,
            detail
        ));
    }
    if !message.images.is_empty() {
        lines.push(format!("legacy_images={}", message.images.len()));
    }
    if !message.files.is_empty() {
        lines.push(format!("legacy_files={}", message.files.len()));
    }
    if !lines.is_empty() {
        prompt.push_str("\n\n[attachments]\n");
        prompt.push_str(&lines.join("\n"));
    }
    prompt
}
