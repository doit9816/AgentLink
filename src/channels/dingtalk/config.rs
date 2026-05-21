use anyhow::{anyhow, Result};
use std::net::SocketAddr;

#[derive(Debug, Clone, Default)]
pub struct DingTalkPlatformConfig {
    pub name: String,
    pub client_id: String,
    pub client_secret: String,
    pub robot_code: String,
    pub listen: String,
    pub callback_path: String,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

fn string_option(opts: &toml::value::Table, key: &str) -> Option<String> {
    opts.get(key)
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
}

fn bool_option(opts: &toml::value::Table, key: &str) -> Option<bool> {
    opts.get(key).and_then(|v| v.as_bool())
}

impl TryFrom<toml::value::Table> for DingTalkPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let client_id = string_option(&opts, "client_id").unwrap_or_default();
        let client_secret = string_option(&opts, "client_secret").unwrap_or_default();
        let dry_run = bool_option(&opts, "dry_run").unwrap_or(false);
        if !dry_run && (client_id.is_empty() || client_secret.is_empty()) {
            return Err(anyhow!(
                "dingtalk requires client_id and client_secret unless dry_run = true"
            ));
        }
        let listen =
            string_option(&opts, "listen").unwrap_or_else(|| "127.0.0.1:18300".to_string());
        if listen.parse::<SocketAddr>().is_err() {
            return Err(anyhow!(
                "dingtalk listen must be a socket address, got `{listen}`"
            ));
        }
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "dingtalk".to_string()),
            client_id: client_id.clone(),
            client_secret,
            robot_code: string_option(&opts, "robot_code").unwrap_or(client_id),
            listen,
            callback_path: string_option(&opts, "callback_path")
                .unwrap_or_else(|| "/dingtalk/webhook".to_string()),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run,
        })
    }
}
