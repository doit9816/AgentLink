use crate::core::{Event, EventType};
use serde_json::Value;

pub(super) fn map_session_update(session_id: &str, params: &Value) -> Vec<Event> {
    let wrap = match params.get("update") {
        Some(update) if !update.is_null() => update,
        _ => return Vec::new(),
    };
    let sid = params
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(session_id);
    let kind = wrap
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "agent_message_chunk" => map_agent_message_chunk(sid, wrap),
        "tool_call" => map_tool_call(sid, wrap),
        "tool_call_update" => map_tool_call_update(sid, wrap),
        "plan" => map_plan(sid, wrap),
        "user_message_chunk" => Vec::new(),
        other => map_session_update_fallback(sid, other, wrap),
    }
}

fn map_agent_message_chunk(session_id: &str, update: &Value) -> Vec<Event> {
    let text = update
        .pointer("/content/text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if text.is_empty() {
        return Vec::new();
    }
    vec![text_event(session_id, text)]
}

fn map_tool_call(session_id: &str, update: &Value) -> Vec<Event> {
    let tool_name = update
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| update.get("kind").and_then(Value::as_str))
        .unwrap_or("tool");
    let kind = update
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let raw = update.get("rawInput").cloned().unwrap_or(Value::Null);
    let mut tool_input = summarize_acp_tool_input(kind, &raw);
    if tool_input.is_empty() {
        tool_input = tool_name.to_string();
    }
    vec![Event {
        event_type: EventType::ToolUse,
        content: String::new(),
        tool_name: Some(tool_name.to_string()),
        tool_input: Some(tool_input),
        tool_input_raw: None,
        tool_result: None,
        session_id: Some(session_id.to_string()),
        request_id: None,
        done: false,
        error: None,
        metadata: None,
    }]
}

fn map_tool_call_update(session_id: &str, update: &Value) -> Vec<Event> {
    let tool_label = update
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| update.get("toolCallId").and_then(Value::as_str))
        .unwrap_or("tool");
    let body = extract_tool_call_content_text(update.get("content"));
    let status = update
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let content = match status.as_str() {
        "completed" | "failed" => {
            if body.is_empty() && status == "completed" {
                return Vec::new();
            }
            if body.is_empty() && status == "failed" {
                "(failed)".to_string()
            } else {
                body
            }
        }
        "in_progress" | "pending" => {
            if body.is_empty() {
                return Vec::new();
            }
            body
        }
        _ => {
            if body.is_empty() {
                return Vec::new();
            }
            body
        }
    };
    vec![Event {
        event_type: EventType::ToolResult,
        content: truncate_runes(&content, 800),
        tool_name: Some(tool_label.to_string()),
        tool_input: None,
        tool_input_raw: None,
        tool_result: None,
        session_id: Some(session_id.to_string()),
        request_id: None,
        done: false,
        error: None,
        metadata: None,
    }]
}

fn extract_tool_call_content_text(blocks: Option<&Value>) -> String {
    let Some(Value::Array(items)) = blocks else {
        return String::new();
    };
    let mut out = String::new();
    for item in items {
        if let Some(text) = item.pointer("/content/text").and_then(Value::as_str) {
            if !text.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
    }
    out
}

fn map_session_update_fallback(session_id: &str, kind: &str, update: &Value) -> Vec<Event> {
    match kind.to_ascii_lowercase().as_str() {
        "reasoning" | "reasoning_chunk" | "thinking" | "agent_thinking_chunk" => {
            let text = update
                .pointer("/content/text")
                .and_then(Value::as_str)
                .or_else(|| update.get("text").and_then(Value::as_str))
                .unwrap_or_default();
            if text.is_empty() {
                return Vec::new();
            }
            vec![Event {
                event_type: EventType::Thinking,
                content: text.to_string(),
                tool_name: None,
                tool_input: None,
                tool_input_raw: None,
                tool_result: None,
                session_id: Some(session_id.to_string()),
                request_id: None,
                done: false,
                error: None,
                metadata: None,
            }]
        }
        _ => Vec::new(),
    }
}

fn map_plan(session_id: &str, update: &Value) -> Vec<Event> {
    let Some(entries) = update.get("entries").and_then(Value::as_array) else {
        return Vec::new();
    };
    if entries.is_empty() {
        return Vec::new();
    }
    let mut body = String::new();
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            body.push('\n');
        }
        let content = entry
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some(status) = entry.get("status").and_then(Value::as_str) {
            if !status.is_empty() {
                body.push('[');
                body.push_str(status);
                body.push_str("] ");
            }
        }
        body.push_str(content);
    }
    vec![Event {
        event_type: EventType::Thinking,
        content: body,
        tool_name: None,
        tool_input: None,
        tool_input_raw: None,
        tool_result: None,
        session_id: Some(session_id.to_string()),
        request_id: None,
        done: false,
        error: None,
        metadata: None,
    }]
}

