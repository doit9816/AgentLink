use crate::setup::feishu_setup::run_feishu_command;
use anyhow::{anyhow, Result};

#[derive(Debug, Clone)]
struct SetupArgs {
    platform: String,
    config_path: String,
    project: String,
    work_dir: String,
    ws_url: String,
    token: Option<String>,
    passthrough: Vec<String>,
}

pub async fn run_setup_command(args: Vec<String>) -> Result<()> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_setup_help();
        return Ok(());
    }
    let args = parse_args(&args)?;
    match args.platform.as_str() {
        "feishu" | "lark" => run_feishu_scan_setup(args).await,
        "qq" => setup_qq_external_scan(&args),
        "weixin" | "wechat-personal" => setup_weixin_external_scan(&args),
        "all" => {
            print_support_matrix();
            Ok(())
        }
        unsupported => Err(anyhow!(
            "platform `{}` does not support QR setup in this bridge. {}",
            unsupported,
            unsupported_reason(unsupported)
        )),
    }
}

fn parse_args(values: &[String]) -> Result<SetupArgs> {
    let mut args = SetupArgs {
        platform: String::new(),
        config_path: "agentlink.toml".to_string(),
        project: "demo".to_string(),
        work_dir: std::env::current_dir()?.to_string_lossy().to_string(),
        ws_url: "ws://127.0.0.1:3001".to_string(),
        token: None,
        passthrough: Vec::new(),
    };
    let mut i = 0;
    while i < values.len() {
        match values[i].as_str() {
            "--platform" | "--channel" | "-p" => {
                i += 1;
                args.platform = required(values, i, "--platform")?;
            }
            "--config" | "-c" => {
                i += 1;
                args.config_path = required(values, i, "--config")?;
            }
            "--project" => {
                i += 1;
                args.project = required(values, i, "--project")?;
            }
            "--work-dir" => {
                i += 1;
                args.work_dir = required(values, i, "--work-dir")?;
            }
            "--ws-url" => {
                i += 1;
                args.ws_url = required(values, i, "--ws-url")?;
            }
            "--token" => {
                i += 1;
                args.token = Some(required(values, i, "--token")?);
            }
            flag @ ("--app" | "--app-id" | "--app-secret" | "--timeout" | "--debug"
            | "--dry-run") => {
                args.passthrough.push(flag.to_string());
                if flag != "--debug" && flag != "--dry-run" {
                    i += 1;
                    args.passthrough.push(required(values, i, flag)?);
                }
            }
            value if args.platform.is_empty() && !value.starts_with('-') => {
                args.platform = value.to_string();
            }
            other => return Err(anyhow!("unknown setup argument `{other}`")),
        }
        i += 1;
    }
    if args.platform.trim().is_empty() {
        return Err(anyhow!("setup requires --platform <name>"));
    }
    args.platform = args.platform.trim().to_ascii_lowercase();
    Ok(args)
}

async fn run_feishu_scan_setup(args: SetupArgs) -> Result<()> {
    let mut feishu_args = vec![
        "setup".to_string(),
        "--config".to_string(),
        args.config_path,
        "--project".to_string(),
        args.project,
        "--work-dir".to_string(),
        args.work_dir,
        "--platform-type".to_string(),
        args.platform,
    ];
    feishu_args.extend(args.passthrough);
    run_feishu_command(feishu_args).await
}

fn setup_qq_external_scan(args: &SetupArgs) -> Result<()> {
    update_platform_config(
        args,
        "qq",
        "qq",
        vec![
            ("name", "qq".to_string()),
            ("ws_url", args.ws_url.clone()),
            ("access_token", args.token.clone().unwrap_or_default()),
        ],
    )?;
    println!("QQ configured for external QR login.");
    println!("  config:   {}", args.config_path);
    println!("  project:  {}", args.project);
    println!("  platform: qq");
    println!("  ws_url:   {}", args.ws_url);
    println!();
    println!("Next:");
    println!("  1. Start NapCat/LLOneBot or another OneBot v11 gateway.");
    println!("  2. Scan the QQ login QR code in that gateway.");
    println!("  3. Keep the gateway WebSocket at the ws_url above.");
    println!("  4. Run: agentlink --config {}", args.config_path);
    Ok(())
}

fn setup_weixin_external_scan(args: &SetupArgs) -> Result<()> {
    update_platform_config(
        args,
        "weixin",
        "weixin",
        vec![
            ("name", "weixin".to_string()),
            (
                "token",
                args.token
                    .clone()
                    .unwrap_or_else(|| "paste-openclaw-ilink-token".to_string()),
            ),
            ("api_base", "https://ilinkai.weixin.qq.com".to_string()),
        ],
    )?;
    println!("Weixin personal account configured for external QR login.");
    println!("  config:   {}", args.config_path);
    println!("  project:  {}", args.project);
    println!("  platform: weixin");
    println!();
    println!("Next:");
    println!("  1. Use OpenClaw/iLink compatible gateway to scan-login the WeChat account.");
    println!("  2. Put the gateway bearer token into the weixin token field if it changed.");
    println!("  3. Run: agentlink --config {}", args.config_path);
    Ok(())
}

fn update_platform_config(
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

fn required(values: &[String], index: usize, flag: &str) -> Result<String> {
    values
        .get(index)
        .cloned()
        .ok_or_else(|| anyhow!("{flag} requires a value"))
}

fn unsupported_reason(platform: &str) -> &'static str {
    match platform {
        "dingtalk" => "DingTalk bot apps are configured with client_id/client_secret/robot_code, not QR onboarding.",
        "telegram" => "Telegram bots are created by BotFather and use a bot token.",
        "slack" => "Slack apps use app-level and bot tokens; Socket Mode has no QR onboarding.",
        "discord" => "Discord bots use a bot token from Developer Portal.",
        "line" => "Line Messaging API uses channel secret and channel access token.",
        "wecom" => "WeCom apps use corp_id/corp_secret/agent_id and callback settings.",
        "max" => "MAX bots use a bot token.",
        "qqbot" => "QQ official bot uses app_id/app_secret for Gateway auth.",
        "weibo" => "Weibo bot integration uses app_id/app_secret and ws token endpoint.",
        "http" | "bridge" => "This is an adapter protocol, not a login channel.",
        "mock" => "Mock channel does not need onboarding.",
        _ => "No QR onboarding implementation is available.",
    }
}

fn print_support_matrix() {
    println!("QR setup support:");
    println!("  feishu/lark  supported, bridge prints QR and writes app_id/app_secret");
    println!("  qq           external QR in NapCat/LLOneBot; bridge writes OneBot config");
    println!("  weixin       external QR in OpenClaw/iLink gateway; bridge writes config shell");
    println!("  others       no QR onboarding; use platform app tokens/secrets");
}

fn print_setup_help() {
    println!(
        r#"agentlink setup --platform <name> [options]

Examples:
  agentlink setup --platform feishu --config agentlink.toml --project demo
  agentlink setup --platform qq --config agentlink.toml --ws-url ws://127.0.0.1:3001
  agentlink setup --platform weixin --config agentlink.toml --token <gateway-token>
  agentlink setup --platform all

Options:
  --platform, --channel <name>  feishu, lark, qq, weixin, or all
  --config <path>              Config file to create/update. Default: agentlink.toml
  --project <name>             Project name. Default: demo
  --work-dir <path>            Codex work_dir when creating config
  --ws-url <url>               QQ OneBot gateway WebSocket URL
  --token <token>              External gateway token
  --app/--app-id/--app-secret  Passed through to feishu/lark setup
"#
    );
}
