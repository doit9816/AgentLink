use crate::core::{Event, EventType};
use serde_json::Value;

#[derive(Debug)]
pub(crate) struct CodexExecOutput {
    pub(crate) final_text: String,
    pub(crate) final_session_id: Option<String>,
    pub(crate) thread_id: Option<String>,
    pub(crate) progress_events: Vec<Event>,
}

pub(crate) fn parse_codex_exec_stdout(stdout: &str) -> CodexExecOutput {
    let mut thread_id = None;
    let mut final_text = String::new();
    let mut pending_messages = Vec::new();
    let mut progress_events = Vec::new();
    let mut plain_lines = Vec::new();

    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            plain_lines.push(line.to_string());
            continue;
        };
        if let Some(id) = value
            .get("thread_id")
            .or_else(|| value.get("threadId"))
            .and_then(Value::as_str)
        {
            thread_id = Some(id.to_string());
        }
        match value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "thread.started" => {
                if let Some(id) = value.get("thread_id").and_then(Value::as_str) {
                    thread_id = Some(id.to_string());
                }
            }
            "item.started" => {
                if let Some(event) = codex_exec_item_started(&value) {
                    progress_events.push(event);
                }
            }
            "item.completed" => {
                if let Some(text) = codex_exec_completed_message(&value) {
                    pending_messages.push(text);
                    continue;
                }
                if let Some(event) = codex_exec_item_completed(&value) {
                    progress_events.push(event);
                }
            }
            "turn.completed" => {
                final_text = pending_messages.join("");
                pending_messages.clear();
            }
            "turn.failed" => {
                if let Some(message) = value.pointer("/error/message").and_then(Value::as_str) {
                    progress_events.push(Event::error(message.to_string()));
                }
            }
            _ => {
                if let Some(text) = codex_exec_text_fallback(&value) {
                    final_text = text;
                }
            }
        }
    }

    if final_text.trim().is_empty() && !pending_messages.is_empty() {
        final_text = pending_messages.join("");
    }
    if final_text.trim().is_empty() && !plain_lines.is_empty() {
        final_text = plain_lines.join("\n");
    }

    CodexExecOutput {
        final_text,
        final_session_id: thread_id.clone(),
        thread_id,
        progress_events,
    }
}

fn codex_exec_item_started(value: &Value) -> Option<Event> {
    let item = value.get("item")?;
    match item.get("type").and_then(Value::as_str)? {
        "command_execution" => Some(Event {
            event_type: EventType::ToolUse,
            content: String::new(),
            tool_name: Some("Bash".to_string()),
            tool_input: item
                .get("command")
                .and_then(Value::as_str)
                .map(str::to_string),
            tool_input_raw: Some(item.clone()),
            tool_result: None,
            session_id: None,
            request_id: None,
            done: false,
            error: None,
            metadata: None,
        }),
        "function_call" => Some(Event {
            event_type: EventType::ToolUse,
            content: String::new(),
            tool_name: item.get("name").and_then(Value::as_str).map(str::to_string),
            tool_input: item
                .get("arguments")
                .and_then(Value::as_str)
                .map(str::to_string),
            tool_input_raw: Some(item.clone()),
            tool_result: None,
            session_id: None,
            request_id: None,
            done: false,
            error: None,
            metadata: None,
        }),
        _ => None,
    }
}

fn codex_exec_item_completed(value: &Value) -> Option<Event> {
    let item = value.get("item")?;
    match item.get("type").and_then(Value::as_str)? {
        "reasoning" => extract_item_text(item, &["summary", "summary_text"]).map(|content| Event {
            event_type: EventType::Thinking,
            content,
            tool_name: None,
            tool_input: None,
            tool_input_raw: None,
            tool_result: None,
            session_id: None,
            request_id: None,
            done: false,
            error: None,
            metadata: None,
        }),
        "command_execution" => Some(Event {
            event_type: EventType::ToolResult,
            content: String::new(),
            tool_name: Some("Bash".to_string()),
            tool_input: item
                .get("command")
                .and_then(Value::as_str)
                .map(str::to_string),
            tool_input_raw: Some(item.clone()),
            tool_result: item
                .get("aggregated_output")
                .or_else(|| item.get("output"))
                .and_then(Value::as_str)
                .map(str::to_string),
            session_id: None,
            request_id: None,
            done: false,
            error: None,
            metadata: Some(item.clone()),
        }),
        "function_call" => Some(Event {
            event_type: EventType::ToolResult,
            content: String::new(),
            tool_name: item.get("name").and_then(Value::as_str).map(str::to_string),
            tool_input: item
                .get("arguments")
                .and_then(Value::as_str)
                .map(str::to_string),
            tool_input_raw: Some(item.clone()),
            tool_result: item
                .get("output")
                .and_then(Value::as_str)
                .map(str::to_string),
            session_id: None,
            request_id: None,
            done: false,
            error: None,
            metadata: Some(item.clone()),
        }),
        _ => None,
    }
}

fn codex_exec_completed_message(value: &Value) -> Option<String> {
    let item = value.get("item")?;
    let item_type = item.get("type").and_then(Value::as_str)?;
    if !matches!(item_type, "agent_message" | "message" | "assistant_message") {
        return None;
    }
    extract_item_text(item, &["content", "output_text", "text"])
}

fn extract_item_text(item: &Value, fields: &[&str]) -> Option<String> {
    for field in fields {
        let Some(value) = item.get(*field) else {
            continue;
        };
        if let Some(text) = value.as_str() {
            return Some(text.to_string());
        }
        if let Some(items) = value.as_array() {
            let mut out = String::new();
            for part in items {
                if let Some(text) = part.as_str() {
                    out.push_str(text);
                } else if let Some(text) = part.get("text").and_then(Value::as_str) {
                    out.push_str(text);
                } else if let Some(text) = part.get("content").and_then(Value::as_str) {
                    out.push_str(text);
                }
            }
            if !out.is_empty() {
                return Some(out);
            }
        }
    }
    None
}

fn codex_exec_text_fallback(value: &Value) -> Option<String> {
    value
        .get("content")
        .or_else(|| value.get("text"))
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            value
                .pointer("/message/content")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}
