//! Minimal ACP JSON-RPC stdio peer for integration tests.
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let session_id = "mock-acp-session-1";
    let mut pending_prompt: Option<Value> = None;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(env) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        // Client response to our server request (permission).
        if env.get("result").is_some() && env.get("method").is_none() {
            if let Some(prompt_id) = pending_prompt.take() {
                finish_prompt(&mut stdout, prompt_id, session_id, "mock-acp: approved");
            }
            continue;
        }

        let method = env
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let id = env.get("id").cloned().unwrap_or(Value::Null);
        let params = env.get("params").cloned().unwrap_or(Value::Null);

        if method.is_empty() {
            continue;
        }

        match method {
            "initialize" => {
                write_line(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": 1,
                            "agentCapabilities": {
                                "loadSession": false,
                                "sessionCapabilities": { "list": {} }
                            }
                        }
                    }),
                );
            }
            "authenticate" => {
                write_line(
                    &mut stdout,
                    json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
                );
            }
            "session/new" => {
                write_line(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "sessionId": session_id,
                            "modes": {
                                "currentModeId": "default",
                                "availableModes": [
                                    { "id": "default", "name": "Default" },
                                    { "id": "yolo", "name": "YOLO" }
                                ]
                            }
                        }
                    }),
                );
            }
            "session/list" => {
                write_line(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "sessions": [
                                {
                                    "sessionId": session_id,
                                    "title": "mock session",
                                    "cwd": params.get("cwd").and_then(Value::as_str).unwrap_or(".")
                                }
                            ]
                        }
                    }),
                );
            }
            "session/prompt" => {
                let prompt = prompt_text(&params);
                if prompt.contains("approval") {
                    pending_prompt = Some(id);
                    write_line(
                        &mut stdout,
                        json!({
                            "jsonrpc": "2.0",
                            "id": 100,
                            "method": "session/request_permission",
                            "params": {
                                "sessionId": session_id,
                                "toolCall": {
                                    "toolCallId": "tc-mock-1",
                                    "title": "bash",
                                    "kind": "bash",
                                    "rawInput": { "command": "echo test" }
                                },
                                "options": [
                                    { "optionId": "allow-once", "name": "Allow", "kind": "allow" },
                                    { "optionId": "deny", "name": "Deny", "kind": "reject" }
                                ]
                            }
                        }),
                    );
                } else {
                    let reply = format!("mock-acp: {prompt}");
                    finish_prompt(&mut stdout, id, session_id, &reply);
                }
            }
            _ => {
                write_line(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": "not implemented" }
                    }),
                );
            }
        }
    }
}

fn prompt_text(params: &Value) -> String {
    params
        .get("prompt")
        .and_then(Value::as_array)
        .and_then(|blocks| blocks.first())
        .and_then(|block| block.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn emit_agent_chunk(stdout: &mut impl Write, session_id: &str, text: &str) {
    write_line(
        stdout,
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": text }
                }
            }
        }),
    );
}

fn finish_prompt(stdout: &mut impl Write, id: Value, session_id: &str, text: &str) {
    emit_agent_chunk(stdout, session_id, text);
    write_line(stdout, json!({ "jsonrpc": "2.0", "id": id, "result": {} }));
}

fn write_line(stdout: &mut impl Write, value: Value) {
    let mut bytes = serde_json::to_vec(&value).expect("json");
    bytes.push(b'\n');
    stdout.write_all(&bytes).expect("write");
    stdout.flush().expect("flush");
}
