use anyhow::{anyhow, Result};

#[derive(Debug, Clone)]
pub struct DiscordPlatformConfig {
    pub name: String,
    pub token: String,
    pub api_base: String,
    pub gateway_url: Option<String>,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

impl Default for DiscordPlatformConfig {
    fn default() -> Self {
        Self {
            name: "discord".to_string(),
            token: String::new(),
            api_base: "https://discord.com/api/v10".to_string(),
            gateway_url: None,
            share_session_in_channel: false,
            dry_run: false,
        }
    }
}

fn string_option(opts: &toml::value::Table, key: &str) -> Option<String> {
    opts.get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn bool_option(opts: &toml::value::Table, key: &str) -> Option<bool> {
    opts.get(key).and_then(|value| value.as_bool())
}

impl TryFrom<toml::value::Table> for DiscordPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let dry_run = bool_option(&opts, "dry_run").unwrap_or(false);
        let token = string_option(&opts, "token").unwrap_or_default();
        if token.trim().is_empty() && !dry_run {
            return Err(anyhow!("discord requires token unless dry_run = true"));
        }
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "discord".to_string()),
            token,
            api_base: string_option(&opts, "api_base")
                .unwrap_or_else(|| "https://discord.com/api/v10".to_string()),
            gateway_url: string_option(&opts, "gateway_url"),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run,
        })
    }
}
