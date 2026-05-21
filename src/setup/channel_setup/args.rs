use anyhow::{anyhow, Result};

#[derive(Debug, Clone)]
pub struct SetupArgs {
    pub platform: String,
    pub config_path: String,
    pub project: String,
    pub work_dir: String,
    pub ws_url: String,
    pub token: Option<String>,
    pub passthrough: Vec<String>,
}

pub fn parse_args(values: &[String]) -> Result<SetupArgs> {
    let mut args = SetupArgs {
        platform: String::new(),
        config_path: "agentlink.toml".to_string(),
        project: "demo".to_string(),
        work_dir: std::env::current_dir()?.to_string_lossy().to_string(),
        ws_url: "ws://127.0.0.1:3001".to_string(),
        token: None,
        passthrough: Vec::new(),
    };
    let mut i = 0;
    while i < values.len() {
        match values[i].as_str() {
            "--platform" | "--channel" | "-p" => {
                i += 1;
                args.platform = required(values, i, "--platform")?;
            }
            "--config" | "-c" => {
                i += 1;
                args.config_path = required(values, i, "--config")?;
            }
            "--project" => {
                i += 1;
                args.project = required(values, i, "--project")?;
            }
            "--work-dir" => {
                i += 1;
                args.work_dir = required(values, i, "--work-dir")?;
            }
            "--ws-url" => {
                i += 1;
                args.ws_url = required(values, i, "--ws-url")?;
            }
            "--token" => {
                i += 1;
                args.token = Some(required(values, i, "--token")?);
            }
            flag @ ("--app" | "--app-id" | "--app-secret" | "--timeout" | "--debug"
            | "--dry-run") => {
                args.passthrough.push(flag.to_string());
                if flag != "--debug" && flag != "--dry-run" {
                    i += 1;
                    args.passthrough.push(required(values, i, flag)?);
                }
            }
            value if args.platform.is_empty() && !value.starts_with('-') => {
                args.platform = value.to_string();
            }
            other => return Err(anyhow!("unknown setup argument `{other}`")),
        }
        i += 1;
    }
    if args.platform.trim().is_empty() {
        return Err(anyhow!("setup requires --platform <name>"));
    }
    args.platform = args.platform.trim().to_ascii_lowercase();
    Ok(args)
}

fn required(values: &[String], index: usize, flag: &str) -> Result<String> {
    values
        .get(index)
        .cloned()
        .ok_or_else(|| anyhow!("{flag} requires a value"))
}
