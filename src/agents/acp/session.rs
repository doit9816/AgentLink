use super::mapping::{
    build_permission_result, map_session_update, pick_permission_option_id,
    summarize_acp_tool_input, PermissionOption,
};
use super::options::AcpAgent;
use super::rpc::{json_id_key, AcpRpc};
use super::state::{AcpModesBlock, AcpModesState};
use crate::core::{
    AgentSession, Event, FileAttachment, ImageAttachment, PermissionBehavior, PermissionResult,
};
use crate::util::path_env::{apply_enriched_path_tokio, resolve_executable};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::process::Child;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

const TOOL_INPUT_CACHE_MAX: usize = 1000;

#[derive(Clone)]
struct PermState {
    rpc_id: Value,
    options: Vec<PermissionOption>,
}

struct SessionShared {
    acp_session_id: Mutex<String>,
    perm_by_id: Mutex<HashMap<String, PermState>>,
    tool_input_by_id: Mutex<HashMap<String, String>>,
    modes: Mutex<AcpModesState>,
    events_tx: mpsc::UnboundedSender<Event>,
}

pub struct AcpSession {
    agent: AcpAgent,
    child: Mutex<Option<Child>>,
    rpc: Arc<AcpRpc>,
    alive: AtomicBool,
    shared: Arc<SessionShared>,
    events_rx: AsyncMutex<mpsc::UnboundedReceiver<Event>>,
    send_lock: AsyncMutex<()>,
}

