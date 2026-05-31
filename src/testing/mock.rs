use crate::core::{
    Agent, AgentCapabilities, AgentSession, AgentSessionInfo, Event, FileAttachment,
    ImageAttachment, Message, MessageHandler, PermissionBehavior, PermissionResult, Platform,
    ReplyContext, SessionStartRequest,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

#[derive(Default)]
pub struct MockAgent {
    counter: AtomicU64,
}

impl MockAgent {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Agent for MockAgent {
    fn name(&self) -> &str {
        "mock"
    }

    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            images: true,
            files: true,
            approvals: true,
            session_list: true,
            ..AgentCapabilities::default()
        }
    }

    async fn start_session(&self, request: SessionStartRequest) -> Result<Arc<dyn AgentSession>> {
        let id = request.resume_session_id.unwrap_or_else(|| {
            format!(
                "mock-session-{}",
                self.counter.fetch_add(1, Ordering::SeqCst) + 1
            )
        });
        Ok(Arc::new(MockSession::new(id)))
    }

    async fn list_sessions(&self) -> Result<Vec<AgentSessionInfo>> {
        Ok(Vec::new())
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }
}

pub struct MockSession {
    id: String,
    alive: AtomicBool,
    tx: mpsc::UnboundedSender<Event>,
    rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<Event>>,
    approvals: Mutex<HashMap<String, oneshot::Sender<PermissionResult>>>,
}

impl MockSession {
    fn new(id: String) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            id,
            alive: AtomicBool::new(true),
            tx,
            rx: tokio::sync::Mutex::new(rx),
            approvals: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl AgentSession for MockSession {
    async fn send(
        &self,
        prompt: String,
        images: Vec<ImageAttachment>,
        files: Vec<FileAttachment>,
    ) -> Result<()> {
        if !self.alive() {
            return Err(anyhow!("mock session is closed"));
        }
        if prompt.to_ascii_lowercase().contains("approval") || prompt.contains("审批") {
            let request_id = "mock-approval-1".to_string();
            let (tx, rx) = oneshot::channel();
            self.approvals
                .lock()
                .unwrap()
                .insert(request_id.clone(), tx);
            self.tx.send(Event::permission_request(
                request_id,
                "mock-tool",
                "mock protected action",
            ))?;
            let event_tx = self.tx.clone();
            let id = self.id.clone();
            tokio::spawn(async move {
                let result = rx.await;
                let content = match result {
                    Ok(PermissionResult {
                        behavior: PermissionBehavior::Allow,
                        ..
                    }) => format!("mock approved: {prompt}"),
                    _ => format!("mock denied: {prompt}"),
                };
                let _ = event_tx.send(Event::result(content, id));
            });
        } else {
            let mut prompt = prompt;
            if !images.is_empty() || !files.is_empty() {
                prompt = format!("{prompt} [images={}, files={}]", images.len(), files.len());
            }
            self.tx
                .send(Event::result(format!("mock: {prompt}"), self.id.clone()))?;
        }
        Ok(())
    }

    async fn respond_permission(&self, request_id: String, result: PermissionResult) -> Result<()> {
        let Some(tx) = self.approvals.lock().unwrap().remove(&request_id) else {
            return Err(anyhow!("mock approval request `{request_id}` not found"));
        };
        tx.send(result)
            .map_err(|_| anyhow!("mock approval receiver closed"))?;
        Ok(())
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundMessage {
    pub kind: String,
    pub reply_ctx: ReplyContext,
    pub content: String,
}

pub struct MockPlatform {
    name: String,
    handler: Mutex<Option<MessageHandler>>,
    out: Mutex<Vec<OutboundMessage>>,
    out_tx: mpsc::UnboundedSender<OutboundMessage>,
    out_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<OutboundMessage>>,
}

impl MockPlatform {
    pub fn new(name: impl Into<String>) -> Arc<Self> {
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        Arc::new(Self {
            name: name.into(),
            handler: Mutex::new(None),
            out: Mutex::new(Vec::new()),
            out_tx,
            out_rx: tokio::sync::Mutex::new(out_rx),
        })
    }

    pub async fn inject(self: &Arc<Self>, mut message: Message) -> Result<()> {
        let Some(handler) = self.handler.lock().unwrap().clone() else {
            return Err(anyhow!("mock platform is not started"));
        };
        if message.platform.is_none() {
            message.platform = Some(self.name.clone());
        }
        let platform = Arc::clone(self) as Arc<dyn Platform>;
        tokio::spawn(async move {
            handler(platform, message).await;
        });
        Ok(())
    }

    pub async fn wait_for_outbound(&self) -> Option<OutboundMessage> {
        self.out_rx.lock().await.recv().await
    }

    pub fn outbound(&self) -> Vec<OutboundMessage> {
        self.out.lock().unwrap().clone()
    }

    fn record(&self, kind: &str, reply_ctx: ReplyContext, content: String) {
        let msg = OutboundMessage {
            kind: kind.to_string(),
            reply_ctx,
            content,
        };
        self.out.lock().unwrap().push(msg.clone());
        let _ = self.out_tx.send(msg);
    }
}

#[async_trait]
impl Platform for MockPlatform {
    fn name(&self) -> &str {
        &self.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().unwrap() = Some(handler);
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        self.record("reply", reply_ctx, content);
        Ok(())
    }

    async fn send(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        self.record("send", reply_ctx, content);
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }
}
