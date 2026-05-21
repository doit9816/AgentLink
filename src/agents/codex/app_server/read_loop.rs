use super::super::shared::approval_summary;
use super::rpc::RpcEnvelope;
use crate::core::Event;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, oneshot};

pub(super) fn spawn_read_loop(
    stdout: tokio::process::ChildStdout,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<RpcEnvelope>>>>,
    events_tx: mpsc::UnboundedSender<Event>,
    approvals: Arc<Mutex<HashMap<String, i64>>>,
    final_text: Arc<Mutex<String>>,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(env) = serde_json::from_str::<RpcEnvelope>(&line) else {
                continue;
            };
            match (env.id, env.method.clone()) {
                (Some(id), None) => {
                    if let Some(tx) = pending.lock().unwrap().remove(&id) {
                        let _ = tx.send(env);
                    }
                }
                (Some(id), Some(method)) => {
                    handle_server_request(id, method, env.params, &events_tx, &approvals);
                }
                (None, Some(method)) => {
                    handle_notification(method, env.params, &events_tx, &final_text);
                }
                _ => {}
            }
        }
    });
}

fn handle_server_request(
    id: i64,
    method: String,
    params: Option<Value>,
    events_tx: &mpsc::UnboundedSender<Event>,
    approvals: &Arc<Mutex<HashMap<String, i64>>>,
) {
    match method.as_str() {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            let request_id = id.to_string();
            approvals.lock().unwrap().insert(request_id.clone(), id);
            let params = params.unwrap_or(Value::Null);
            let summary = approval_summary(&params);
            let tool_name = if method.contains("fileChange") {
                "Patch"
            } else {
                "Bash"
            };
            let _ = events_tx.send(Event::permission_request(request_id, tool_name, summary));
        }
        _ => {
            let _ = events_tx.send(Event::error(format!(
                "unsupported codex server request: {method}"
            )));
        }
    }
}

fn handle_notification(
    method: String,
    params: Option<Value>,
    events_tx: &mpsc::UnboundedSender<Event>,
    final_text: &Arc<Mutex<String>>,
) {
    let params = params.unwrap_or(Value::Null);
    match method.as_str() {
        "item/agentMessage/delta" => {
            if let Some(delta) = params.get("delta").and_then(Value::as_str) {
                final_text.lock().unwrap().push_str(delta);
            }
        }
        "item/completed" => {
            if let Some(text) = extract_completed_agent_message(&params) {
                *final_text.lock().unwrap() = text;
            }
        }
        "turn/completed" => {
            let text = std::mem::take(&mut *final_text.lock().unwrap());
            let _ = events_tx.send(Event::result(text, ""));
        }
        "error" => {
            let _ = events_tx.send(Event::error(params.to_string()));
        }
        _ => {}
    }
}

pub fn extract_completed_agent_message(value: &Value) -> Option<String> {
    let item = value.get("item")?;
    let item_type = item.get("type").and_then(Value::as_str)?;
    if !matches!(item_type, "agentMessage" | "assistantMessage" | "message") {
        return None;
    }
    if let Some(text) = item.get("text").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = item.get("content").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    let mut out = String::new();
    for part in item.get("content")?.as_array()? {
        if let Some(text) = part.get("text").and_then(Value::as_str) {
            out.push_str(text);
        }
    }
    Some(out)
}
