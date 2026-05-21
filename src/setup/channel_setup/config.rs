use super::args::SetupArgs;
use anyhow::{anyhow, Result};

pub fn update_platform_config(
    args: &SetupArgs,
    platform_id: &str,
    platform_type: &str,
    options: Vec<(&str, String)>,
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
        .position(|p| p.get("name").and_then(toml::Value::as_str) == Some(args.project.as_str()))
        .unwrap_or_else(|| {
            projects.push(default_project(&args.project, &args.work_dir));
            projects.len() - 1
        });
    let project = projects[project_index]
        .as_table_mut()
        .ok_or_else(|| anyhow!("project must be a table"))?;
    ensure_default_platform(project, platform_id)?;
    let platforms = project
        .entry("platforms".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| anyhow!("project.platforms must be an array"))?;
    let platform_index = platforms
        .iter()
        .position(|p| p.get("id").and_then(toml::Value::as_str) == Some(platform_id))
        .unwrap_or_else(|| {
            platforms.push(toml::Value::Table(toml::map::Map::new()));
            platforms.len() - 1
        });
    platforms[platform_index] = platform_value(platform_id, platform_type, options);
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
    project.insert(
        "default_platforms".to_string(),
        toml::Value::Array(Vec::new()),
    );
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
    defaults.retain(|value| value.as_str() != Some("mock"));
    if !defaults
        .iter()
        .any(|value| value.as_str() == Some(platform_id))
    {
        defaults.push(toml::Value::String(platform_id.to_string()));
    }
    Ok(())
}

fn platform_value(
    platform_id: &str,
    platform_type: &str,
    options: Vec<(&str, String)>,
) -> toml::Value {
    let mut platform = toml::map::Map::new();
    platform.insert(
        "id".to_string(),
        toml::Value::String(platform_id.to_string()),
    );
    platform.insert(
        "type".to_string(),
        toml::Value::String(platform_type.to_string()),
    );
    let mut opts = toml::map::Map::new();
    for (key, value) in options {
        opts.insert(key.to_string(), toml::Value::String(value));
    }
    platform.insert("options".to_string(), toml::Value::Table(opts));
    toml::Value::Table(platform)
}
