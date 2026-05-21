use crate::agents::cli_agent::CliAgent;
use crate::agents::codex::CodexAgent;
use crate::app::config::{Config, PlatformConfig};
use crate::channels::bridge::{BridgePlatform, BridgePlatformConfig};
use crate::channels::dingtalk::{DingTalkPlatform, DingTalkPlatformConfig};
use crate::channels::discord::{DiscordPlatform, DiscordPlatformConfig};
use crate::channels::feishu::{FeishuPlatform, FeishuPlatformConfig};
use crate::channels::http_channel::{HttpPlatform, HttpPlatformConfig};
use crate::channels::line::{LinePlatform, LinePlatformConfig};
use crate::channels::max::{max_config_from_options, MaxPlatform};
use crate::channels::qq::{QqPlatform, QqPlatformConfig};
use crate::channels::qqbot::{qqbot_config_from_options, QqBotPlatform};
use crate::channels::slack::{SlackPlatform, SlackPlatformConfig};
use crate::channels::telegram::{TelegramPlatform, TelegramPlatformConfig};
use crate::channels::wecom::{WeComPlatform, WeComPlatformConfig};
use crate::channels::weibo::{weibo_config_from_options, WeiboPlatform};
use crate::channels::weixin::{weixin_config_from_options, WeixinPlatform};
use crate::testing::mock::{MockAgent, MockPlatform};
use crate::{Engine, Registry, SessionStore};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use toml::Value as TomlValue;

pub fn default_registry() -> Registry {
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

pub fn validate_with_registry(config: &Config, registry: &Registry) -> Result<()> {
    for project in &config.projects {
        registry.create_agent(&project.agent.kind, project.agent.options.clone())?;
        for platform in &project.platforms {
            registry.create_platform(&platform.kind, platform_options(platform))?;
        }
    }
    Ok(())
}

pub async fn run(
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
        for platform in &selected {
            platforms.push(registry.create_platform(&platform.kind, platform_options(platform))?);
        }
        let store_path = PathBuf::from(&config.data_dir).join(format!("{}.sqlite3", project.name));
        let store = SessionStore::open(store_path)?;
        let engine = Engine::new(project.name.clone(), agent, platforms, store);
        engine.start().await?;
        let platform_names = selected
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

fn platform_options(platform: &PlatformConfig) -> toml::value::Table {
    let mut options = platform.options.clone();
    if let Some(id) = &platform.id {
        options
            .entry("name".to_string())
            .or_insert_with(|| TomlValue::String(id.clone()));
    }
    options
}
