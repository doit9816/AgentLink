mod args;
mod config;
mod registration;

use anyhow::{anyhow, Result};
use args::{parse_args, print_feishu_help, FeishuSetupArgs, SetupMode};
use config::write_feishu_config;
use registration::{registration_flow, validate_credentials, RegistrationResult};

pub async fn run_feishu_command(args: Vec<String>) -> Result<()> {
    let Some(subcommand) = args.first().map(String::as_str) else {
        print_feishu_help();
        return Ok(());
    };
    match subcommand {
        "setup" => run_feishu_setup(parse_args(&args[1..], SetupMode::Auto)?).await,
        "new" | "create" => run_feishu_setup(parse_args(&args[1..], SetupMode::New)?).await,
        "bind" | "link" => run_feishu_setup(parse_args(&args[1..], SetupMode::Bind)?).await,
        "help" | "--help" | "-h" => {
            print_feishu_help();
            Ok(())
        }
        other => Err(anyhow!("unknown feishu subcommand `{other}`")),
    }
}

async fn run_feishu_setup(mut args: FeishuSetupArgs) -> Result<()> {
    let effective_mode = match args.mode {
        SetupMode::Auto if args.app_id.is_some() || args.app_secret.is_some() => SetupMode::Bind,
        SetupMode::Auto => SetupMode::New,
        mode => mode,
    };

    let result = match effective_mode {
        SetupMode::Bind => {
            let app_id = args
                .app_id
                .clone()
                .ok_or_else(|| anyhow!("bind requires --app or --app-id"))?;
            let app_secret = args
                .app_secret
                .clone()
                .ok_or_else(|| anyhow!("bind requires --app or --app-secret"))?;
            let detected =
                validate_credentials(&app_id, &app_secret, args.platform_type.as_deref()).await?;
            println!("Credentials verified for app_id {app_id}.");
            RegistrationResult {
                app_id,
                app_secret,
                owner_open_id: None,
                platform_type: args.platform_type.take().unwrap_or(detected),
            }
        }
        SetupMode::New => registration_flow(args.timeout_seconds, args.debug).await?,
        SetupMode::Auto => unreachable!(),
    };

    let platform_type = args
        .platform_type
        .clone()
        .unwrap_or_else(|| result.platform_type.clone());
    write_feishu_config(&args, &result, &platform_type)?;

    println!("Feishu/Lark configured.");
    println!("  config:   {}", args.config_path);
    println!("  project:  {}", args.project);
    println!("  platform: {platform_type}");
    println!("  app_id:   {}", result.app_id);
    println!("  mode:     websocket");
    if let Some(owner) = result.owner_open_id {
        println!("  owner_open_id: {owner}");
    }
    println!();
    println!("Next:");
    println!("  agentlink --config {}", args.config_path);
    println!("  在飞书/Lark 开放平台启用机器人，并用长连接方式订阅 im.message.receive_v1。");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::args::{parse_args, FeishuSetupArgs, SetupMode};
    use super::config::write_feishu_config;
    use super::registration::RegistrationResult;

    #[test]
    fn parse_setup_app_credentials() {
        let args = parse_args(
            &[
                "--config".to_string(),
                "tmp.toml".to_string(),
                "--project".to_string(),
                "p1".to_string(),
                "--app".to_string(),
                "cli_xxx:sec_xxx".to_string(),
                "--platform-type".to_string(),
                "feishu".to_string(),
                "--dry-run".to_string(),
            ],
            SetupMode::Auto,
        )
        .unwrap();
        assert_eq!(args.config_path, "tmp.toml");
        assert_eq!(args.project, "p1");
        assert_eq!(args.app_id.as_deref(), Some("cli_xxx"));
        assert_eq!(args.app_secret.as_deref(), Some("sec_xxx"));
        assert_eq!(args.platform_type.as_deref(), Some("feishu"));
        assert!(args.dry_run);
    }

    #[test]
    fn write_config_creates_project_and_feishu_platform() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agentlink.toml");
        let args = FeishuSetupArgs {
            mode: SetupMode::Bind,
            config_path: path.to_string_lossy().to_string(),
            project: "demo".to_string(),
            platform_type: Some("feishu".to_string()),
            app_id: None,
            app_secret: None,
            timeout_seconds: 600,
            work_dir: "D:/work".to_string(),
            listen: "0.0.0.0:18200".to_string(),
            callback_path: "/feishu/webhook".to_string(),
            dry_run: true,
            debug: false,
        };
        let result = RegistrationResult {
            app_id: "cli_xxx".to_string(),
            app_secret: "sec_xxx".to_string(),
            owner_open_id: Some("ou_xxx".to_string()),
            platform_type: "feishu".to_string(),
        };
        write_feishu_config(&args, &result, "feishu").unwrap();
        let raw = std::fs::read_to_string(path).unwrap();
        assert!(raw.contains("type = \"codex\""));
        assert!(raw.contains("default_platforms = [\"feishu\"]"));
        assert!(raw.contains("id = \"feishu\""));
        assert!(raw.contains("type = \"feishu\""));
        assert!(raw.contains("app_id = \"cli_xxx\""));
        assert!(raw.contains("dry_run = true"));
    }
}
