use crate::core::{
    Agent, AgentCapabilities, AgentSession, AgentSessionInfo, Event, FileAttachment,
    ImageAttachment, PermissionResult,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};

#[derive(Debug, Clone)]
pub struct CliAgent {
    name: String,
    command: String,
    args: Vec<String>,
    work_dir: String,
    model: Option<String>,
    mode: Option<String>,
    env: BTreeMap<String, String>,
    prompt_stdin: bool,
    append_prompt: bool,
    timeout_secs: u64,
}

impl CliAgent {
    pub fn new_from_options(name: &str, opts: toml::value::Table) -> Self {
        let preset = CliPreset::for_agent(name);
        Self {
            name: table_string(&opts, "name").unwrap_or_else(|| name.to_string()),
            command: table_string(&opts, "command")
                .or_else(|| table_string(&opts, "cmd"))
                .or_else(|| table_string(&opts, "cli_path"))
                .unwrap_or_else(|| preset.command.to_string()),
            args: table_string_vec(&opts, "args").unwrap_or_else(|| preset.args()),
            work_dir: table_string(&opts, "work_dir").unwrap_or_else(|| ".".to_string()),
            model: table_string(&opts, "model"),
            mode: table_string(&opts, "mode"),
            env: table_env(&opts),
            prompt_stdin: table_bool(&opts, "prompt_stdin").unwrap_or(preset.prompt_stdin),
            append_prompt: table_bool(&opts, "append_prompt").unwrap_or(preset.append_prompt),
            timeout_secs: table_u64(&opts, "timeout_secs")
                .or_else(|| table_u64(&opts, "timeout_seconds"))
                .or_else(|| table_u64(&opts, "timeout_mins").map(|mins| mins * 60))
                .unwrap_or(1800),
        }
    }
}

#[async_trait]
impl Agent for CliAgent {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> AgentCapabilities {
        AgentCapabilities {
            images: true,
            files: true,
            model_switching: self.model.is_some(),
            streaming_output: true,
            ..AgentCapabilities::default()
        }
    }

    async fn start_session(&self, session_id: Option<String>) -> Result<Arc<dyn AgentSession>> {
        Ok(Arc::new(CliAgentSession::new(self.clone(), session_id)))
    }

    async fn list_sessions(&self) -> Result<Vec<AgentSessionInfo>> {
        Ok(Vec::new())
    }

    async fn stop(&self) -> Result<()> {
        Ok(())
    }
}

pub struct CliAgentSession {
    agent: CliAgent,
    id: String,
    alive: AtomicBool,
    tx: mpsc::UnboundedSender<Event>,
    rx: Mutex<mpsc::UnboundedReceiver<Event>>,
}

impl CliAgentSession {
    fn new(agent: CliAgent, session_id: Option<String>) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let (tx, rx) = mpsc::unbounded_channel();
        let id = session_id.unwrap_or_else(|| {
            format!(
                "{}-cli-session-{}",
                agent.name,
                NEXT_ID.fetch_add(1, Ordering::SeqCst)
            )
        });
        Self {
            agent,
            id,
            alive: AtomicBool::new(true),
            tx,
            rx: Mutex::new(rx),
        }
    }
}

#[async_trait]
impl AgentSession for CliAgentSession {
    async fn send(
        &self,
        prompt: String,
        images: Vec<ImageAttachment>,
        files: Vec<FileAttachment>,
    ) -> Result<()> {
        if !self.alive() {
            return Err(anyhow!("cli session is closed"));
        }
        let agent = self.agent.clone();
        let session_id = self.id.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = run_cli_turn(&agent, &session_id, prompt, images, files).await;
            let event = match result {
                Ok(content) => Event::result(content, session_id),
                Err(err) => Event::error(err.to_string()),
            };
            let _ = tx.send(event);
        });
        Ok(())
    }

    async fn respond_permission(
        &self,
        _request_id: String,
        _result: PermissionResult,
    ) -> Result<()> {
        Err(anyhow!("cli agent does not support permission callbacks"))
    }

    async fn recv_event(&self) -> Option<Event> {
        self.rx.lock().await.recv().await
    }

    fn current_session_id(&self) -> String {
        self.id.clone()
    }

    fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    async fn close(&self) -> Result<()> {
        self.alive.store(false, Ordering::SeqCst);
        Ok(())
    }
}

