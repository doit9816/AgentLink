use anyhow::Result;

#[derive(Debug, Clone)]
pub struct QqPlatformConfig {
    pub name: String,
    pub ws_url: String,
    pub token: Option<String>,
    pub share_session_in_channel: bool,
    pub dry_run: bool,
}

impl Default for QqPlatformConfig {
    fn default() -> Self {
        Self {
            name: "qq".to_string(),
            ws_url: "ws://127.0.0.1:3001".to_string(),
            token: None,
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

impl TryFrom<toml::value::Table> for QqPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "qq".to_string()),
            ws_url: string_option(&opts, "ws_url")
                .unwrap_or_else(|| "ws://127.0.0.1:3001".to_string()),
            token: string_option(&opts, "token"),
            share_session_in_channel: bool_option(&opts, "share_session_in_channel")
                .unwrap_or(false),
            dry_run: bool_option(&opts, "dry_run").unwrap_or(false),
        })
    }
}
