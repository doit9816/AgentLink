use crate::core::{
    Agent, AgentCapabilities, AgentSession, AgentSessionInfo, Event, FileAttachment,
    ImageAttachment, PermissionResult,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex};

#[derive(Debug, Clone)]
pub struct CodexAgent {
    pub work_dir: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub mode: String,
    pub backend: String,
    pub codex_bin: String,
    pub cli_extra_args: Vec<String>,
    pub codex_home: Option<String>,
    pub base_url: Option<String>,
    pub model_provider: Option<String>,
    pub env: BTreeMap<String, String>,
    pub timeout_secs: u64,
}

impl CodexAgent {
    pub fn new_from_options(opts: toml::value::Table) -> Self {
        let cli_path = table_string(&opts, "cli_path");
        let cli_path_parts = cli_path.as_deref().map(split_cli_words).unwrap_or_default();
        let cli_path_bin = cli_path_parts.first().cloned();
        let cli_path_args = if cli_path_parts.len() > 1 {
            cli_path_parts[1..].to_vec()
        } else {
            Vec::new()
        };
        Self {
            work_dir: table_string(&opts, "work_dir").unwrap_or_else(|| ".".to_string()),
            model: table_string(&opts, "model"),
            reasoning_effort: table_string(&opts, "reasoning_effort")
                .or_else(|| table_string(&opts, "model_reasoning_effort")),
            mode: table_string(&opts, "mode").unwrap_or_else(|| "suggest".to_string()),
            backend: table_string(&opts, "backend")
                .map(|value| normalize_codex_backend(&value))
                .unwrap_or_else(|| "exec".to_string()),
            codex_bin: table_string(&opts, "codex_bin")
                .or_else(|| table_string(&opts, "command"))
                .or_else(|| table_string(&opts, "cmd"))
                .or(cli_path_bin)
                .unwrap_or_else(|| "codex".to_string()),
            cli_extra_args: table_string_vec(&opts, "args").unwrap_or(cli_path_args),
            codex_home: table_string(&opts, "codex_home"),
            base_url: table_string(&opts, "base_url")
                .or_else(|| table_string(&opts, "openai_base_url")),
            model_provider: table_string(&opts, "model_provider"),
            env: table_env(&opts),
            timeout_secs: table_u64(&opts, "timeout_secs")
                .or_else(|| table_u64(&opts, "timeout_seconds"))
                .or_else(|| table_u64(&opts, "timeout_mins").map(|mins| mins * 60))
                .unwrap_or(1800),
        }
    }
}

#[async_trait]
impl Agent for CodexAgent {
    fn name(&self) -> &str {
        "codex"
    }

    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            images: true,
            files: true,
            approvals: true,
            model_switching: true,
            session_list: true,
            context_usage: true,
            streaming_output: true,
        }
    }

    async fn start_session(&self, session_id: Option<String>) -> Result<Arc<dyn AgentSession>> {
        match self.backend.as_str() {
            "app-server" => Ok(Arc::new(
                CodexAppServerSession::start(self.clone(), session_id).await?,
            )),
            "exec" => Ok(Arc::new(CodexExecSession::new(self.clone(), session_id))),
            other => Err(anyhow!(
                "unsupported codex backend `{other}`, expected `exec` or `app-server`"
            )),
        }
    }

    async fn list_sessions(&self) -> Result<Vec<AgentSessionInfo>> {
        Ok(Vec::new())
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }
}

pub struct CodexExecSession {
    agent: CodexAgent,
    id: String,
    thread_id: Mutex<String>,
    alive: AtomicBool,
    tx: mpsc::UnboundedSender<Event>,
    rx: AsyncMutex<mpsc::UnboundedReceiver<Event>>,
}

impl CodexExecSession {
    fn new(agent: CodexAgent, session_id: Option<String>) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let (tx, rx) = mpsc::unbounded_channel();
        let id = session_id.unwrap_or_else(|| {
            format!(
                "codex-exec-session-{}",
                NEXT_ID.fetch_add(1, Ordering::SeqCst)
            )
        });
        let initial_thread_id = if id.starts_with("codex-exec-session-") {
            String::new()
        } else {
            id.clone()
        };
        Self {
            agent,
            id,
            thread_id: Mutex::new(initial_thread_id),
            alive: AtomicBool::new(true),
            tx,
            rx: AsyncMutex::new(rx),
        }
    }

    fn codex_thread_id(&self) -> String {
        self.thread_id.lock().unwrap().clone()
    }
}

