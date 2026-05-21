use super::models::ReplyContext;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

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
