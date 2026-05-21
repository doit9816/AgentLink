use anyhow::{anyhow, Result};
use std::net::SocketAddr;

#[derive(Debug, Clone, Default)]
pub struct FeishuPlatformConfig {
    pub name: String,
    pub app_id: String,
    pub app_secret: String,
    pub api_base: String,
    pub connection_mode: String,
    pub listen: String,
    pub callback_path: String,
    pub verification_token: Option<String>,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

fn string_option(opts: &toml::value::Table, key: &str) -> Option<String> {
    opts.get(key)
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

fn bool_option(opts: &toml::value::Table, key: &str) -> Option<bool> {
    opts.get(key).and_then(|value| value.as_bool())
}

impl TryFrom<toml::value::Table> for FeishuPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let app_id = string_option(&opts, "app_id").unwrap_or_default();
        let app_secret = string_option(&opts, "app_secret").unwrap_or_default();
        let dry_run = bool_option(&opts, "dry_run").unwrap_or(false);
        if !dry_run && (app_id.is_empty() || app_secret.is_empty()) {
            return Err(anyhow!(
                "feishu requires app_id and app_secret unless dry_run = true"
            ));
        }
        let connection_mode = string_option(&opts, "connection_mode")
            .or_else(|| string_option(&opts, "mode"))
            .unwrap_or_else(|| "websocket".to_string())
            .trim()
            .to_ascii_lowercase();
        if connection_mode != "websocket" && connection_mode != "webhook" {
            return Err(anyhow!(
                "feishu connection_mode must be websocket or webhook, got `{connection_mode}`"
            ));
        }
        let listen =
            string_option(&opts, "listen").unwrap_or_else(|| "127.0.0.1:18200".to_string());
        if listen.parse::<SocketAddr>().is_err() {
            return Err(anyhow!(
                "feishu listen must be a socket address, got `{listen}`"
            ));
        }
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "feishu".to_string()),
            app_id,
            app_secret,
            api_base: string_option(&opts, "api_base")
                .unwrap_or_else(|| "https://open.feishu.cn".to_string()),
            connection_mode,
            listen,
            callback_path: string_option(&opts, "callback_path")
                .unwrap_or_else(|| "/feishu/webhook".to_string()),
            verification_token: string_option(&opts, "verification_token"),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run,
        })
    }
}
