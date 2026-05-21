use super::args::FeishuSetupArgs;
use super::registration::{RegistrationResult, OPEN_LARK_BASE};
use anyhow::{anyhow, Result};

pub(super) fn write_feishu_config(
    args: &FeishuSetupArgs,
    result: &RegistrationResult,
    platform_type: &str,
) -> Result<()> {
    let mut root = if std::path::Path::new(&args.config_path).exists() {
        let raw = std::fs::read_to_string(&args.config_path)?;
        raw.parse::<toml::Value>()?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let table = root
        .as_table_mut()
        .ok_or_else(|| anyhow!("config root must be a TOML table"))?;
    table
        .entry("data_dir".to_string())
        .or_insert(toml::Value::String("./data".to_string()));

    let projects = table
        .entry("projects".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| anyhow!("projects must be an array"))?;

    let project_index = projects
        .iter()
        .position(|project| {
            project.get("name").and_then(toml::Value::as_str) == Some(args.project.as_str())
        })
        .unwrap_or_else(|| {
            projects.push(default_project(&args.project, &args.work_dir));
            projects.len() - 1
        });
    let project = projects[project_index]
        .as_table_mut()
        .ok_or_else(|| anyhow!("project must be a table"))?;
    ensure_default_platform(project, platform_type)?;
    let platforms = project
        .entry("platforms".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| anyhow!("project.platforms must be an array"))?;
    let platform_index = platforms
        .iter()
        .position(|platform| {
            platform
                .get("type")
                .and_then(toml::Value::as_str)
                .is_some_and(|value| value == "feishu" || value == "lark")
        })
        .unwrap_or_else(|| {
            platforms.push(toml::Value::Table(toml::map::Map::new()));
            platforms.len() - 1
        });
    platforms[platform_index] = feishu_platform_value(args, result, platform_type);
    let rendered = toml::to_string_pretty(&root)?;
    if let Some(parent) = std::path::Path::new(&args.config_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(&args.config_path, rendered)?;
    Ok(())
}

fn default_project(name: &str, work_dir: &str) -> toml::Value {
    let mut project = toml::map::Map::new();
    project.insert("name".to_string(), toml::Value::String(name.to_string()));
    let mut agent = toml::map::Map::new();
    agent.insert("type".to_string(), toml::Value::String("codex".to_string()));
    let mut options = toml::map::Map::new();
    options.insert(
        "work_dir".to_string(),
        toml::Value::String(work_dir.to_string()),
    );
    options.insert(
        "mode".to_string(),
        toml::Value::String("suggest".to_string()),
    );
    options.insert(
        "codex_bin".to_string(),
        toml::Value::String("codex".to_string()),
    );
    agent.insert("options".to_string(), toml::Value::Table(options));
    project.insert("agent".to_string(), toml::Value::Table(agent));
    project.insert(
        "default_platforms".to_string(),
        toml::Value::Array(Vec::new()),
    );
    project.insert("platforms".to_string(), toml::Value::Array(Vec::new()));
    toml::Value::Table(project)
}

fn ensure_default_platform(
    project: &mut toml::map::Map<String, toml::Value>,
    platform_id: &str,
) -> Result<()> {
    let defaults = project
        .entry("default_platforms".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| anyhow!("project.default_platforms must be an array"))?;
    if !defaults
        .iter()
        .any(|value| value.as_str() == Some(platform_id))
    {
        defaults.push(toml::Value::String(platform_id.to_string()));
    }
    Ok(())
}

fn feishu_platform_value(
    args: &FeishuSetupArgs,
    result: &RegistrationResult,
    platform_type: &str,
) -> toml::Value {
    let mut platform = toml::map::Map::new();
    platform.insert(
        "id".to_string(),
        toml::Value::String(platform_type.to_string()),
    );
    platform.insert(
        "type".to_string(),
        toml::Value::String(platform_type.to_string()),
    );
    let mut options = toml::map::Map::new();
    options.insert(
        "name".to_string(),
        toml::Value::String(platform_type.to_string()),
    );
    options.insert(
        "listen".to_string(),
        toml::Value::String(args.listen.clone()),
    );
    options.insert(
        "callback_path".to_string(),
        toml::Value::String(args.callback_path.clone()),
    );
    options.insert(
        "connection_mode".to_string(),
        toml::Value::String("websocket".to_string()),
    );
    options.insert(
        "app_id".to_string(),
        toml::Value::String(result.app_id.clone()),
    );
    options.insert(
        "app_secret".to_string(),
        toml::Value::String(result.app_secret.clone()),
    );
    if platform_type == "lark" {
        options.insert(
            "api_base".to_string(),
            toml::Value::String(OPEN_LARK_BASE.to_string()),
        );
    }
    if args.dry_run {
        options.insert("dry_run".to_string(), toml::Value::Boolean(true));
    }
    platform.insert("options".to_string(), toml::Value::Table(options));
    toml::Value::Table(platform)
}
