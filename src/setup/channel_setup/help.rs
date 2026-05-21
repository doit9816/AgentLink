pub fn unsupported_reason(platform: &str) -> &'static str {
    match platform {
        "dingtalk" => "DingTalk bot apps are configured with client_id/client_secret/robot_code, not QR onboarding.",
        "telegram" => "Telegram bots are created by BotFather and use a bot token.",
        "slack" => "Slack apps use app-level and bot tokens; Socket Mode has no QR onboarding.",
        "discord" => "Discord bots use a bot token from Developer Portal.",
        "line" => "Line Messaging API uses channel secret and channel access token.",
        "wecom" => "WeCom apps use corp_id/corp_secret/agent_id and callback settings.",
        "max" => "MAX bots use a bot token.",
        "qqbot" => "QQ official bot uses app_id/app_secret for Gateway auth.",
        "weibo" => "Weibo bot integration uses app_id/app_secret and ws token endpoint.",
        "http" | "bridge" => "This is an adapter protocol, not a login channel.",
        "mock" => "Mock channel does not need onboarding.",
        _ => "No QR onboarding implementation is available.",
    }
}

pub fn print_support_matrix() {
    println!("QR setup support:");
    println!("  feishu/lark  supported, bridge prints QR and writes app_id/app_secret");
    println!("  qq           external QR in NapCat/LLOneBot; bridge writes OneBot config");
    println!("  weixin       external QR in OpenClaw/iLink gateway; bridge writes config shell");
    println!("  others       no QR onboarding; use platform app tokens/secrets");
}

pub fn print_setup_help() {
    println!(
        r#"agentlink setup --platform <name> [options]

Examples:
  agentlink setup --platform feishu --config agentlink.toml --project demo
  agentlink setup --platform qq --config agentlink.toml --ws-url ws://127.0.0.1:3001
  agentlink setup --platform weixin --config agentlink.toml --token <gateway-token>
  agentlink setup --platform all

Options:
  --platform, --channel <name>  feishu, lark, qq, weixin, or all
  --config <path>              Config file to create/update. Default: agentlink.toml
  --project <name>             Project name. Default: demo
  --work-dir <path>            Codex work_dir when creating config
  --ws-url <url>               QQ OneBot gateway WebSocket URL
  --token <token>              External gateway token
  --app/--app-id/--app-secret  Passed through to feishu/lark setup
"#
    );
}
