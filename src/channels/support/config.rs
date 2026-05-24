use super::*;

#[derive(Debug, Clone)]
pub struct LinePlatformConfig {
    pub name: String,
    pub listen: String,
    pub callback_path: String,
    pub secret: Option<String>,
    pub token: Option<String>,
    pub corp_id: Option<String>,
    pub corp_secret: Option<String>,
    pub agent_id: Option<String>,
    pub callback_token: Option<String>,
    pub callback_aes_key: Option<String>,
    pub api_base: String,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

impl Default for LinePlatformConfig {
    fn default() -> Self {
        Self {
            name: "line".to_string(),
            listen: "127.0.0.1:18400".to_string(),
            callback_path: "/line/webhook".to_string(),
            secret: None,
            token: None,
            corp_id: None,
            corp_secret: None,
            agent_id: None,
            callback_token: None,
            callback_aes_key: None,
            api_base: String::new(),
            share_session_in_channel: false,
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WeComPlatformConfig {
    pub name: String,
    pub connection_mode: String,
    pub listen: String,
    pub callback_path: String,
    pub bot_id: Option<String>,
    pub bot_secret: Option<String>,
    pub websocket_url: String,
    pub secret: Option<String>,
    pub token: Option<String>,
    pub corp_id: Option<String>,
    pub corp_secret: Option<String>,
    pub agent_id: Option<String>,
    pub callback_token: Option<String>,
    pub callback_aes_key: Option<String>,
    pub api_base: String,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

impl WeComPlatformConfig {
    pub fn uses_websocket(&self) -> bool {
        self.connection_mode
            .trim()
            .eq_ignore_ascii_case("websocket")
    }
}

impl Default for WeComPlatformConfig {
    fn default() -> Self {
        Self {
            name: "wecom".to_string(),
            connection_mode: "websocket".to_string(),
            listen: "127.0.0.1:18500".to_string(),
            callback_path: "/wecom/callback".to_string(),
            bot_id: None,
            bot_secret: None,
            websocket_url: "wss://openws.work.weixin.qq.com".to_string(),
            secret: None,
            token: None,
            corp_id: None,
            corp_secret: None,
            agent_id: None,
            callback_token: None,
            callback_aes_key: None,
            api_base: String::new(),
            share_session_in_channel: false,
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PollPlatformConfig {
    pub name: String,
    pub app_id: Option<String>,
    pub app_secret: Option<String>,
    pub token_endpoint: Option<String>,
    pub token: String,
    pub api_base: String,
    pub ws_endpoint: Option<String>,
    pub poll_timeout_secs: u64,
    pub webhook_url: Option<String>,
    pub webhook_listen: String,
    pub webhook_path: String,
    pub webhook_secret: Option<String>,
    pub route_tag: Option<String>,
    pub account_id: Option<String>,
    pub allow_from: Option<String>,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

impl Default for PollPlatformConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            app_id: None,
            app_secret: None,
            token_endpoint: None,
            token: String::new(),
            api_base: String::new(),
            ws_endpoint: None,
            poll_timeout_secs: 30,
            webhook_url: None,
            webhook_listen: "127.0.0.1:18600".to_string(),
            webhook_path: "/max/webhook".to_string(),
            webhook_secret: None,
            route_tag: None,
            account_id: None,
            allow_from: None,
            share_session_in_channel: false,
            dry_run: false,
        }
    }
}

pub(crate) fn string_option(opts: &toml::value::Table, key: &str) -> Option<String> {
    opts.get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

pub(crate) fn bool_option(opts: &toml::value::Table, key: &str) -> Option<bool> {
    opts.get(key).and_then(|value| value.as_bool())
}

pub(crate) fn u64_option(opts: &toml::value::Table, key: &str) -> Option<u64> {
    opts.get(key)
        .and_then(|value| value.as_integer())
        .map(|value| value as u64)
}

macro_rules! webhook_config_try_from {
    ($config:ident, $default_name:literal, $default_base:literal) => {
        impl TryFrom<toml::value::Table> for $config {
            type Error = anyhow::Error;

            fn try_from(opts: toml::value::Table) -> Result<Self> {
                Ok(Self {
                    name: string_option(&opts, "name").unwrap_or_else(|| $default_name.to_string()),
                    listen: string_option(&opts, "listen")
                        .unwrap_or_else(|| Self::default().listen),
                    callback_path: string_option(&opts, "callback_path")
                        .unwrap_or_else(|| Self::default().callback_path),
                    secret: string_option(&opts, "secret")
                        .or_else(|| string_option(&opts, "channel_secret"))
                        .or_else(|| string_option(&opts, "corp_id")),
                    token: string_option(&opts, "token")
                        .or_else(|| string_option(&opts, "channel_token"))
                        .or_else(|| string_option(&opts, "corp_secret"))
                        .or_else(|| string_option(&opts, "agent_id")),
                    corp_id: string_option(&opts, "corp_id"),
                    corp_secret: string_option(&opts, "corp_secret"),
                    agent_id: string_option(&opts, "agent_id"),
                    callback_token: string_option(&opts, "callback_token"),
                    callback_aes_key: string_option(&opts, "callback_aes_key"),
                    api_base: string_option(&opts, "api_base")
                        .or_else(|| string_option(&opts, "api_base_url"))
                        .unwrap_or_else(|| $default_base.to_string()),
                    share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                        .unwrap_or(false),
                    dry_run: bool_option(&opts, "dry_run").unwrap_or(false),
                })
            }
        }
    };
}

webhook_config_try_from!(LinePlatformConfig, "line", "https://api.line.me");

impl TryFrom<toml::value::Table> for WeComPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let defaults = Self::default();
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "wecom".to_string()),
            connection_mode: string_option(&opts, "connection_mode")
                .unwrap_or_else(|| defaults.connection_mode.clone()),
            listen: string_option(&opts, "listen").unwrap_or_else(|| defaults.listen.clone()),
            callback_path: string_option(&opts, "callback_path")
                .unwrap_or_else(|| defaults.callback_path.clone()),
            bot_id: string_option(&opts, "bot_id"),
            bot_secret: string_option(&opts, "bot_secret"),
            websocket_url: string_option(&opts, "websocket_url")
                .unwrap_or_else(|| defaults.websocket_url.clone()),
            secret: string_option(&opts, "secret")
                .or_else(|| string_option(&opts, "channel_secret"))
                .or_else(|| string_option(&opts, "corp_id")),
            token: string_option(&opts, "token")
                .or_else(|| string_option(&opts, "channel_token"))
                .or_else(|| string_option(&opts, "corp_secret"))
                .or_else(|| string_option(&opts, "agent_id")),
            corp_id: string_option(&opts, "corp_id"),
            corp_secret: string_option(&opts, "corp_secret"),
            agent_id: string_option(&opts, "agent_id"),
            callback_token: string_option(&opts, "callback_token"),
            callback_aes_key: string_option(&opts, "callback_aes_key"),
            api_base: string_option(&opts, "api_base")
                .or_else(|| string_option(&opts, "api_base_url"))
                .unwrap_or_else(|| "https://qyapi.weixin.qq.com".to_string()),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run: bool_option(&opts, "dry_run").unwrap_or(false),
        })
    }
}

impl TryFrom<toml::value::Table> for PollPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "poll".to_string()),
            token: string_option(&opts, "token")
                .or_else(|| string_option(&opts, "app_token"))
                .unwrap_or_default(),
            app_id: string_option(&opts, "app_id"),
            app_secret: string_option(&opts, "app_secret"),
            token_endpoint: string_option(&opts, "token_endpoint"),
            api_base: string_option(&opts, "api_base")
                .or_else(|| string_option(&opts, "base_url"))
                .unwrap_or_default(),
            ws_endpoint: string_option(&opts, "ws_endpoint")
                .or_else(|| string_option(&opts, "gateway_url")),
            poll_timeout_secs: u64_option(&opts, "poll_timeout_secs").unwrap_or(30),
            webhook_url: string_option(&opts, "webhook_url"),
            webhook_listen: string_option(&opts, "webhook_listen")
                .unwrap_or_else(|| "127.0.0.1:18600".to_string()),
            webhook_path: string_option(&opts, "webhook_path")
                .unwrap_or_else(|| "/max/webhook".to_string()),
            webhook_secret: string_option(&opts, "webhook_secret"),
            route_tag: string_option(&opts, "route_tag"),
            account_id: string_option(&opts, "account_id"),
            allow_from: string_option(&opts, "allow_from"),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run: bool_option(&opts, "dry_run").unwrap_or(false),
        })
    }
}

