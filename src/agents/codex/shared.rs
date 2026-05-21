use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub(super) fn mode_settings(mode: &str) -> (&'static str, &'static str) {
    match mode {
        "auto-edit" | "full-auto" => ("never", "workspace-write"),
        "yolo" => ("never", "danger-full-access"),
        _ => ("on-request", "read-only"),
    }
}

pub(super) fn table_string(opts: &toml::value::Table, key: &str) -> Option<String> {
    opts.get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

pub(super) fn table_string_vec(opts: &toml::value::Table, key: &str) -> Option<Vec<String>> {
    opts.get(key).and_then(|value| {
        value.as_array().map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
    })
}

pub(super) fn table_u64(opts: &toml::value::Table, key: &str) -> Option<u64> {
    opts.get(key)
        .and_then(|value| value.as_integer())
        .map(|value| value as u64)
}

pub(super) fn table_env(opts: &toml::value::Table) -> BTreeMap<String, String> {
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

pub(super) fn normalize_codex_backend(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "app_server" | "appserver" | "stdio" => "app-server".to_string(),
        "cli" | "codex-cli" => "exec".to_string(),
        other => other.to_string(),
    }
}

pub(super) fn split_cli_words(value: &str) -> Vec<String> {
    value.split_whitespace().map(str::to_string).collect()
}

pub(super) fn approval_summary(params: &Value) -> String {
    params
        .get("command")
        .and_then(Value::as_str)
        .or_else(|| params.get("reason").and_then(Value::as_str))
        .unwrap_or_else(|| params.as_str().unwrap_or(""))
        .to_string()
}

pub(super) async fn attachment_path_or_temp(
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

pub(super) fn shell_json(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("{value:?}"))
}

pub(super) fn ensure_work_dir_exists(path: &str) -> Result<()> {
    if PathBuf::from(path).exists() {
        Ok(())
    } else {
        Err(anyhow!("Codex work_dir does not exist: {path}"))
    }
}