async fn run_cli_turn(
    agent: &CliAgent,
    session_id: &str,
    prompt: String,
    images: Vec<ImageAttachment>,
    files: Vec<FileAttachment>,
) -> Result<String> {
    let prompt = prompt_with_attachment_notes(prompt, images, files);
    let mut args = render_args(agent, session_id, &prompt);
    if agent.append_prompt && !args.iter().any(|arg| arg.contains(&prompt)) && !agent.prompt_stdin {
        args.push(prompt.clone());
    }
    let mut command = Command::new(&agent.command);
    command
        .args(&args)
        .current_dir(&agent.work_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if agent.prompt_stdin {
        command.stdin(Stdio::piped());
    }
    for (key, value) in &agent.env {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .map_err(|err| anyhow!("start {} CLI `{}`: {err}", agent.name, agent.command))?;
    if agent.prompt_stdin {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(prompt.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
        }
    }
    let output = tokio::time::timeout(
        Duration::from_secs(agent.timeout_secs),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| anyhow!("{} CLI turn timed out", agent.name))??;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        return Err(anyhow!(
            "{} CLI exited with {}: {}",
            agent.name,
            output.status,
            stderr.trim()
        ));
    }
    Ok(extract_cli_output(&stdout).unwrap_or_else(|| stdout.trim().to_string()))
}

fn prompt_with_attachment_notes(
    mut prompt: String,
    images: Vec<ImageAttachment>,
    files: Vec<FileAttachment>,
) -> String {
    let mut notes = Vec::new();
    for image in images {
        notes.push(format!(
            "image: mime={}, file={}",
            image.mime_type,
            image.file_name.unwrap_or_default()
        ));
    }
    for file in files {
        notes.push(format!(
            "file: mime={}, file={}",
            file.mime_type,
            file.file_name.unwrap_or_default()
        ));
    }
    if !notes.is_empty() {
        prompt.push_str("\n\n[attachments]\n");
        prompt.push_str(&notes.join("\n"));
    }
    prompt
}

fn render_args(agent: &CliAgent, session_id: &str, prompt: &str) -> Vec<String> {
    agent
        .args
        .iter()
        .map(|arg| {
            arg.replace("{prompt}", prompt)
                .replace("{session_id}", session_id)
                .replace("{model}", agent.model.as_deref().unwrap_or(""))
                .replace("{mode}", agent.mode.as_deref().unwrap_or(""))
        })
        .filter(|arg| !arg.is_empty())
        .collect()
}

fn extract_cli_output(stdout: &str) -> Option<String> {
    let mut final_text = None;
    let mut deltas = String::new();
    for line in stdout.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(text) = value
            .get("content")
            .or_else(|| value.get("text"))
            .or_else(|| value.get("message"))
            .and_then(Value::as_str)
        {
            final_text = Some(text.to_string());
        }
        if let Some(delta) = value.get("delta").and_then(Value::as_str) {
            deltas.push_str(delta);
        }
        if let Some(text) = value.pointer("/message/content").and_then(Value::as_str) {
            final_text = Some(text.to_string());
        }
    }
    final_text.or_else(|| (!deltas.is_empty()).then_some(deltas))
}

struct CliPreset {
    command: &'static str,
    args: &'static [&'static str],
    prompt_stdin: bool,
    append_prompt: bool,
}

impl CliPreset {
    fn for_agent(name: &str) -> Self {
        match name {
            "claudecode" | "claude-code" | "claude" => Self {
                command: "claude",
                args: &["-p", "{prompt}"],
                prompt_stdin: false,
                append_prompt: false,
            },
            "cursor" | "cursor-agent" => Self {
                command: "agent",
                args: &["--print", "--output-format", "stream-json"],
                prompt_stdin: true,
                append_prompt: false,
            },
            "gemini" | "gemini-cli" => Self {
                command: "gemini",
                args: &["-p", "--output-format", "stream-json"],
                prompt_stdin: true,
                append_prompt: false,
            },
            "qoder" | "qoder-cli" => Self {
                command: "qodercli",
                args: &["-p", "{prompt}", "-f", "stream-json"],
                prompt_stdin: false,
                append_prompt: false,
            },
            "opencode" => Self {
                command: "opencode",
                args: &["run", "--format", "json"],
                prompt_stdin: true,
                append_prompt: false,
            },
            "iflow" | "iflow-cli" => Self {
                command: "iflow",
                args: &["-i"],
                prompt_stdin: true,
                append_prompt: false,
            },
            "kimi" | "kimi-cli" => Self {
                command: "kimi",
                args: &["--print", "--output-format", "stream-json"],
                prompt_stdin: true,
                append_prompt: false,
            },
            "pi" => Self {
                command: "pi",
                args: &["{prompt}"],
                prompt_stdin: false,
                append_prompt: false,
            },
            "devin" => Self {
                command: "devin",
                args: &["acp"],
                prompt_stdin: true,
                append_prompt: false,
            },
            "acp" => Self {
                command: "acp-agent",
                args: &[],
                prompt_stdin: true,
                append_prompt: false,
            },
            _ => Self {
                command: "agent",
                args: &[],
                prompt_stdin: true,
                append_prompt: false,
            },
        }
    }

    fn args(&self) -> Vec<String> {
        self.args.iter().map(|arg| arg.to_string()).collect()
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

fn table_bool(opts: &toml::value::Table, key: &str) -> Option<bool> {
    opts.get(key).and_then(|v| v.as_bool())
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

#[cfg(test)]
mod tests {
    use super::{extract_cli_output, CliAgent};

    #[test]
    fn cli_json_output_extracts_text() {
        assert_eq!(
            extract_cli_output("{\"delta\":\"hi\"}\n{\"delta\":\" there\"}").as_deref(),
            Some("hi there")
        );
        assert_eq!(
            extract_cli_output("{\"content\":\"final\"}").as_deref(),
            Some("final")
        );
    }

    #[test]
    fn cli_agent_preset_can_be_overridden() {
        let mut opts = toml::value::Table::new();
        opts.insert(
            "command".to_string(),
            toml::Value::String("echo".to_string()),
        );
        let agent = CliAgent::new_from_options("gemini", opts);
        assert_eq!(agent.command, "echo");
        assert_eq!(agent.name, "gemini");
    }
}
