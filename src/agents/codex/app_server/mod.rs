mod read_loop;
mod rpc;

use super::shared::{attachment_path_or_temp, mode_settings};
use super::CodexAgent;
use crate::core::{
    AgentSession, Event, FileAttachment, ImageAttachment, PermissionBehavior, PermissionResult,
};
use crate::util::path_env::{apply_enriched_path_tokio, resolve_executable};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin};
use tokio::sync::{mpsc, oneshot};

pub use read_loop::extract_completed_agent_message;
use read_loop::spawn_read_loop;
use rpc::RpcEnvelope;

pub struct CodexAppServerSession {
    child: Mutex<Option<Child>>,
    stdin: tokio::sync::Mutex<ChildStdin>,
    alive: AtomicBool,
    next_id: AtomicI64,
    thread_id: Mutex<String>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<RpcEnvelope>>>>,
    events_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<Event>>,
    approvals: Arc<Mutex<HashMap<String, i64>>>,
    final_text: Arc<Mutex<String>>,
}

impl CodexAppServerSession {
    pub(super) async fn start(agent: CodexAgent, resume_id: Option<String>) -> Result<Self> {
        let codex_bin = resolve_executable(&agent.codex_bin)
            .map_err(|err| anyhow!("start codex app-server (`{}`): {err}", agent.codex_bin))?;
        let mut child_cmd = tokio::process::Command::new(&codex_bin);
        child_cmd
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .current_dir(&agent.work_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        apply_enriched_path_tokio(&mut child_cmd);
        let mut child = child_cmd
            .spawn()
            .map_err(|err| anyhow!("start codex app-server (`{}`): {err}", codex_bin.display()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("missing codex stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("missing codex stdout"))?;
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let approvals = Arc::new(Mutex::new(HashMap::new()));
        let final_text = Arc::new(Mutex::new(String::new()));

        spawn_read_loop(
            stdout,
            Arc::clone(&pending),
            events_tx.clone(),
            Arc::clone(&approvals),
            Arc::clone(&final_text),
        );

        let session = Self {
            child: Mutex::new(Some(child)),
            stdin: tokio::sync::Mutex::new(stdin),
            alive: AtomicBool::new(true),
            next_id: AtomicI64::new(1),
            thread_id: Mutex::new(String::new()),
            pending,
            events_rx: tokio::sync::Mutex::new(events_rx),
            approvals,
            final_text,
        };
        session.initialize().await?;
        session.ensure_thread(&agent, resume_id).await?;
        Ok(session)
    }

    async fn initialize(&self) -> Result<()> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "cmscloud-agentlink",
                    "title": "CMSCloud AgentLink",
                    "version": "0.1.0"
                },
                "capabilities": {
                    "experimentalApi": true,
                    "optOutNotificationMethods": [
                        "command/exec/outputDelta",
                        "item/plan/delta",
                        "item/fileChange/outputDelta",
                        "item/reasoning/summaryTextDelta",
                        "item/reasoning/textDelta"
                    ]
                }
            }),
        )
        .await?;
        self.notify("initialized", Value::Null).await
    }

    async fn ensure_thread(&self, agent: &CodexAgent, resume_id: Option<String>) -> Result<()> {
        let (approval, sandbox) = mode_settings(&agent.mode);
        let mut params = json!({
            "cwd": agent.work_dir,
            "approvalPolicy": approval,
            "sandbox": sandbox,
            "experimentalRawEvents": false
        });
        if let Some(model) = &agent.model {
            params["model"] = json!(model);
        }
        let method = if let Some(thread_id) = resume_id {
            params["threadId"] = json!(thread_id);
            "thread/resume"
        } else {
            "thread/start"
        };
        let result = self.request(method, params).await?;
        let thread_id = result
            .get("thread")
            .and_then(|thread| thread.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("{method} returned empty thread id"))?;
        *self.thread_id.lock().unwrap() = thread_id.to_string();
        Ok(())
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.write_json(json!({ "id": id, "method": method, "params": params }))
            .await?;
        let env = rx
            .await
            .map_err(|_| anyhow!("{method} response channel closed"))?;
        if let Some(err) = env.error {
            return Err(anyhow!(
                "{} failed: {}",
                method,
                err.get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
            ));
        }
        Ok(env.result.unwrap_or(Value::Null))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.write_json(json!({ "method": method, "params": params }))
            .await
    }

    async fn write_json(&self, value: Value) -> Result<()> {
        let mut stdin = self.stdin.lock().await;
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        stdin.write_all(&bytes).await?;
        Ok(())
    }
}

#[async_trait]
impl AgentSession for CodexAppServerSession {
    async fn send(
        &self,
        prompt: String,
        images: Vec<ImageAttachment>,
        files: Vec<FileAttachment>,
    ) -> Result<()> {
        self.final_text.lock().unwrap().clear();
        let mut input = vec![json!({
            "type": "text",
            "text": prompt,
            "text_elements": []
        })];
        let mut attachment_notes = Vec::new();
        for (index, image) in images.into_iter().enumerate() {
            if let Some(path) =
                attachment_path_or_temp("image", index, image.file_name, image.data).await?
            {
                input.push(json!({ "type": "localImage", "path": path }));
            }
        }
        for (index, file) in files.into_iter().enumerate() {
            if let Some(path) =
                attachment_path_or_temp("file", index, file.file_name, file.data).await?
            {
                attachment_notes.push(format!("file attachment: {path}"));
            }
        }
        if !attachment_notes.is_empty() {
            input.push(json!({
                "type": "text",
                "text": format!("\n\n[local files]\n{}", attachment_notes.join("\n")),
                "text_elements": []
            }));
        }
        self.request(
            "turn/start",
            json!({
                "threadId": self.current_session_id(),
                "input": input,
                "approvalPolicy": "on-request"
            }),
        )
        .await?;
        Ok(())
    }

    async fn respond_permission(&self, request_id: String, result: PermissionResult) -> Result<()> {
        let Some(raw_id) = self.approvals.lock().unwrap().remove(&request_id) else {
            return Err(anyhow!("codex approval request `{request_id}` not found"));
        };
        let decision = match result.behavior {
            PermissionBehavior::Allow => "accept",
            PermissionBehavior::Deny => "decline",
        };
        self.write_json(json!({
            "id": raw_id,
            "result": { "decision": decision }
        }))
        .await?;
        Ok(())
    }

    async fn recv_event(&self) -> Option<Event> {
        self.events_rx.lock().await.recv().await
    }

    fn current_session_id(&self) -> String {
        self.thread_id.lock().unwrap().clone()
    }

    fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    async fn close(&self) -> Result<()> {
        self.alive.store(false, Ordering::SeqCst);
        let child = { self.child.lock().unwrap().take() };
        if let Some(mut child) = child {
            let _ = child.kill().await;
        }
        Ok(())
    }
}
