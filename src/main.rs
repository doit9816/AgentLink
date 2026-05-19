use agentlink::agents::cli_agent::CliAgent;
use agentlink::agents::codex::CodexAgent;
use agentlink::app::config::Config;
use agentlink::channels::bridge::{BridgePlatform, BridgePlatformConfig};
use agentlink::channels::dingtalk::{DingTalkPlatform, DingTalkPlatformConfig};
use agentlink::channels::discord::{DiscordPlatform, DiscordPlatformConfig};
use agentlink::channels::feishu::{FeishuPlatform, FeishuPlatformConfig};
use agentlink::channels::http_channel::{HttpPlatform, HttpPlatformConfig};
use agentlink::channels::line::{LinePlatform, LinePlatformConfig};
use agentlink::channels::max::{max_config_from_options, MaxPlatform};
use agentlink::channels::qqbot::{qqbot_config_from_options, QqBotPlatform};
use agentlink::channels::qq::{QqPlatform, QqPlatformConfig};
use agentlink::channels::slack::{SlackPlatform, SlackPlatformConfig};
use agentlink::channels::telegram::{TelegramPlatform, TelegramPlatformConfig};
use agentlink::channels::wecom::{WeComPlatform, WeComPlatformConfig};
use agentlink::channels::weibo::{weibo_config_from_options, WeiboPlatform};
use agentlink::channels::weixin::{weixin_config_from_options, WeixinPlatform};
use agentlink::setup::channel_setup::run_setup_command;
use agentlink::setup::feishu_setup::run_feishu_command;
use agentlink::testing::mock::{MockAgent, MockPlatform};
use agentlink::{Engine, Registry, SessionStore};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use toml::Value as TomlValue;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    let command = Command::parse(std::env::args().skip(1).collect())?;
    match command {
        Command::Help => {
            print_help();
            return Ok(());
        }
        Command::Version => {
            println!("agentlink {VERSION}");
            return Ok(());
        }
        Command::ValidateConfig { path } => {
            let config = Config::load(&path)?;
            validate_with_registry(&config, &default_registry())?;
            println!("config ok: {path}");
            return Ok(());
        }
        Command::Run {
            config_path,
            platforms,
            projects,
        } => run(&config_path, &platforms, &projects).await,
        Command::Feishu { args } => run_feishu_command(args).await,
        Command::Setup { args } => run_setup_command(args).await,
    }
}

async fn run(
    config_path: &str,
    selected_platforms: &[String],
    selected_projects: &[String],
) -> Result<()> {
    let config = Config::load(config_path)?;
    let registry = default_registry();
    validate_with_registry(&config, &registry)?;

    std::fs::create_dir_all(&config.data_dir)?;
    let mut engines = Vec::new();
    for project in &config.projects {
        if !selected_projects.is_empty()
            && !selected_projects.iter().any(|name| name == &project.name)
        {
            continue;
        }
        let agent = registry.create_agent(&project.agent.kind, project.agent.options.clone())?;
        let mut platforms = Vec::new();
        let selected = project.selected_platforms(selected_platforms)?;
        for platform in selected {
            platforms.push(registry.create_platform(&platform.kind, platform_options(platform))?);
        }
        let store_path = PathBuf::from(&config.data_dir).join(format!("{}.sqlite3", project.name));
        let store = SessionStore::open(store_path)?;
        let engine = Engine::new(project.name.clone(), agent, platforms, store);
        engine.start().await?;
        let platform_names = project
            .selected_platforms(selected_platforms)?
            .iter()
            .map(|platform| platform.display_name())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "project `{}` started; platforms: {}",
            project.name, platform_names
        );
        engines.push(engine);
    }
    if engines.is_empty() {
        anyhow::bail!("no project selected");
    }

    tracing::info!("agentlink started; press Ctrl+C to stop");
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutdown signal received");
    for engine in engines {
        let _ = engine.stop().await;
    }
    Ok(())
}

