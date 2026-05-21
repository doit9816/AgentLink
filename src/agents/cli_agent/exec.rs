use super::CliAgent;
use crate::core::{FileAttachment, ImageAttachment};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

pub(super) async fn run_cli_turn(
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

pub(super) fn extract_cli_output(stdout: &str) -> Option<String> {
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
