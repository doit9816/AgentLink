use super::exec_args::{build_codex_exec_args, codex_exec_env};
use super::exec_output::{parse_codex_exec_stdout, CodexExecOutput};
use super::exec_prompt::codex_exec_prompt_and_images;
use super::CodexAgent;
use crate::core::{AgentSession, Event, FileAttachment, ImageAttachment, PermissionResult};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex as AsyncMutex};

pub struct CodexExecSession {
    agent: CodexAgent,
    id: String,
    thread_id: Mutex<String>,
    alive: AtomicBool,
    tx: mpsc::UnboundedSender<Event>,
    rx: AsyncMutex<mpsc::UnboundedReceiver<Event>>,
}

impl CodexExecSession {
    pub(super) fn new(agent: CodexAgent, session_id: Option<String>) -> Self {
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
