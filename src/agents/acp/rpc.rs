use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::oneshot;

pub(super) type RpcNotifyHandler = Arc<dyn Fn(String, Value) + Send + Sync>;
pub(super) type RpcRequestHandler = Arc<dyn Fn(String, Value, Value) + Send + Sync>;

#[derive(Debug, Clone)]
pub(super) struct RpcError {
    pub code: i64,
    pub message: String,
}

#[derive(Debug)]
struct RpcOutcome {
    result: Option<Value>,
    error: Option<RpcError>,
}

#[derive(Debug, serde::Deserialize)]
struct RpcLine {
    id: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
    result: Option<Value>,
    error: Option<RpcErrorJson>,
}

#[derive(Debug, serde::Deserialize)]
struct RpcErrorJson {
    code: i64,
    message: String,
}

pub(super) struct AcpRpc {
    stdin: tokio::sync::Mutex<ChildStdin>,
    next_id: AtomicI64,
    pending: Mutex<HashMap<String, oneshot::Sender<RpcOutcome>>>,
    on_notify: RpcNotifyHandler,
    on_request: RpcRequestHandler,
}

impl AcpRpc {
    pub(super) fn new(
        stdin: ChildStdin,
        on_notify: RpcNotifyHandler,
        on_request: RpcRequestHandler,
    ) -> Arc<Self> {
        Arc::new(Self {
            stdin: tokio::sync::Mutex::new(stdin),
            next_id: AtomicI64::new(1),
            pending: Mutex::new(HashMap::new()),
            on_notify,
            on_request,
        })
    }

    pub(super) fn spawn_read_loop(self: &Arc<Self>, stdout: tokio::process::ChildStdout) {
        let rpc = Arc::clone(self);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let Ok(env) = serde_json::from_str::<RpcLine>(trimmed) else {
                    tracing::debug!(line = %trimmed, "acp: skip non-json line");
                    continue;
                };
                if let Some(method) = env.method {
                    let params = env.params.unwrap_or(Value::Null);
                    if is_notification_id(&env.id) {
                        (rpc.on_notify)(method, params);
                    } else if let Some(id) = env.id {
                        (rpc.on_request)(method, id, params);
                    }
                    continue;
                }
                if let Some(id) = env.id {
                    rpc.complete_pending(&id, env.result, env.error);
                }
            }
            rpc.cancel_all("acp: read closed");
        });
    }

    pub(super) async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let key = id.to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(key.clone(), tx);
        if let Err(err) = self
            .write_value(json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }))
            .await
        {
            self.pending.lock().unwrap().remove(&key);
            return Err(anyhow!("acp: write {method}: {err}"));
        }
        let outcome = rx
            .await
            .map_err(|_| anyhow!("acp: {method} response channel closed"))?;
        if let Some(err) = outcome.error {
            return Err(anyhow!("json-rpc {}: {}", err.code, err.message));
        }
        Ok(outcome.result.unwrap_or(Value::Null))
    }

    pub(super) async fn respond_success(&self, id: Value, result: Value) -> Result<()> {
        self.write_value(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }))
        .await
    }

    pub(super) async fn respond_error(&self, id: Value, code: i64, message: &str) -> Result<()> {
        self.write_value(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        }))
        .await
    }

    async fn write_value(&self, value: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(&bytes).await?;
        stdin.flush().await?;
        Ok(())
    }

    fn complete_pending(&self, id: &Value, result: Option<Value>, error: Option<RpcErrorJson>) {
        let key = json_id_key(id);
        let Some(tx) = self.pending.lock().unwrap().remove(&key) else {
            tracing::debug!(id = %key, "acp: unmatched rpc response");
            return;
        };
        let outcome = RpcOutcome {
            result,
            error: error.map(|e| RpcError {
                code: e.code,
                message: e.message,
            }),
        };
        let _ = tx.send(outcome);
    }

    fn cancel_all(&self, message: &str) {
        let mut pending = self.pending.lock().unwrap();
        for (_, tx) in pending.drain() {
            let _ = tx.send(RpcOutcome {
                result: None,
                error: Some(RpcError {
                    code: -32000,
                    message: message.to_string(),
                }),
            });
        }
    }
}

pub(super) fn json_id_key(id: &Value) -> String {
    match id {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn is_notification_id(id: &Option<Value>) -> bool {
    match id {
        None => true,
        Some(Value::Null) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::json_id_key;
    use serde_json::json;

    #[test]
    fn json_id_key_formats_numeric_and_string_ids() {
        assert_eq!(json_id_key(&json!(1)), "1");
        assert_eq!(json_id_key(&json!("req-abc")), "req-abc");
    }
}
