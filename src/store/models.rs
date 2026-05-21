#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub project: String,
    pub platform: String,
    pub session_key: String,
    pub agent_type: String,
    pub agent_session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRecord {
    pub approval_id: String,
    pub project: String,
    pub platform: String,
    pub session_key: String,
    pub agent_session_id: String,
    pub agent_request_id: String,
    pub status: String,
    pub summary: String,
    pub expires_at: u64,
    pub decision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetRecord {
    pub project: String,
    pub platform: String,
    pub session_key: String,
    pub user_id: String,
    pub user_name: Option<String>,
    pub message_id: Option<String>,
    pub reply_ctx: String,
    pub content_preview: String,
    pub updated_at: u64,
}