#[async_trait]
impl AgentSession for CodexExecSession {
    async fn send(
        &self,
        prompt: String,
        images: Vec<ImageAttachment>,
        files: Vec<FileAttachment>,
    ) -> Result<()> {
        if !self.alive() {
            return Err(anyhow!("codex exec session is closed"));
        }
        let agent = self.agent.clone();
        let session_id = self.id.clone();
        let current_thread_id = self.codex_thread_id();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = run_codex_exec_turn(
                &agent,
                &session_id,
                &current_thread_id,
                prompt,
                images,
                files,
            )
            .await;
            match result {
                Ok(output) => {
                    for event in output.progress_events {
                        let _ = tx.send(event);
                    }
                    let final_session_id = output
                        .final_session_id
                        .unwrap_or_else(|| session_id.clone());
                    let _ = tx.send(Event::result(output.final_text, final_session_id));
                }
                Err(err) => {
                    let _ = tx.send(Event::error(err.to_string()));
                }
            }
        });
        Ok(())
    }

    async fn respond_permission(
        &self,
        _request_id: String,
        _result: PermissionResult,
    ) -> Result<()> {
        Err(anyhow!(
            "codex exec backend does not expose permission callbacks; use backend=`app-server` for interactive approvals"
        ))
    }

    async fn recv_event(&self) -> Option<Event> {
        let event = self.rx.lock().await.recv().await;
        if let Some(event) = &event {
            if let Some(session_id) = &event.session_id {
                if !session_id.is_empty() && !session_id.starts_with("codex-exec-session-") {
                    *self.thread_id.lock().unwrap() = session_id.clone();
                }
            }
        }
        event
    }

    fn current_session_id(&self) -> String {
        let thread_id = self.codex_thread_id();
        if thread_id.is_empty() {
            self.id.clone()
        } else {
            thread_id
        }
    }

    fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    async fn close(&self) -> Result<()> {
        self.alive.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Debug)]
struct CodexExecOutput {
    final_text: String,
    final_session_id: Option<String>,
    thread_id: Option<String>,
    progress_events: Vec<Event>,
}

