use serde::{Deserialize, Serialize};

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
