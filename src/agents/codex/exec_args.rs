use super::shared::shell_json;
use super::CodexAgent;
use std::collections::BTreeMap;

pub(crate) fn build_codex_exec_args(
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

pub(crate) fn codex_exec_env(agent: &CodexAgent) -> BTreeMap<String, String> {
    let mut env = agent.env.clone();
    if let Some(codex_home) = &agent.codex_home {
        env.insert("CODEX_HOME".to_string(), codex_home.clone());
    }
    env
}
