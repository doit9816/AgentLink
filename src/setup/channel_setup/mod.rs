mod args;
mod config;
mod help;

use crate::setup::feishu_setup::run_feishu_command;
use anyhow::{anyhow, Result};
use args::{parse_args, SetupArgs};
use config::update_platform_config;
use help::{print_setup_help, print_support_matrix, unsupported_reason};

pub async fn run_setup_command(args: Vec<String>) -> Result<()> {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_setup_help();
        return Ok(());
    }
    let args = parse_args(&args)?;
    match args.platform.as_str() {
        "feishu" | "lark" => run_feishu_scan_setup(args).await,
        "qq" => setup_qq_external_scan(&args),
        "weixin" | "wechat-personal" => setup_weixin_external_scan(&args),
        "all" => {
            print_support_matrix();
            Ok(())
        }
        unsupported => Err(anyhow!(
            "platform `{}` does not support QR setup in this bridge. {}",
            unsupported,
            unsupported_reason(unsupported)
        )),
    }
}

async fn run_feishu_scan_setup(args: SetupArgs) -> Result<()> {
    let mut feishu_args = vec![
        "setup".to_string(),
        "--config".to_string(),
        args.config_path,
        "--project".to_string(),
        args.project,
        "--work-dir".to_string(),
        args.work_dir,
        "--platform-type".to_string(),
        args.platform,
    ];
    feishu_args.extend(args.passthrough);
    run_feishu_command(feishu_args).await
}

fn setup_qq_external_scan(args: &SetupArgs) -> Result<()> {
    update_platform_config(
        args,
        "qq",
        "qq",
        vec![
            ("name", "qq".to_string()),
            ("ws_url", args.ws_url.clone()),
            ("access_token", args.token.clone().unwrap_or_default()),
        ],
    )?;
    println!("QQ configured for external QR login.");
    println!("  config:   {}", args.config_path);
    println!("  project:  {}", args.project);
    println!("  platform: qq");
    println!("  ws_url:   {}", args.ws_url);
    println!();
    println!("Next:");
    println!("  1. Start NapCat/LLOneBot or another OneBot v11 gateway.");
    println!("  2. Scan the QQ login QR code in that gateway.");
    println!("  3. Keep the gateway WebSocket at the ws_url above.");
    println!("  4. Run: agentlink --config {}", args.config_path);
    Ok(())
}

fn setup_weixin_external_scan(args: &SetupArgs) -> Result<()> {
    update_platform_config(
        args,
        "weixin",
        "weixin",
        vec![
            ("name", "weixin".to_string()),
            (
                "token",
                args.token
                    .clone()
                    .unwrap_or_else(|| "paste-openclaw-ilink-token".to_string()),
            ),
            ("api_base", "https://ilinkai.weixin.qq.com".to_string()),
        ],
    )?;
    println!("Weixin personal account configured for external QR login.");
    println!("  config:   {}", args.config_path);
    println!("  project:  {}", args.project);
    println!("  platform: weixin");
    println!();
    println!("Next:");
    println!("  1. Use OpenClaw/iLink compatible gateway to scan-login the WeChat account.");
    println!("  2. Put the gateway bearer token into the weixin token field if it changed.");
    println!("  3. Run: agentlink --config {}", args.config_path);
    Ok(())
}
