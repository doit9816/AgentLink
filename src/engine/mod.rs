mod runtime;

use crate::core::{
    parse_approval_command, parse_session_command, Agent, AgentSession, Message, MessageHandler,
    Platform,
};
use crate::store::SessionStore;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::Duration;

pub struct Engine {
    project: String,
    default_work_dir: String,
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
        default_work_dir: impl Into<String>,
        agent: Arc<dyn Agent>,
        platforms: Vec<Arc<dyn Platform>>,
        store: SessionStore,
    ) -> Arc<Self> {
        Arc::new(Self {
            project: project.into(),
            default_work_dir: default_work_dir.into(),
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
                    let platform_name = platform.name().to_string();
                    let session_key = message.session_key.clone();
                    let reply_ctx = message.reply_ctx.clone();
                    let platform_for_error = Arc::clone(&platform);
                    if let Err(err) = engine.handle_message(platform, message).await {
                        tracing::error!(
                            project = %engine.project,
                            platform = %platform_name,
                            session_key = %session_key,
                            error = %err,
                            "channel to agent handling failed"
                        );
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
            default_work_dir: self.default_work_dir.clone(),
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
            preview = %runtime::log_preview(&message.content, 80),
            "chat message received"
        );
        if let Some(command) = parse_approval_command(&message.content) {
            return self
                .handle_approval_command(platform, message, command.approval_id, command.decision)
                .await;
        }
        if let Some(command) = parse_session_command(&message.content) {
            return self
                .handle_session_command(platform, message, command)
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
        self.record_target(&platform_name, &message)?;
        let queue = self
            .session_queue(&platform_name, &message.session_key)
            .await;
        let _guard = queue.lock().await;
        self.process_user_message(platform, message).await
    }

    pub(crate) fn ensure_conversation_slots(
        &self,
        platform: &str,
        session_key: &str,
    ) -> anyhow::Result<()> {
        use crate::store::ConversationSlot;
        let slots = self
            .store
            .list_conversation_slots(&self.project, platform, session_key)?;
        if !slots.is_empty() {
            return Ok(());
        }
        if let Some(legacy) = self
            .store
            .get_session(&self.project, platform, session_key)?
        {
            let now = crate::store::unix_now();
            let slot = ConversationSlot {
                project: self.project.clone(),
                platform: platform.to_string(),
                session_key: session_key.to_string(),
                slot_id: "1".to_string(),
                label: String::new(),
                agent_session_id: legacy.agent_session_id,
                is_active: true,
                created_at: now,
                updated_at: now,
            };
            self.store.insert_conversation_slot(&slot)?;
        }
        Ok(())
    }

    pub(crate) fn effective_work_dir(&self, platform: &str, session_key: &str) -> anyhow::Result<String> {
        let prefs = self
            .store
            .get_conversation_prefs(&self.project, platform, session_key)?;
        if let Some(dir) = prefs.work_dir_override {
            return Ok(dir);
        }
        Ok(self.default_work_dir.clone())
    }

    pub(crate) fn runtime_session_key(&self, platform: &str, session_key: &str) -> String {
        format!("{platform}:{session_key}")
    }

    pub(crate) async fn drop_runtime_session(&self, platform: &str, session_key: &str) {
        let key = self.runtime_session_key(platform, session_key);
        if let Some(session) = self.sessions.lock().await.remove(&key) {
            let _ = session.close().await;
        }
    }
}
