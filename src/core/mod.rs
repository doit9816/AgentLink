use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

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
    async fn start_session(&self, session_id: Option<String>) -> Result<Arc<dyn AgentSession>>;
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCapabilities {
    pub images: bool,
    pub files: bool,
    pub approvals: bool,
    pub model_switching: bool,
    pub session_list: bool,
    pub context_usage: bool,
    pub streaming_output: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplyContext {
    pub value: String,
}

impl From<&str> for ReplyContext {
    fn from(value: &str) -> Self {
        Self {
            value: value.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub project: Option<String>,
    pub session_key: String,
    pub platform: Option<String>,
    pub message_id: Option<String>,
    #[serde(default)]
    pub message_type: MessageType,
    pub user_id: String,
    pub user_name: Option<String>,
    pub content: String,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    pub images: Vec<ImageAttachment>,
    pub files: Vec<FileAttachment>,
    pub reply_ctx: ReplyContext,
    #[serde(skip, default = "SystemTime::now")]
    pub created_at: SystemTime,
}

impl Message {
    pub fn text(
        session_key: impl Into<String>,
        user_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            project: None,
            session_key: session_key.into(),
            platform: None,
            message_id: None,
            message_type: MessageType::Text,
            user_id: user_id.into(),
            user_name: None,
            content: content.into(),
            attachments: Vec::new(),
            images: Vec::new(),
            files: Vec::new(),
            reply_ctx: ReplyContext::default(),
            created_at: SystemTime::now(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    #[default]
    Text,
    Image,
    File,
    Audio,
    Video,
    Location,
    Card,
    Sticker,
    Mixed,
    Event,
    Raw,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attachment {
    pub kind: AttachmentKind,
    pub mime_type: Option<String>,
    pub data: Option<Vec<u8>>,
    pub url: Option<String>,
    pub path: Option<String>,
    pub file_name: Option<String>,
    pub text: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    #[default]
    File,
    Image,
    Audio,
    Video,
    Location,
    Card,
    Sticker,
    Raw,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageAttachment {
    pub mime_type: String,
    pub data: Vec<u8>,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileAttachment {
    pub mime_type: String,
    pub data: Vec<u8>,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Text,
    Thinking,
    ToolUse,
    ToolResult,
    PermissionRequest,
    Result,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub event_type: EventType,
    pub content: String,
    pub tool_name: Option<String>,
    pub tool_input: Option<String>,
    pub tool_input_raw: Option<serde_json::Value>,
    pub tool_result: Option<String>,
    pub session_id: Option<String>,
    pub request_id: Option<String>,
    pub done: bool,
    pub error: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

impl Event {
    pub fn result(content: impl Into<String>, session_id: impl Into<String>) -> Self {
        Self {
            event_type: EventType::Result,
            content: content.into(),
            tool_name: None,
            tool_input: None,
            tool_input_raw: None,
            tool_result: None,
            session_id: Some(session_id.into()),
            request_id: None,
            done: true,
            error: None,
            metadata: None,
        }
    }

    pub fn permission_request(
        request_id: impl Into<String>,
        tool_name: impl Into<String>,
        tool_input: impl Into<String>,
    ) -> Self {
        Self {
            event_type: EventType::PermissionRequest,
            content: String::new(),
            tool_name: Some(tool_name.into()),
            tool_input: Some(tool_input.into()),
            tool_input_raw: None,
            tool_result: None,
            session_id: None,
            request_id: Some(request_id.into()),
            done: false,
            error: None,
            metadata: None,
        }
    }

    pub fn error(error: impl Into<String>) -> Self {
        Self {
            event_type: EventType::Error,
            content: String::new(),
            tool_name: None,
            tool_input: None,
            tool_input_raw: None,
            tool_result: None,
            session_id: None,
            request_id: None,
            done: true,
            error: Some(error.into()),
            metadata: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionResult {
    pub behavior: PermissionBehavior,
    pub message: Option<String>,
    pub updated_input: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionBehavior {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSessionInfo {
    pub id: String,
    pub summary: Option<String>,
    pub message_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalCommand {
    pub decision: PermissionBehavior,
    pub approval_id: String,
}

pub fn parse_approval_command(content: &str) -> Option<ApprovalCommand> {
    let mut parts = content.split_whitespace();
    let command = parts.next()?.to_ascii_lowercase();
    let approval_id = parts.next()?.to_string();
    if parts.next().is_some() {
        return None;
    }
    match command.as_str() {
        "/allow" => Some(ApprovalCommand {
            decision: PermissionBehavior::Allow,
            approval_id,
        }),
        "/deny" => Some(ApprovalCommand {
            decision: PermissionBehavior::Deny,
            approval_id,
        }),
        _ => None,
    }
}

pub fn new_approval_id() -> String {
    use rand::RngCore;
    let mut bytes = [0_u8; 4];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!(
        "apv_{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3]
    )
}