pub fn max_config_from_options(opts: toml::value::Table) -> Result<PollPlatformConfig> {
    let mut cfg = PollPlatformConfig::try_from(opts)?;
    if cfg.name == "poll" {
        cfg.name = "max".to_string();
    }
    if cfg.api_base.is_empty() {
        cfg.api_base = "https://platform-api.max.ru".to_string();
    }
    Ok(cfg)
}

pub fn weixin_config_from_options(opts: toml::value::Table) -> Result<PollPlatformConfig> {
    let mut cfg = PollPlatformConfig::try_from(opts)?;
    if cfg.name == "poll" {
        cfg.name = "weixin".to_string();
    }
    if cfg.api_base.is_empty() {
        cfg.api_base = "https://ilinkai.weixin.qq.com".to_string();
    }
    Ok(cfg)
}

pub fn qqbot_config_from_options(opts: toml::value::Table) -> Result<PollPlatformConfig> {
    let mut cfg = PollPlatformConfig::try_from(opts.clone())?;
    if cfg.name == "poll" {
        cfg.name = "qqbot".to_string();
    }
    if cfg.api_base.is_empty() {
        cfg.api_base = "https://api.sgroup.qq.com".to_string();
    }
    if cfg.ws_endpoint.is_none() {
        cfg.ws_endpoint = string_option(&opts, "gateway_url");
    }
    Ok(cfg)
}

pub fn weibo_config_from_options(opts: toml::value::Table) -> Result<PollPlatformConfig> {
    let mut cfg = PollPlatformConfig::try_from(opts.clone())?;
    if cfg.name == "poll" {
        cfg.name = "weibo".to_string();
    }
    if cfg.api_base.is_empty() {
        cfg.api_base = "https://open-im.api.weibo.com".to_string();
    }
    if cfg.ws_endpoint.is_none() {
        cfg.ws_endpoint = string_option(&opts, "ws_endpoint");
    }
    Ok(cfg)
}
