mod app_server;
mod exec;
mod exec_args;
mod exec_output;
mod exec_prompt;
mod shared;

use crate::core::{Agent, AgentCapabilities, AgentSession, AgentSessionInfo, SessionStartRequest};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::sync::Arc;

pub use app_server::extract_completed_agent_message;

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
        let cli_path = shared::table_string(&opts, "cli_path");
        let cli_path_parts = cli_path
            .as_deref()
            .map(shared::split_cli_words)
            .unwrap_or_default();
        let cli_path_bin = cli_path_parts.first().cloned();
        let cli_path_args = if cli_path_parts.len() > 1 {
            cli_path_parts[1..].to_vec()
        } else {
            Vec::new()
        };
        Self {
            work_dir: shared::table_string(&opts, "work_dir").unwrap_or_else(|| ".".to_string()),
            model: shared::table_string(&opts, "model"),
            reasoning_effort: shared::table_string(&opts, "reasoning_effort")
                .or_else(|| shared::table_string(&opts, "model_reasoning_effort")),
            mode: shared::table_string(&opts, "mode").unwrap_or_else(|| "suggest".to_string()),
            backend: shared::table_string(&opts, "backend")
                .map(|value| shared::normalize_codex_backend(&value))
                .unwrap_or_else(|| "exec".to_string()),
            codex_bin: shared::table_string(&opts, "codex_bin")
                .or_else(|| shared::table_string(&opts, "command"))
                .or_else(|| shared::table_string(&opts, "cmd"))
                .or(cli_path_bin)
                .unwrap_or_else(|| "codex".to_string()),
            cli_extra_args: shared::table_string_vec(&opts, "args").unwrap_or(cli_path_args),
            codex_home: shared::table_string(&opts, "codex_home"),
            base_url: shared::table_string(&opts, "base_url")
                .or_else(|| shared::table_string(&opts, "openai_base_url")),
            model_provider: shared::table_string(&opts, "model_provider"),
            env: shared::table_env(&opts),
            timeout_secs: shared::table_u64(&opts, "timeout_secs")
                .or_else(|| shared::table_u64(&opts, "timeout_seconds"))
                .or_else(|| shared::table_u64(&opts, "timeout_mins").map(|mins| mins * 60))
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

    async fn start_session(&self, request: SessionStartRequest) -> Result<Arc<dyn AgentSession>> {
        let mut agent = self.clone();
        if let Some(dir) = request.work_dir {
            agent.work_dir = dir;
        }
        let resume_id = request.resume_session_id;
        match agent.backend.as_str() {
            "app-server" => Ok(Arc::new(
                app_server::CodexAppServerSession::start(agent.clone(), resume_id).await?,
            )),
            "exec" => Ok(Arc::new(exec::CodexExecSession::new(agent, resume_id))),
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

#[cfg(test)]
mod tests {
    use super::exec_args::build_codex_exec_args;
    use super::exec_output::{parse_codex_exec_stdout, CodexExecOutput};
    use super::CodexAgent;

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