async fn run_codex_exec_turn(
    agent: &CodexAgent,
    session_id: &str,
    thread_id: &str,
    prompt: String,
    images: Vec<ImageAttachment>,
    files: Vec<FileAttachment>,
) -> Result<CodexExecOutput> {
    let (prompt, image_paths) = codex_exec_prompt_and_images(agent, prompt, images, files).await?;
    let args = build_codex_exec_args(agent, thread_id, &image_paths);
    let mut command = Command::new(&agent.codex_bin);
    command
        .args(&args)
        .current_dir(&agent.work_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in codex_exec_env(agent) {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .map_err(|err| anyhow!("start Codex CLI `{}`: {err}", agent.codex_bin))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(prompt.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
    }
    let output = tokio::time::timeout(
        Duration::from_secs(agent.timeout_secs),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| anyhow!("Codex CLI turn timed out after {}s", agent.timeout_secs))??;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let mut parsed = parse_codex_exec_stdout(&stdout);
    if !output.status.success() {
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        return Err(anyhow!("Codex CLI exited with {}: {detail}", output.status));
    }
    if parsed.final_text.trim().is_empty() && !stdout.trim().is_empty() {
        parsed.final_text = stdout.trim().to_string();
    }
    if parsed.final_session_id.is_none() {
        parsed.final_session_id = parsed
            .thread_id
            .clone()
            .or_else(|| (!thread_id.is_empty()).then(|| thread_id.to_string()))
            .or_else(|| Some(session_id.to_string()));
    }
    Ok(parsed)
}

async fn codex_exec_prompt_and_images(
    agent: &CodexAgent,
    mut prompt: String,
    images: Vec<ImageAttachment>,
    files: Vec<FileAttachment>,
) -> Result<(String, Vec<String>)> {
    let mut image_paths = Vec::new();
    for (index, image) in images.into_iter().enumerate() {
        if let Some(path) =
            attachment_path_or_temp("image", index, image.file_name, image.data).await?
        {
            image_paths.push(path);
        }
    }
    let mut file_notes = Vec::new();
    for (index, file) in files.into_iter().enumerate() {
        if let Some(path) =
            attachment_path_or_temp("file", index, file.file_name, file.data).await?
        {
            file_notes.push(format!("- {path}"));
        }
    }
    if !file_notes.is_empty() {
        prompt.push_str("\n\n[local files]\n");
        prompt.push_str(&file_notes.join("\n"));
    }
    if prompt.trim().is_empty() && !image_paths.is_empty() {
        prompt = "Please analyze the attached image(s).".to_string();
    }
    if !PathBuf::from(&agent.work_dir).exists() {
        return Err(anyhow!("Codex work_dir does not exist: {}", agent.work_dir));
    }
    Ok((prompt, image_paths))
}

fn build_codex_exec_args(
    agent: &CodexAgent,
    thread_id: &str,
    image_paths: &[String],
) -> Vec<String> {
    let mut args = Vec::new();
    args.extend(agent.cli_extra_args.clone());
    if thread_id.trim().is_empty() {
        args.extend(["exec".to_string(), "--skip-git-repo-check".to_string()]);
    } else {
        args.extend([
            "exec".to_string(),
            "resume".to_string(),
            "--skip-git-repo-check".to_string(),
        ]);
    }
    match agent.mode.as_str() {
        "auto-edit" | "full-auto" => args.push("--full-auto".to_string()),
        "yolo" => args.push("--dangerously-bypass-approvals-and-sandbox".to_string()),
        _ => {}
    }
    if let Some(model) = &agent.model {
        args.extend(["--model".to_string(), model.clone()]);
    }
    if let Some(provider) = &agent.model_provider {
        args.extend([
            "-c".to_string(),
            format!("model_provider={}", shell_json(provider)),
        ]);
    }
    if let Some(base_url) = &agent.base_url {
        args.extend([
            "-c".to_string(),
            format!("openai_base_url={}", shell_json(base_url)),
        ]);
    }
    if let Some(effort) = &agent.reasoning_effort {
        args.extend([
            "-c".to_string(),
            format!("model_reasoning_effort={}", shell_json(effort)),
        ]);
    }
    if !thread_id.trim().is_empty() {
        args.push(thread_id.to_string());
    }
    for image_path in image_paths {
        args.extend(["--image".to_string(), image_path.clone()]);
    }
    if thread_id.trim().is_empty() {
        args.extend([
            "--json".to_string(),
            "--cd".to_string(),
            agent.work_dir.clone(),
        ]);
    } else {
        args.push("--json".to_string());
    }
    args.push("-".to_string());
    args
}

fn codex_exec_env(agent: &CodexAgent) -> BTreeMap<String, String> {
    let mut env = agent.env.clone();
    if let Some(codex_home) = &agent.codex_home {
        env.insert("CODEX_HOME".to_string(), codex_home.clone());
    }
    env
}

fn shell_json(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("{value:?}"))
}

fn parse_codex_exec_stdout(stdout: &str) -> CodexExecOutput {
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
            event_type: crate::core::EventType::ToolUse,
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
            event_type: crate::core::EventType::ToolUse,
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
            event_type: crate::core::EventType::Thinking,
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
            event_type: crate::core::EventType::ToolResult,
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
            event_type: crate::core::EventType::ToolResult,
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
    async fn start(agent: CodexAgent, resume_id: Option<String>) -> Result<Self> {
        let mut child = Command::new(&agent.codex_bin)
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .current_dir(&agent.work_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| anyhow!("start codex app-server: {err}"))?;

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
            .and_then(|t| t.get("id"))
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
            crate::core::PermissionBehavior::Allow => "accept",
            crate::core::PermissionBehavior::Deny => "decline",
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

#[derive(Debug, serde::Deserialize)]
struct RpcEnvelope {
    id: Option<i64>,
    method: Option<String>,
    params: Option<Value>,
    result: Option<Value>,
    error: Option<Value>,
}

fn spawn_read_loop(
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
            let _ = events_tx.send(Event::permission_request(
                request_id.clone(),
                tool_name,
                summary,
            ));
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

fn approval_summary(params: &Value) -> String {
    params
        .get("command")
        .and_then(Value::as_str)
        .or_else(|| params.get("reason").and_then(Value::as_str))
        .unwrap_or_else(|| params.as_str().unwrap_or(""))
        .to_string()
}

fn mode_settings(mode: &str) -> (&'static str, &'static str) {
    match mode {
        "auto-edit" | "full-auto" => ("never", "workspace-write"),
        "yolo" => ("never", "danger-full-access"),
        _ => ("on-request", "read-only"),
    }
}

fn table_string(opts: &toml::value::Table, key: &str) -> Option<String> {
    opts.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn table_string_vec(opts: &toml::value::Table, key: &str) -> Option<Vec<String>> {
    opts.get(key).and_then(|value| {
        value.as_array().map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
    })
}

fn table_u64(opts: &toml::value::Table, key: &str) -> Option<u64> {
    opts.get(key).and_then(|v| v.as_integer()).map(|v| v as u64)
}

fn table_env(opts: &toml::value::Table) -> BTreeMap<String, String> {
    opts.get("env")
        .and_then(|value| value.as_table())
        .map(|table| {
            table
                .iter()
                .filter_map(|(key, value)| value.as_str().map(|v| (key.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn normalize_codex_backend(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "app_server" | "appserver" | "stdio" => "app-server".to_string(),
        "cli" | "codex-cli" => "exec".to_string(),
        other => other.to_string(),
    }
}

fn split_cli_words(value: &str) -> Vec<String> {
    value.split_whitespace().map(str::to_string).collect()
}

async fn attachment_path_or_temp(
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
    let path: PathBuf = dir.join(format!("{}-{}-{}.bin", prefix, std::process::id(), index));
    tokio::fs::write(&path, data).await?;
    Ok(Some(path.to_string_lossy().to_string()))
}

#[cfg(test)]
mod tests {
    use super::{build_codex_exec_args, parse_codex_exec_stdout, CodexAgent, CodexExecOutput};

    fn codex_agent() -> CodexAgent {
        let mut opts = toml::value::Table::new();
        opts.insert(
            "work_dir".to_string(),
            toml::Value::String("D:/repo".to_string()),
        );
        opts.insert(
            "model".to_string(),
            toml::Value::String("gpt-5.2".to_string()),
        );
        opts.insert(
            "reasoning_effort".to_string(),
            toml::Value::String("medium".to_string()),
        );
        CodexAgent::new_from_options(opts)
    }

    #[test]
    fn codex_defaults_to_exec_backend() {
        let agent = CodexAgent::new_from_options(toml::value::Table::new());
        assert_eq!(agent.backend, "exec");
        assert_eq!(agent.codex_bin, "codex");
    }

    #[test]
    fn codex_cli_path_splits_binary_and_extra_args() {
        let mut opts = toml::value::Table::new();
        opts.insert(
            "cli_path".to_string(),
            toml::Value::String("codex --profile work".to_string()),
        );
        let agent = CodexAgent::new_from_options(opts);
        assert_eq!(agent.codex_bin, "codex");
        assert_eq!(agent.cli_extra_args, vec!["--profile", "work"]);
    }

    #[test]
    fn codex_exec_start_args_match_cli_json_flow() {
        let agent = codex_agent();
        let args = build_codex_exec_args(&agent, "", &["a.png".to_string()]);
        assert!(args.starts_with(&["exec".to_string(), "--skip-git-repo-check".to_string()]));
        assert!(args.contains(&"--json".to_string()));
        assert!(args.contains(&"--cd".to_string()));
        assert!(args.contains(&"a.png".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("-"));
    }

    #[test]
    fn codex_exec_resume_args_put_thread_before_json() {
        let agent = codex_agent();
        let args = build_codex_exec_args(&agent, "thread-1", &[]);
        let thread_pos = args.iter().position(|arg| arg == "thread-1").unwrap();
        let json_pos = args.iter().position(|arg| arg == "--json").unwrap();
        assert!(thread_pos < json_pos);
        assert!(!args.contains(&"--cd".to_string()));
    }

    #[test]
    fn codex_exec_stdout_extracts_thread_and_final_message() {
        let output: CodexExecOutput = parse_codex_exec_stdout(
            r#"{"type":"thread.started","thread_id":"abc"}
{"type":"item.completed","item":{"type":"agent_message","content":[{"text":"你好"},{"text":" Codex"}]}}
{"type":"turn.completed"}"#,
        );
        assert_eq!(output.thread_id.as_deref(), Some("abc"));
        assert_eq!(output.final_text, "你好 Codex");
        assert_eq!(output.final_session_id.as_deref(), Some("abc"));
    }
}