fn text_event(session_id: &str, text: &str) -> Event {
    Event {
        event_type: EventType::Text,
        content: text.to_string(),
        tool_name: None,
        tool_input: None,
        tool_input_raw: None,
        tool_result: None,
        session_id: Some(session_id.to_string()),
        request_id: None,
        done: false,
        error: None,
        metadata: None,
    }
}

pub(super) fn summarize_acp_tool_input(kind: &str, raw: &Value) -> String {
    if raw.is_null() {
        return String::new();
    }
    let Some(obj) = raw.as_object() else {
        return raw.to_string();
    };
    if obj.is_empty() {
        return String::new();
    }
    match kind.to_ascii_lowercase().as_str() {
        "bash" | "shell" | "terminal" | "execute" => {
            if let Some(cmd) = obj.get("command").and_then(Value::as_str) {
                if let Some(desc) = obj.get("description").and_then(Value::as_str) {
                    if !desc.is_empty() {
                        return format!("# {desc}\n{cmd}");
                    }
                }
                return cmd.to_string();
            }
        }
        "read" | "write" | "edit" => {
            if let Some(fp) = obj
                .get("file_path")
                .or_else(|| obj.get("path"))
                .and_then(Value::as_str)
            {
                return fp.to_string();
            }
        }
        _ => {}
    }
    if let Some(cmd) = obj.get("command").and_then(Value::as_str) {
        if let Some(desc) = obj.get("description").and_then(Value::as_str) {
            if !desc.is_empty() {
                return format!("# {desc}\n{cmd}");
            }
        }
        return cmd.to_string();
    }
    serde_json::to_string_pretty(raw).unwrap_or_else(|_| raw.to_string())
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(super) struct PermissionOption {
    #[serde(rename = "optionId")]
    pub option_id: String,
    pub name: String,
    pub kind: String,
}

pub(super) fn pick_permission_option_id(allow: bool, options: &[PermissionOption]) -> String {
    if options.is_empty() {
        return String::new();
    }
    if allow {
        for opt in options {
            if opt.kind.to_ascii_lowercase().contains("allow") {
                return opt.option_id.clone();
            }
        }
        for opt in options {
            if opt.name.to_ascii_lowercase().contains("allow") {
                return opt.option_id.clone();
            }
        }
        return options[0].option_id.clone();
    }
    for opt in options {
        let kind = opt.kind.to_ascii_lowercase();
        if kind.contains("reject") || kind.contains("deny") {
            return opt.option_id.clone();
        }
    }
    for opt in options {
        let name = opt.name.to_ascii_lowercase();
        if name.contains("reject") || name.contains("deny") {
            return opt.option_id.clone();
        }
    }
    options
        .last()
        .map(|o| o.option_id.clone())
        .unwrap_or_default()
}

pub(super) fn build_permission_result(allow: bool, option_id: &str) -> Value {
    if !allow {
        if option_id.is_empty() {
            return serde_json::json!({ "outcome": { "outcome": "cancelled" } });
        }
        return serde_json::json!({
            "outcome": { "outcome": "selected", "optionId": option_id }
        });
    }
    serde_json::json!({
        "outcome": { "outcome": "selected", "optionId": option_id }
    })
}

fn truncate_runes(s: &str, max: usize) -> String {
    let runes: Vec<char> = s.chars().collect();
    if runes.len() <= max {
        return s.to_string();
    }
    runes[..max].iter().collect::<String>() + "..."
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_agent_message_chunk_to_text_event() {
        let params = json!({
            "sessionId": "s1",
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": { "type": "text", "text": "hello" }
            }
        });
        let events = map_session_update("fallback", &params);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::Text);
        assert_eq!(events[0].content, "hello");
    }

    #[test]
    fn summarizes_bash_tool_input() {
        let raw = json!({ "command": "ls", "description": "list" });
        assert_eq!(summarize_acp_tool_input("bash", &raw), "# list\nls");
    }
}
