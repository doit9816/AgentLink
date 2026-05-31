use super::events::Event;
use super::messages::{FileAttachment, ImageAttachment, Message};
use super::models::{AgentCapabilities, AgentSessionInfo, PermissionResult, ReplyContext};
use super::session_start::SessionStartRequest;
use anyhow::Result;
use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type MessageHandler = Arc<
    dyn Fn(Arc<dyn Platform>, Message) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync,
>;

#[async_trait]
pub trait Platform: Send + Sync {
    fn name(&self) -> &str;
    async fn start(&self, handler: MessageHandler) -> Result<()>;
    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()>;
    async fn send(&self, reply_ctx: ReplyContext, content: String) -> Result<()>;
    async fn stop(&self) -> Result<()>;
}

#[async_trait]
pub trait Agent: Send + Sync {
    fn name(&self) -> &str;
    fn capabilities(&self) -> AgentCapabilities;
    async fn start_session(&self, request: SessionStartRequest) -> Result<Arc<dyn AgentSession>>;
    async fn list_sessions(&self) -> Result<Vec<AgentSessionInfo>>;
    async fn stop(&self) -> Result<()>;
}

#[async_trait]
pub trait AgentSession: Send + Sync {
    async fn send(
        &self,
        prompt: String,
        images: Vec<ImageAttachment>,
        files: Vec<FileAttachment>,
    ) -> Result<()>;
    async fn respond_permission(&self, request_id: String, result: PermissionResult) -> Result<()>;
    async fn recv_event(&self) -> Option<Event>;
    fn current_session_id(&self) -> String;
    fn alive(&self) -> bool;
    async fn close(&self) -> Result<()>;
}