impl AcpSession {
    pub async fn start(agent: AcpAgent, resume_session_id: Option<String>) -> Result<Arc<Self>> {
        let work_dir = std::path::Path::new(&agent.work_dir);
        let abs_work_dir = work_dir
            .canonicalize()
            .unwrap_or_else(|_| work_dir.to_path_buf());
        let command = resolve_executable(&agent.command)
            .map_err(|err| anyhow!("acp: resolve command `{}`: {err}", agent.command))?;

        let mut child_cmd = tokio::process::Command::new(&command);
        child_cmd
            .args(&agent.args)
            .current_dir(&abs_work_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if command.components().count() == 1 {
            apply_enriched_path_tokio(&mut child_cmd);
        }
        for (key, value) in &agent.env {
            child_cmd.env(key, value);
        }

        let mut child = child_cmd
            .spawn()
            .map_err(|err| anyhow!("acp: start `{}`: {err}", command.display()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("acp: missing stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("acp: missing stdout"))?;

        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let shared = Arc::new(SessionShared {
            acp_session_id: Mutex::new(
                resume_session_id
                    .clone()
                    .filter(|id| !id.trim().is_empty())
                    .unwrap_or_default(),
            ),
            perm_by_id: Mutex::new(HashMap::new()),
            tool_input_by_id: Mutex::new(HashMap::new()),
            modes: Mutex::new(AcpModesState::default()),
            events_tx: events_tx.clone(),
        });

        let on_notify = {
            let shared = Arc::clone(&shared);
            Arc::new(move |method: String, params: Value| {
                if method != "session/update" {
                    tracing::debug!(method = %method, "acp: notification");
                    return;
                }
                cache_tool_call_input(&shared.tool_input_by_id, &params);
                let sid = shared.acp_session_id.lock().unwrap().clone();
                for event in map_session_update(&sid, &params) {
                    let _ = shared.events_tx.send(event);
                }
            })
        };

        let rpc_slot: Arc<Mutex<Option<Arc<AcpRpc>>>> = Arc::new(Mutex::new(None));
        let on_request = {
            let rpc_slot = Arc::clone(&rpc_slot);
            let shared = Arc::clone(&shared);
            Arc::new(move |method: String, id: Value, params: Value| {
                let Some(rpc) = rpc_slot.lock().unwrap().clone() else {
                    return;
                };
                match method.as_str() {
                    "session/request_permission" => {
                        handle_permission_request(rpc, id, params, Arc::clone(&shared));
                    }
                    "cursor/ask_question"
                    | "cursor/create_plan"
                    | "cursor/update_todos"
                    | "cursor/task"
                    | "cursor/generate_image" => {
                        tokio::spawn(async move {
                            let _ = rpc.respond_success(id, json!({})).await;
                        });
                    }
                    other if other.starts_with("cursor/") => {
                        tokio::spawn(async move {
                            let _ = rpc.respond_success(id, json!({})).await;
                        });
                    }
                    other => {
                        tracing::info!(method = %other, "acp: unhandled server request");
                        tokio::spawn(async move {
                            let _ = rpc
                                .respond_error(id, -32601, "method not implemented")
                                .await;
                        });
                    }
                }
            })
        };

        let rpc = AcpRpc::new(stdin, on_notify, on_request);
        *rpc_slot.lock().unwrap() = Some(Arc::clone(&rpc));
        rpc.spawn_read_loop(stdout);

        let session = Arc::new(AcpSession {
            agent: agent.clone(),
            child: Mutex::new(Some(child)),
            rpc,
            alive: AtomicBool::new(true),
            shared,
            events_rx: AsyncMutex::new(events_rx),
            send_lock: AsyncMutex::new(()),
        });

        session.handshake(resume_session_id).await?;
        if let Some(mode) = &agent.mode {
            session.set_live_mode(mode).await;
        }

        Ok(session)
    }

    async fn handshake(&self, resume_session_id: Option<String>) -> Result<()> {
        let init_params = json!({
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": { "readTextFile": false, "writeTextFile": false },
                "terminal": false
            },
            "clientInfo": { "name": "agentlink", "version": env!("CARGO_PKG_VERSION") }
        });
        let init_result = self.rpc.call("initialize", init_params).await?;
        let init: InitializeResult = serde_json::from_value(init_result)
            .map_err(|err| anyhow!("acp: parse initialize result: {err}"))?;
        let caps = init.agent_capabilities.as_ref();
        let load_session = caps.and_then(|c| c.load_session).unwrap_or(false);
        let list_supported = caps
            .and_then(|c| c.session_capabilities.as_ref())
            .and_then(|s| s.list.as_ref())
            .is_some();
        if !list_supported {
            self.agent
                .state
                .list_unsupported
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }

        if let Some(auth) = &self.agent.auth_method {
            self.rpc
                .call("authenticate", json!({ "methodId": auth }))
                .await
                .map_err(|err| anyhow!("acp: authenticate ({auth}): {err}"))?;
        }

        if let Some(resume) = resume_session_id.filter(|id| !id.trim().is_empty()) {
            if load_session {
                match self
                    .rpc
                    .call(
                        "session/load",
                        json!({
                            "sessionId": resume,
                            "cwd": self.work_dir_display(),
                            "mcpServers": []
                        }),
                    )
                    .await
                {
                    Ok(load_result) => {
                        if let Ok(parsed) = serde_json::from_value::<SessionNewResult>(load_result)
                        {
                            if !parsed.session_id.is_empty() {
                                *self.shared.acp_session_id.lock().unwrap() = parsed.session_id;
                                self.absorb_modes(parsed.modes.as_ref());
                                return Ok(());
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "acp: session/load failed, starting new session");
                    }
                }
            }
        }

        let new_result = self
            .rpc
            .call(
                "session/new",
                json!({
                    "cwd": self.work_dir_display(),
                    "mcpServers": []
                }),
            )
            .await?;
        let parsed: SessionNewResult = serde_json::from_value(new_result)
            .map_err(|err| anyhow!("acp: parse session/new: {err}"))?;
        if parsed.session_id.is_empty() {
            return Err(anyhow!("acp: session/new returned empty sessionId"));
        }
        *self.shared.acp_session_id.lock().unwrap() = parsed.session_id;
        self.absorb_modes(parsed.modes.as_ref());
        Ok(())
    }

    fn absorb_modes(&self, block: Option<&AcpModesBlock>) {
        let Some(block) = block else {
            return;
        };
        if block.available_modes.is_empty() {
            return;
        }
        {
            let mut guard = self.shared.modes.lock().unwrap();
            guard.available = block.available_modes.clone();
            if !block.current_mode_id.is_empty() {
                guard.current_mode_id = block.current_mode_id.clone();
            }
        }
        self.agent.state.report_modes(block);
    }

    fn match_available_mode(&self, mode: &str) -> String {
        let available = self.shared.modes.lock().unwrap().available.clone();
        if available.is_empty() {
            return mode.trim().to_string();
        }
        let matched = self.agent.state.match_mode_id(mode);
        if !matched.is_empty() {
            return matched;
        }
        String::new()
    }

    async fn set_live_mode(&self, mode: &str) -> bool {
        let sid = self.current_session_id();
        if sid.is_empty() {
            return false;
        }
        let mode_id = self.match_available_mode(mode);
        if mode_id.is_empty() {
            tracing::debug!(mode = %mode, session_id = %sid, "acp: unknown mode");
            return false;
        }
        match self
            .rpc
            .call(
                "session/set_mode",
                json!({ "sessionId": sid, "modeId": mode_id }),
            )
            .await
        {
            Ok(_) => {
                self.shared.modes.lock().unwrap().current_mode_id = mode_id.clone();
                self.agent.state.modes.lock().unwrap().current_mode_id = mode_id.clone();
                tracing::info!(mode = %mode_id, session_id = %sid, "acp: live mode applied");
                true
            }
            Err(err) => {
                tracing::warn!(
                    mode = %mode_id,
                    session_id = %sid,
                    error = %err,
                    "acp: session/set_mode failed"
                );
                false
            }
        }
    }

    fn work_dir_display(&self) -> String {
        std::path::Path::new(&self.agent.work_dir)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(&self.agent.work_dir))
            .to_string_lossy()
            .into_owned()
    }
}

#[derive(Debug, Deserialize)]
struct InitializeResult {
    #[serde(rename = "agentCapabilities")]
    agent_capabilities: Option<AgentCapabilities>,
}

#[derive(Debug, Deserialize)]
struct AgentCapabilities {
    #[serde(rename = "loadSession")]
    load_session: Option<bool>,
    #[serde(rename = "sessionCapabilities")]
    session_capabilities: Option<SessionCapabilities>,
}

#[derive(Debug, Deserialize)]
struct SessionCapabilities {
    list: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct SessionNewResult {
    #[serde(rename = "sessionId")]
    session_id: String,
    modes: Option<AcpModesBlock>,
}

fn cache_tool_call_input(cache: &Mutex<HashMap<String, String>>, params: &Value) {
    let update = match params.get("update") {
        Some(v) if !v.is_null() => v,
        _ => return,
    };
    let kind = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "tool_call" => {
            let tool_call_id = update
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let raw = update.get("rawInput").cloned().unwrap_or(Value::Null);
            if tool_call_id.is_empty() || raw.is_null() {
                return;
            }
            let input_kind = update
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let summary = summarize_acp_tool_input(input_kind, &raw);
            let mut guard = cache.lock().unwrap();
            evict_tool_input_cache_if_needed(&mut guard);
            guard.insert(tool_call_id.to_string(), summary);
        }
        "tool_call_update" => {
            let tool_call_id = update
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let raw = update.get("rawInput").cloned().unwrap_or(Value::Null);
            if tool_call_id.is_empty() || raw.is_null() {
                return;
            }
            let summary = summarize_acp_tool_input("", &raw);
            if summary.is_empty() {
                return;
            }
            let mut guard = cache.lock().unwrap();
            evict_tool_input_cache_if_needed(&mut guard);
            guard.insert(tool_call_id.to_string(), summary);
        }
        _ => {}
    }
}

fn evict_tool_input_cache_if_needed(cache: &mut HashMap<String, String>) {
    if cache.len() < TOOL_INPUT_CACHE_MAX {
        return;
    }
    let target = TOOL_INPUT_CACHE_MAX / 2;
    while cache.len() > target {
        let Some(key) = cache.keys().next().cloned() else {
            break;
        };
        cache.remove(&key);
    }
}

fn handle_permission_request(
    rpc: Arc<AcpRpc>,
    id: Value,
    params: Value,
    shared: Arc<SessionShared>,
) {
    let parsed: PermissionRequestParams = match serde_json::from_value(params.clone()) {
        Ok(v) => v,
        Err(_) => {
            let rpc = Arc::clone(&rpc);
            tokio::spawn(async move {
                let _ = rpc.respond_error(id, -32602, "invalid params").await;
            });
            return;
        }
    };
    let req_key = json_id_key(&id);
    shared.perm_by_id.lock().unwrap().insert(
        req_key.clone(),
        PermState {
            rpc_id: id,
            options: parsed.options.clone(),
        },
    );

    let tool_call_id = parsed.tool_call.tool_call_id.clone();
    let tool_name = parsed
        .tool_call
        .title
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| parsed.tool_call.kind.clone().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "permission".to_string());
    let session_id = shared.acp_session_id.lock().unwrap().clone();
    tokio::spawn(async move {
        for _ in 0..10 {
            if shared
                .tool_input_by_id
                .lock()
                .unwrap()
                .get(&tool_call_id)
                .is_some_and(|s| !s.is_empty())
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let tool_input = shared
            .tool_input_by_id
            .lock()
            .unwrap()
            .get(&tool_call_id)
            .cloned()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                summarize_acp_tool_input(
                    parsed.tool_call.kind.as_deref().unwrap_or_default(),
                    &parsed.tool_call.raw_input.clone().unwrap_or(Value::Null),
                )
            });
        let tool_input = if tool_input.is_empty() {
            parsed
                .tool_call
                .title
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or(tool_call_id)
        } else {
            tool_input
        };
        let _ = shared.events_tx.send(Event {
            event_type: crate::core::EventType::PermissionRequest,
            content: String::new(),
            tool_name: Some(tool_name),
            tool_input: Some(tool_input),
            tool_input_raw: Some(params),
            tool_result: None,
            session_id: Some(session_id),
            request_id: Some(req_key),
            done: false,
            error: None,
            metadata: None,
        });
    });
}

#[derive(Debug, Deserialize)]
struct PermissionRequestParams {
    #[serde(rename = "toolCall")]
    tool_call: PermissionToolCall,
    #[serde(default)]
    options: Vec<PermissionOption>,
}

#[derive(Debug, Deserialize)]
struct PermissionToolCall {
    #[serde(rename = "toolCallId")]
    tool_call_id: String,
    title: Option<String>,
    kind: Option<String>,
    #[serde(rename = "rawInput")]
    raw_input: Option<Value>,
}

#[async_trait]
impl AgentSession for AcpSession {
    async fn send(
        &self,
        prompt: String,
        images: Vec<ImageAttachment>,
        files: Vec<FileAttachment>,
    ) -> Result<()> {
        if !self.alive() {
            return Err(anyhow!("acp: session closed"));
        }
        let _guard = self.send_lock.lock().await;
        let prompt = self.append_attachments(prompt, images, files).await?;
        let sid = self.current_session_id();
        if sid.is_empty() {
            return Err(anyhow!("acp: no agent session id"));
        }

        let params = json!({
            "sessionId": sid,
            "prompt": [{ "type": "text", "text": prompt }]
        });
        self.rpc.call("session/prompt", params).await?;
        let _ = self
            .shared
            .events_tx
            .send(Event::result(String::new(), sid));
        Ok(())
    }

