use anyhow::{anyhow, Result};

#[derive(Debug, Clone)]
pub struct TelegramPlatformConfig {
    pub name: String,
    pub token: String,
    pub api_base: String,
    pub poll_timeout_secs: u64,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

impl Default for TelegramPlatformConfig {
    fn default() -> Self {
        Self {
            name: "telegram".to_string(),
            token: String::new(),
            api_base: "https://api.telegram.org".to_string(),
            poll_timeout_secs: 30,
            share_session_in_channel: false,
            dry_run: false,
        }
    }
}

fn string_option(opts: &toml::value::Table, key: &str) -> Option<String> {
    opts.get(key)
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
}

fn bool_option(opts: &toml::value::Table, key: &str) -> Option<bool> {
    opts.get(key).and_then(|v| v.as_bool())
}

fn u64_option(opts: &toml::value::Table, key: &str) -> Option<u64> {
    opts.get(key).and_then(|v| v.as_integer()).map(|v| v as u64)
}

impl TryFrom<toml::value::Table> for TelegramPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let dry_run = bool_option(&opts, "dry_run").unwrap_or(false);
        let token = string_option(&opts, "token").unwrap_or_default();
        if token.trim().is_empty() && !dry_run {
            return Err(anyhow!("telegram requires token unless dry_run = true"));
        }
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "telegram".to_string()),
            token,
            api_base: string_option(&opts, "api_base")
                .unwrap_or_else(|| "https://api.telegram.org".to_string()),
            poll_timeout_secs: u64_option(&opts, "poll_timeout_secs").unwrap_or(30),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run,
        })
    }
}
