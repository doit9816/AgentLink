use super::preset::CliPreset;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct CliAgent {
    pub(super) name: String,
    pub(super) command: String,
    pub(super) args: Vec<String>,
    pub(super) work_dir: String,
    pub(super) model: Option<String>,
    pub(super) mode: Option<String>,
    pub(super) env: BTreeMap<String, String>,
    pub(super) prompt_stdin: bool,
    pub(super) append_prompt: bool,
    pub(super) timeout_secs: u64,
}

impl CliAgent {
    pub fn new_from_options(name: &str, opts: toml::value::Table) -> Self {
        let preset = CliPreset::for_agent(name);
        Self {
            name: table_string(&opts, "name").unwrap_or_else(|| name.to_string()),
            command: table_string(&opts, "command")
                .or_else(|| table_string(&opts, "cmd"))
                .or_else(|| table_string(&opts, "cli_path"))
                .unwrap_or_else(|| preset.command.to_string()),
            args: table_string_vec(&opts, "args").unwrap_or_else(|| preset.args()),
            work_dir: table_string(&opts, "work_dir").unwrap_or_else(|| ".".to_string()),
            model: table_string(&opts, "model"),
            mode: table_string(&opts, "mode"),
            env: table_env(&opts),
            prompt_stdin: table_bool(&opts, "prompt_stdin").unwrap_or(preset.prompt_stdin),
            append_prompt: table_bool(&opts, "append_prompt").unwrap_or(preset.append_prompt),
            timeout_secs: table_u64(&opts, "timeout_secs")
                .or_else(|| table_u64(&opts, "timeout_seconds"))
                .or_else(|| table_u64(&opts, "timeout_mins").map(|mins| mins * 60))
                .unwrap_or(1800),
        }
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

fn table_bool(opts: &toml::value::Table, key: &str) -> Option<bool> {
    opts.get(key).and_then(|v| v.as_bool())
}

fn table_u64(opts: &toml::value::Table, key: &str) -> Option<u64> {
    opts.get(key).and_then(|v| v.as_integer()).map(|v| v as u64)
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
