use anyhow::{anyhow, Result};
use std::net::SocketAddr;

#[derive(Debug, Clone, Default)]
pub struct HttpPlatformConfig {
    pub name: String,
    pub listen: String,
    pub bearer_token: Option<String>,
    pub outbound_url: Option<String>,
}

pub(crate) fn string_option(opts: &toml::value::Table, key: &str) -> Option<String> {
    opts.get(key)
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
}

impl TryFrom<toml::value::Table> for HttpPlatformConfig {
    type Error = anyhow::Error;

    fn try_from(opts: toml::value::Table) -> Result<Self> {
        let listen =
            string_option(&opts, "listen").unwrap_or_else(|| "127.0.0.1:18080".to_string());
        if listen.parse::<SocketAddr>().is_err() {
            return Err(anyhow!(
                "http platform listen must be a socket address, got `{listen}`"
            ));
        }
        Ok(Self {
            name: string_option(&opts, "name").unwrap_or_else(|| "http".to_string()),
            listen,
            bearer_token: string_option(&opts, "bearer_token"),
            outbound_url: string_option(&opts, "outbound_url"),
        })
    }
}
