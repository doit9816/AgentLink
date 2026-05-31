use super::state::AcpAgentState;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct AcpAgent {
    pub(super) name: String,
    pub(super) command: String,
    pub(super) args: Vec<String>,
    pub(super) work_dir: String,
    pub(super) env: BTreeMap<String, String>,
    pub(super) auth_method: Option<String>,
    pub(super) display_name: String,
    pub(super) mode: Option<String>,
    pub(super) state: Arc<AcpAgentState>,
}

impl AcpAgent {
    pub fn new_from_options(opts: toml::value::Table) -> anyhow::Result<Self> {
        let command = table_string(&opts, "command")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "acp agent option \"command\" is required (path or name of the ACP agent binary)"
                )
            })?;
        let display_name = table_string(&opts, "display_name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "ACP".to_string());
        Ok(Self {
            name: table_string(&opts, "name").unwrap_or_else(|| "acp".to_string()),
            command,
            args: table_string_vec(&opts, "args").unwrap_or_default(),
            work_dir: table_string(&opts, "work_dir").unwrap_or_else(|| ".".to_string()),
            env: table_env(&opts),
            auth_method: table_string(&opts, "auth_method").filter(|s| !s.trim().is_empty()),
            display_name,
            mode: table_string(&opts, "mode").filter(|s| !s.trim().is_empty()),
            state: AcpAgentState::new(),
        })
    }
}

fn table_string(opts: &toml::value::Table, key: &str) -> Option<String> {
    opts.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn table_string_vec(opts: &toml::value::Table, key: &str) -> Option<Vec<String>> {
    opts.get(key).and_then(|value| {
        value.as_array().map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
    })
}

fn table_env(opts: &toml::value::Table) -> BTreeMap<String, String> {
    opts.get("env")
        .and_then(|value| value.as_table())
        .map(|table| {
            table
                .iter()
                .filter_map(|(key, value)| value.as_str().map(|v| (key.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default()
}