fn default_registry() -> Registry {
    let mut registry = Registry::new();
    registry.register_agent("mock", |_| Ok(Arc::new(MockAgent::new())));
    registry.register_agent("codex", |opts| {
        Ok(Arc::new(CodexAgent::new_from_options(opts)))
    });
    for name in [
        "cli",
        "claudecode",
        "claude-code",
        "claude",
        "cursor",
        "cursor-agent",
        "gemini",
        "gemini-cli",
        "kimi",
        "kimi-cli",
        "qoder",
        "qoder-cli",
        "opencode",
        "iflow",
        "iflow-cli",
        "pi",
        "devin",
        "acp",
    ] {
        registry.register_agent(name, move |opts| {
            Ok(Arc::new(CliAgent::new_from_options(name, opts)))
        });
    }
    registry.register_platform("mock", |opts| {
        let name = opts.get("name").and_then(|v| v.as_str()).unwrap_or("mock");
        Ok(MockPlatform::new(name))
    });
    registry.register_platform("http", |opts| {
        let config = HttpPlatformConfig::try_from(opts)?;
        Ok(HttpPlatform::new(config))
    });
    registry.register_platform("bridge", |opts| {
        let config = BridgePlatformConfig::try_from(opts)?;
        Ok(BridgePlatform::new(config))
    });
    registry.register_platform("feishu", |opts| {
        let config = FeishuPlatformConfig::try_from(opts)?;
        Ok(FeishuPlatform::new(config))
    });
    registry.register_platform("lark", |opts| {
        let mut config = FeishuPlatformConfig::try_from(opts)?;
        if config.name == "feishu" {
            config.name = "lark".to_string();
        }
        if config.api_base == "https://open.feishu.cn" {
            config.api_base = "https://open.larksuite.com".to_string();
        }
        Ok(FeishuPlatform::new(config))
    });
    registry.register_platform("dingtalk", |opts| {
        let config = DingTalkPlatformConfig::try_from(opts)?;
        Ok(DingTalkPlatform::new(config))
    });
    registry.register_platform("telegram", |opts| {
        let config = TelegramPlatformConfig::try_from(opts)?;
        Ok(TelegramPlatform::new(config))
    });
    registry.register_platform("slack", |opts| {
        let config = SlackPlatformConfig::try_from(opts)?;
        Ok(SlackPlatform::new(config))
    });
    registry.register_platform("discord", |opts| {
        let config = DiscordPlatformConfig::try_from(opts)?;
        Ok(DiscordPlatform::new(config))
    });
    registry.register_platform("qq", |opts| {
        let config = QqPlatformConfig::try_from(opts)?;
        Ok(QqPlatform::new(config))
    });
    registry.register_platform("line", |opts| {
        let config = LinePlatformConfig::try_from(opts)?;
        Ok(LinePlatform::new(config))
    });
    registry.register_platform("wecom", |opts| {
        let config = WeComPlatformConfig::try_from(opts)?;
        Ok(WeComPlatform::new(config))
    });
    registry.register_platform("max", |opts| {
        let config = max_config_from_options(opts)?;
        Ok(MaxPlatform::new(config))
    });
    registry.register_platform("weixin", |opts| {
        let config = weixin_config_from_options(opts)?;
        Ok(WeixinPlatform::new(config))
    });
    registry.register_platform("qqbot", |opts| {
        let config = qqbot_config_from_options(opts)?;
        Ok(QqBotPlatform::new(config))
    });
    registry.register_platform("weibo", |opts| {
        let config = weibo_config_from_options(opts)?;
        Ok(WeiboPlatform::new(config))
    });
    registry
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("agentlink=info,info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

fn validate_with_registry(config: &Config, registry: &Registry) -> Result<()> {
    for project in &config.projects {
        registry.create_agent(&project.agent.kind, project.agent.options.clone())?;
        for platform in &project.platforms {
            registry.create_platform(&platform.kind, platform_options(platform))?;
        }
    }
    Ok(())
}

fn platform_options(platform: &agentlink::app::config::PlatformConfig) -> toml::value::Table {
    let mut options = platform.options.clone();
    if let Some(id) = &platform.id {
        options
            .entry("name".to_string())
            .or_insert_with(|| TomlValue::String(id.clone()));
    }
    options
}

enum Command {
    Run {
        config_path: String,
        platforms: Vec<String>,
        projects: Vec<String>,
    },
    ValidateConfig {
        path: String,
    },
    Help,
    Version,
    Feishu {
        args: Vec<String>,
    },
    Setup {
        args: Vec<String>,
    },
}

impl Command {
    fn parse(args: Vec<String>) -> Result<Self> {
        if args.is_empty() {
            return Ok(Self::Run {
                config_path: "agentlink.toml".to_string(),
                platforms: Vec::new(),
                projects: Vec::new(),
            });
        }
        if args[0] == "feishu" {
            return Ok(Self::Feishu {
                args: args[1..].to_vec(),
            });
        }
        if args[0] == "setup" {
            return Ok(Self::Setup {
                args: args[1..].to_vec(),
            });
        }

        let mut config_path: Option<String> = None;
        let mut platforms = Vec::new();
        let mut projects = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-h" | "--help" => return Ok(Self::Help),
                "-V" | "--version" => return Ok(Self::Version),
                "-c" | "--config" => {
                    i += 1;
                    let Some(value) = args.get(i) else {
                        anyhow::bail!("--config requires a path");
                    };
                    config_path = Some(value.clone());
                }
                "-p" | "--platform" | "--channel" => {
                    i += 1;
                    let Some(value) = args.get(i) else {
                        anyhow::bail!("{} requires a value", args[i - 1]);
                    };
                    extend_csv(&mut platforms, value);
                }
                "--project" => {
                    i += 1;
                    let Some(value) = args.get(i) else {
                        anyhow::bail!("--project requires a value");
                    };
                    extend_csv(&mut projects, value);
                }
                "--validate-config" => {
                    i += 1;
                    let Some(value) = args.get(i) else {
                        anyhow::bail!("--validate-config requires a path");
                    };
                    return Ok(Self::ValidateConfig {
                        path: value.clone(),
                    });
                }
                value if !value.starts_with('-') && config_path.is_none() => {
                    config_path = Some(value.to_string());
                }
                other => anyhow::bail!("unknown argument `{other}`"),
            }
            i += 1;
        }

        Ok(Self::Run {
            config_path: config_path.unwrap_or_else(|| "agentlink.toml".to_string()),
            platforms,
            projects,
        })
    }
}

fn extend_csv(values: &mut Vec<String>, raw: &str) {
    values.extend(
        raw.split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
    );
}

fn print_help() {
    println!(
        r#"agentlink {VERSION}

USAGE:
  agentlink [--config <path>] [--platform <id|name|type>] [--project <name>]
  agentlink <path>
  agentlink --validate-config <path>
  agentlink setup --platform <name> --config <path> --project <name>
  agentlink feishu setup --config <path> --project <name>
  agentlink feishu setup --config <path> --project <name> --app <id:secret>

OPTIONS:
  -c, --config <path>       Config file path. Default: agentlink.toml
  -p, --platform <selector> Platform/channel to start. Repeat or use comma list.
      --channel <selector>  Alias of --platform.
      --project <name>      Start only this project. Repeat or use comma list.
      --validate-config     Validate config and exit.
  -V, --version             Print version.
  -h, --help                Print help.
"#
    );
}
