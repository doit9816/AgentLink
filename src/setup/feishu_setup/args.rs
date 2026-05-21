use anyhow::{anyhow, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SetupMode {
    Auto,
    New,
    Bind,
}

#[derive(Debug, Clone)]
pub(super) struct FeishuSetupArgs {
    pub(super) mode: SetupMode,
    pub(super) config_path: String,
    pub(super) project: String,
    pub(super) platform_type: Option<String>,
    pub(super) app_id: Option<String>,
    pub(super) app_secret: Option<String>,
    pub(super) timeout_seconds: u64,
    pub(super) work_dir: String,
    pub(super) listen: String,
    pub(super) callback_path: String,
    pub(super) dry_run: bool,
    pub(super) debug: bool,
}

pub(super) fn parse_args(values: &[String], mode: SetupMode) -> Result<FeishuSetupArgs> {
    let mut args = FeishuSetupArgs {
        mode,
        config_path: "agentlink.toml".to_string(),
        project: "demo".to_string(),
        platform_type: None,
        app_id: None,
        app_secret: None,
        timeout_seconds: 600,
        work_dir: std::env::current_dir()?.to_string_lossy().to_string(),
        listen: "0.0.0.0:18200".to_string(),
        callback_path: "/feishu/webhook".to_string(),
        dry_run: false,
        debug: false,
    };
    let mut i = 0;
    while i < values.len() {
        match values[i].as_str() {
            "--config" | "-c" => {
                i += 1;
                args.config_path = required(values, i, "--config")?;
            }
            "--project" => {
                i += 1;
                args.project = required(values, i, "--project")?;
            }
            "--platform-type" => {
                i += 1;
                let value = required(values, i, "--platform-type")?;
                if value != "feishu" && value != "lark" {
                    return Err(anyhow!("--platform-type must be feishu or lark"));
                }
                args.platform_type = Some(value);
            }
            "--app" => {
                i += 1;
                let value = required(values, i, "--app")?;
                let Some((id, secret)) = value.split_once(':') else {
                    return Err(anyhow!("--app must be app_id:app_secret"));
                };
                args.app_id = Some(id.to_string());
                args.app_secret = Some(secret.to_string());
            }
            "--app-id" => {
                i += 1;
                args.app_id = Some(required(values, i, "--app-id")?);
            }
            "--app-secret" => {
                i += 1;
                args.app_secret = Some(required(values, i, "--app-secret")?);
            }
            "--timeout" => {
                i += 1;
                args.timeout_seconds = required(values, i, "--timeout")?.parse()?;
            }
            "--work-dir" => {
                i += 1;
                args.work_dir = required(values, i, "--work-dir")?;
            }
            "--listen" => {
                i += 1;
                args.listen = required(values, i, "--listen")?;
            }
            "--callback-path" => {
                i += 1;
                args.callback_path = required(values, i, "--callback-path")?;
            }
            "--dry-run" => args.dry_run = true,
            "--debug" => args.debug = true,
            "--help" | "-h" => {
                print_feishu_help();
                std::process::exit(0);
            }
            other => return Err(anyhow!("unknown feishu setup argument `{other}`")),
        }
        i += 1;
    }
    Ok(args)
}

fn required(values: &[String], index: usize, flag: &str) -> Result<String> {
    values
        .get(index)
        .cloned()
        .ok_or_else(|| anyhow!("{flag} requires a value"))
}

pub(super) fn print_feishu_help() {
    println!(
        r#"agentlink feishu <setup|new|bind> [options]

Examples:
  agentlink feishu setup --config agentlink.toml --project demo
  agentlink feishu setup --config agentlink.toml --project demo --app cli_xxx:sec_xxx
  agentlink feishu bind --app-id cli_xxx --app-secret sec_xxx

Options:
  --config <path>             Config file to create/update. Default: agentlink.toml
  --project <name>            Project name. Default: demo
  --platform-type <type>      feishu or lark
  --app <id:secret>           Existing app credentials; setup becomes bind mode
  --app-id <id>               Existing app_id
  --app-secret <secret>       Existing app_secret
  --timeout <seconds>         QR onboarding timeout. Default: 600
  --work-dir <path>           Project work_dir when creating config
  --listen <addr>             Webhook fallback listen address. Default: 0.0.0.0:18200
  --callback-path <path>      Webhook fallback path. Default: /feishu/webhook
  --dry-run                   Write dry_run = true for local testing
  --debug                     Print onboarding HTTP responses
"#
    );
}
