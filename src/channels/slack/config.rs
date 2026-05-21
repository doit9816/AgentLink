use anyhow::{anyhow, Result};

#[derive(Debug, Clone)]
pub struct SlackPlatformConfig {
    pub name: String,
    pub bot_token: String,
    pub app_token: String,
    pub api_base: String,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

impl Default for SlackPlatformConfig {
    fn default() -> Self {
        Self {
            name: "slack".to_string(),
            bot_token: String::new(),
            app_token: String::new(),
            api_base: "https://slack.com/api".to_string(),
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

impl TryFrom<toml::value::Table> for SlackPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let dry_run = bool_option(&opts, "dry_run").unwrap_or(false);
        let bot_token = string_option(&opts, "bot_token").unwrap_or_default();
        let app_token = string_option(&opts, "app_token").unwrap_or_default();
        if !dry_run && (bot_token.trim().is_empty() || app_token.trim().is_empty()) {
            return Err(anyhow!(
                "slack requires bot_token and app_token unless dry_run = true"
            ));
        }
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "slack".to_string()),
            bot_token,
            app_token,
            api_base: string_option(&opts, "api_base")
                .unwrap_or_else(|| "https://slack.com/api".to_string()),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run,
        })
    }
}