    async fn respond_permission(&self, request_id: String, result: PermissionResult) -> Result<()> {
        if !self.alive() {
            return Err(anyhow!("acp: session closed"));
        }
        let st = {
            let mut guard = self.shared.perm_by_id.lock().unwrap();
            guard.remove(&request_id)
        };
        let Some(st) = st else {
            return Err(anyhow!("acp: unknown permission request `{request_id}`"));
        };
        let allow = matches!(result.behavior, PermissionBehavior::Allow);
        let opt_id = pick_permission_option_id(allow, &st.options);
        if allow && opt_id.is_empty() {
            return self
                .rpc
                .respond_error(st.rpc_id, -32603, "no permission options from agent")
                .await;
        }
        let body = build_permission_result(allow, &opt_id);
        self.rpc.respond_success(st.rpc_id, body).await
    }

    async fn recv_event(&self) -> Option<Event> {
        self.events_rx.lock().await.recv().await
    }

    fn current_session_id(&self) -> String {
        self.shared.acp_session_id.lock().unwrap().clone()
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

impl AcpSession {
    async fn append_attachments(
        &self,
        mut prompt: String,
        images: Vec<ImageAttachment>,
        files: Vec<FileAttachment>,
    ) -> Result<String> {
        let mut file_paths = Vec::new();
        for (index, file) in files.into_iter().enumerate() {
            if let Some(path) =
                save_attachment_path("file", index, file.file_name, file.data).await?
            {
                file_paths.push(path);
            }
        }
        if !file_paths.is_empty() {
            if prompt.is_empty() {
                prompt = "User attached file(s).".to_string();
            }
            prompt.push_str("\n\n[attached files]\n");
            prompt.push_str(&file_paths.join("\n"));
        }

        let attach_dir = PathBuf::from(&self.agent.work_dir)
            .join(".agentlink")
            .join("attachments");
        tokio::fs::create_dir_all(&attach_dir).await.ok();
        let mut image_paths = Vec::new();
        for (index, image) in images.into_iter().enumerate() {
            if image.data.is_empty() {
                continue;
            }
            let ext = match image.mime_type.as_str() {
                "image/jpeg" => ".jpg",
                "image/gif" => ".gif",
                "image/webp" => ".webp",
                _ => ".png",
            };
            let path = attach_dir.join(format!("img_{}_{}{}", std::process::id(), index, ext));
            if tokio::fs::write(&path, &image.data).await.is_ok() {
                image_paths.push(path.to_string_lossy().into_owned());
            }
        }
        if !image_paths.is_empty() {
            if prompt.is_empty() {
                prompt = "User sent image(s).".to_string();
            }
            prompt.push_str("\n\n(Image files saved locally: ");
            prompt.push_str(&image_paths.join(", "));
            prompt.push(')');
        }
        Ok(prompt)
    }
}

async fn save_attachment_path(
    prefix: &str,
    index: usize,
    file_name: Option<String>,
    data: Vec<u8>,
) -> Result<Option<String>> {
    if let Some(path) = file_name {
        if !path.trim().is_empty() {
            return Ok(Some(path));
        }
    }
    if data.is_empty() {
        return Ok(None);
    }
    let dir = std::env::temp_dir().join("agentlink-attachments");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(format!("{}-{}-{}.bin", prefix, std::process::id(), index));
    tokio::fs::write(&path, data).await?;
    Ok(Some(path.to_string_lossy().into_owned()))
}
