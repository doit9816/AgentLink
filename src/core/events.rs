use serde::{Deserialize, Serialize};

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
