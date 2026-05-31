use super::options::AcpAgent;
use super::rpc::AcpRpc;
use crate::core::AgentSessionInfo;
use crate::util::path_env::{apply_enriched_path_tokio, resolve_executable};
use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

const LIST_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Deserialize)]
struct ProbeInitializeResult {
    #[serde(rename = "agentCapabilities")]
    agent_capabilities: Option<ProbeAgentCapabilities>,
}

#[derive(Debug, Deserialize)]
struct ProbeAgentCapabilities {
    #[serde(rename = "sessionCapabilities")]
    session_capabilities: Option<ProbeSessionCapabilities>,
}

#[derive(Debug, Deserialize)]
struct ProbeSessionCapabilities {
    list: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct SessionListResult {
    sessions: Vec<SessionListEntry>,
}

#[derive(Debug, Deserialize)]
struct SessionListEntry {
    #[serde(rename = "sessionId")]
    session_id: String,
    cwd: Option<String>,
    title: Option<String>,
}

pub(super) async fn list_sessions(agent: &AcpAgent) -> Result<Vec<AgentSessionInfo>> {
    if agent
        .state
        .list_unsupported
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return Ok(Vec::new());
    }

    let work_dir = Path::new(&agent.work_dir);
    let abs_work_dir = work_dir
        .canonicalize()
        .unwrap_or_else(|_| work_dir.to_path_buf());
    let command = resolve_executable(&agent.command)
        .map_err(|err| anyhow!("acp: resolve command `{}`: {err}", agent.command))?;

    let probe = timeout(
        LIST_PROBE_TIMEOUT,
        probe_list_sessions(agent, &command, &abs_work_dir),
    )
    .await
    .map_err(|_| anyhow!("acp: session/list probe timed out"))??;

    Ok(probe)
}

async fn probe_list_sessions(
    agent: &AcpAgent,
    command: &Path,
    abs_work_dir: &Path,
) -> Result<Vec<AgentSessionInfo>> {
    let mut child_cmd = tokio::process::Command::new(command);
    child_cmd
        .args(&agent.args)
        .current_dir(abs_work_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    apply_enriched_path_tokio(&mut child_cmd);
    for (key, value) in &agent.env {
        child_cmd.env(key, value);
    }

    let mut child = child_cmd
        .spawn()
        .map_err(|err| anyhow!("acp: probe start `{}`: {err}", command.display()))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("acp: probe missing stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("acp: probe missing stdout"))?;

    let on_notify = Arc::new(|_method: String, _params: Value| {});
    let on_request = Arc::new(|_method: String, _id: Value, _params: Value| {});
    let rpc = AcpRpc::new(stdin, on_notify, on_request);
    rpc.spawn_read_loop(stdout);

    let result = probe_list_on_transport(agent, &rpc, abs_work_dir).await;
    let _ = child.kill().await;
    result
}

async fn probe_list_on_transport(
    agent: &AcpAgent,
    rpc: &AcpRpc,
    abs_work_dir: &Path,
) -> Result<Vec<AgentSessionInfo>> {
    let init_params = json!({
        "protocolVersion": 1,
        "clientCapabilities": {
            "fs": { "readTextFile": false, "writeTextFile": false },
            "terminal": false
        },
        "clientInfo": { "name": "agentlink", "version": env!("CARGO_PKG_VERSION") }
    });
    let init_result = rpc.call("initialize", init_params).await?;
    let init: ProbeInitializeResult = serde_json::from_value(init_result)
        .map_err(|err| anyhow!("acp: probe parse initialize: {err}"))?;
    let list_supported = init
        .agent_capabilities
        .as_ref()
        .and_then(|c| c.session_capabilities.as_ref())
        .and_then(|s| s.list.as_ref())
        .is_some();
    if !list_supported {
        agent
            .state
            .list_unsupported
            .store(true, std::sync::atomic::Ordering::Relaxed);
        return Ok(Vec::new());
    }

    let list_params = json!({ "cwd": abs_work_dir.to_string_lossy() });
    let list_raw = match rpc.call("session/list", list_params).await {
        Ok(v) => v,
        Err(err) => {
            if rpc_error_unsupported(&err) {
                agent
                    .state
                    .list_unsupported
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                return Ok(Vec::new());
            }
            return Err(err);
        }
    };

    let parsed: SessionListResult = serde_json::from_value(list_raw)
        .map_err(|err| anyhow!("acp: parse session/list: {err}"))?;
    let cwd = abs_work_dir.to_string_lossy();
    let out = parsed
        .sessions
        .into_iter()
        .filter(|entry| {
            entry.session_id.is_empty()
                || entry
                    .cwd
                    .as_deref()
                    .is_none_or(|path| paths_equal(path, cwd.as_ref()))
        })
        .map(|entry| AgentSessionInfo {
            id: entry.session_id,
            summary: entry.title.filter(|s| !s.trim().is_empty()),
            message_count: 0,
        })
        .collect();
    Ok(out)
}

fn rpc_error_unsupported(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    msg.contains("json-rpc -32601")
        || msg.contains("json-rpc -32600")
        || msg.contains("method not found")
}

fn paths_equal(left: &str, right: &str) -> bool {
    let left = Path::new(left);
    let right = Path::new(right);
    let left_path = left.canonicalize().unwrap_or_else(|_| left.to_path_buf());
    let right_path = right.canonicalize().unwrap_or_else(|_| right.to_path_buf());
    left_path == right_path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_equal_matches_cleaned_paths() {
        assert!(paths_equal("/tmp/a", "/tmp/a"));
    }
}
