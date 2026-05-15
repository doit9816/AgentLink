use anyhow::{anyhow, Result};
use qrcode::render::unicode;
use qrcode::QrCode;
use serde::Deserialize;
use serde_json::Value;
use std::io::Write;
use std::time::{Duration, Instant};
use tokio::time::sleep;

const ACCOUNTS_FEISHU_BASE: &str = "https://accounts.feishu.cn";
const ACCOUNTS_LARK_BASE: &str = "https://accounts.larksuite.com";
const OPEN_FEISHU_BASE: &str = "https://open.feishu.cn";
const OPEN_LARK_BASE: &str = "https://open.larksuite.com";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupMode {
    Auto,
    New,
    Bind,
}

#[derive(Debug, Clone)]
pub struct FeishuSetupArgs {
    mode: SetupMode,
    config_path: String,
    project: String,
    platform_type: Option<String>,
    app_id: Option<String>,
    app_secret: Option<String>,
    timeout_seconds: u64,
    work_dir: String,
    listen: String,
    callback_path: String,
    dry_run: bool,
    debug: bool,
}

#[derive(Debug, Deserialize)]
struct RegistrationInitResponse {
    #[serde(default)]
    supported_auth_methods: Vec<String>,
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

#[derive(Debug, Deserialize)]
struct RegistrationBeginResponse {
    #[serde(default)]
    device_code: String,
    #[serde(default)]
    verification_uri_complete: String,
    #[serde(default)]
    interval: u64,
    #[serde(default, alias = "expires_in")]
    expire_in: u64,
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

#[derive(Debug, Deserialize, Default)]
struct RegistrationUserInfo {
    #[serde(default)]
    open_id: String,
    #[serde(default)]
    tenant_brand: String,
}

#[derive(Debug, Deserialize)]
struct RegistrationPollResponse {
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    client_secret: String,
    #[serde(default)]
    user_info: RegistrationUserInfo,
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

#[derive(Debug, Clone)]
struct RegistrationResult {
    app_id: String,
    app_secret: String,
    owner_open_id: Option<String>,
    platform_type: String,
}

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

fn parse_args(values: &[String], mode: SetupMode) -> Result<FeishuSetupArgs> {
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

async fn registration_flow(timeout_seconds: u64, debug: bool) -> Result<RegistrationResult> {
    let client = reqwest::Client::new();
    let mut accounts_base = ACCOUNTS_FEISHU_BASE.to_string();

    let init: RegistrationInitResponse =
        registration_call(&client, &accounts_base, "init", &[], debug).await?;
    if !init.error.is_empty() {
        return Err(anyhow!("{}: {}", init.error, init.error_description));
    }
    if !init.supported_auth_methods.is_empty()
        && !init
            .supported_auth_methods
            .iter()
            .any(|v| v == "client_secret")
    {
        return Err(anyhow!(
            "current onboarding endpoint does not support client_secret"
        ));
    }

    let begin: RegistrationBeginResponse = registration_call(
        &client,
        &accounts_base,
        "begin",
        &[
            ("archetype", "PersonalAgent"),
            ("auth_method", "client_secret"),
            ("request_user_info", "open_id"),
        ],
        debug,
    )
    .await?;
    if !begin.error.is_empty() {
        return Err(anyhow!("{}: {}", begin.error, begin.error_description));
    }
    if begin.device_code.is_empty() || begin.verification_uri_complete.is_empty() {
        return Err(anyhow!("incomplete onboarding response"));
    }

    println!("请使用飞书/Lark 手机 App 扫码完成机器人创建与授权：");
    println!("URL: {}\n", begin.verification_uri_complete);
    print_qr(&begin.verification_uri_complete)?;
    std::io::stdout().flush()?;

    let mut interval = begin.interval.max(5);
    let expire_in = begin.expire_in.max(timeout_seconds);
    let timeout_at =
        Instant::now() + Duration::from_secs(std::cmp::min(expire_in, timeout_seconds));
    let mut platform_type = "feishu".to_string();

    while Instant::now() < timeout_at {
        let poll: RegistrationPollResponse = registration_call(
            &client,
            &accounts_base,
            "poll",
            &[("device_code", begin.device_code.as_str())],
            debug,
        )
        .await?;
        let tenant_brand = poll.user_info.tenant_brand.trim().to_ascii_lowercase();
        if tenant_brand == "lark" && accounts_base != ACCOUNTS_LARK_BASE {
            platform_type = "lark".to_string();
            accounts_base = ACCOUNTS_LARK_BASE.to_string();
            continue;
        }
        if !poll.client_id.is_empty() && !poll.client_secret.is_empty() {
            return Ok(RegistrationResult {
                app_id: poll.client_id,
                app_secret: poll.client_secret,
                owner_open_id: (!poll.user_info.open_id.is_empty())
                    .then_some(poll.user_info.open_id),
                platform_type,
            });
        }
        match poll.error.as_str() {
            "" | "authorization_pending" => {}
            "slow_down" => interval += 5,
            "access_denied" => return Err(anyhow!("authorization denied by user")),
            "expired_token" => return Err(anyhow!("onboarding session expired")),
            other => return Err(anyhow!("{}: {}", other, poll.error_description)),
        }
        sleep(Duration::from_secs(interval)).await;
    }
    Err(anyhow!("timed out waiting for QR onboarding result"))
}

async fn registration_call<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    base: &str,
    action: &str,
    params: &[(&str, &str)],
    debug: bool,
) -> Result<T> {
    let mut form = vec![("action", action)];
    form.extend(params.iter().copied());
    let response = client
        .post(format!("{base}/oauth/v1/app/registration"))
        .form(&form)
        .send()
        .await?;
    let text = response.text().await?;
    if debug {
        eprintln!("[debug] registration action={action} body={text}");
    }
    Ok(serde_json::from_str(&text)?)
}

async fn validate_credentials(
    app_id: &str,
    app_secret: &str,
    platform_type: Option<&str>,
) -> Result<String> {
    let candidates: Vec<(&str, &str)> = match platform_type {
        Some("lark") => vec![("lark", OPEN_LARK_BASE)],
        Some("feishu") => vec![("feishu", OPEN_FEISHU_BASE)],
        _ => vec![("feishu", OPEN_FEISHU_BASE), ("lark", OPEN_LARK_BASE)],
    };
    let client = reqwest::Client::new();
    let mut last_err = None;
    for (kind, base) in candidates {
        let result = client
            .post(format!(
                "{base}/open-apis/auth/v3/tenant_access_token/internal"
            ))
            .json(&serde_json::json!({
                "app_id": app_id,
                "app_secret": app_secret
            }))
            .send()
            .await;
        match result {
            Ok(resp) => {
                let value: Value = resp.error_for_status()?.json().await?;
                if value.get("code").and_then(Value::as_i64) == Some(0)
                    && value
                        .get("tenant_access_token")
                        .and_then(Value::as_str)
                        .is_some()
                {
                    return Ok(kind.to_string());
                }
                last_err = Some(anyhow!("{} returned {}", kind, value));
            }
            Err(err) => last_err = Some(err.into()),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("credential validation failed")))
}

fn write_feishu_config(
    args: &FeishuSetupArgs,
    result: &RegistrationResult,
    platform_type: &str,
) -> Result<()> {
    let mut root = if std::path::Path::new(&args.config_path).exists() {
        let raw = std::fs::read_to_string(&args.config_path)?;
        raw.parse::<toml::Value>()?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let table = root
        .as_table_mut()
        .ok_or_else(|| anyhow!("config root must be a TOML table"))?;
    table
        .entry("data_dir".to_string())
        .or_insert(toml::Value::String("./data".to_string()));

    let projects = table
        .entry("projects".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| anyhow!("projects must be an array"))?;

    let project_index = projects
        .iter()
        .position(|p| p.get("name").and_then(toml::Value::as_str) == Some(args.project.as_str()))
        .unwrap_or_else(|| {
            projects.push(default_project(&args.project, &args.work_dir));
            projects.len() - 1
        });
    let project = projects[project_index]
        .as_table_mut()
        .ok_or_else(|| anyhow!("project must be a table"))?;
    ensure_default_platform(project, platform_type)?;
    let platforms = project
        .entry("platforms".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| anyhow!("project.platforms must be an array"))?;
    let platform_index = platforms
        .iter()
        .position(|p| {
            p.get("type")
                .and_then(toml::Value::as_str)
                .is_some_and(|v| v == "feishu" || v == "lark")
        })
        .unwrap_or_else(|| {
            platforms.push(toml::Value::Table(toml::map::Map::new()));
            platforms.len() - 1
        });
    platforms[platform_index] = feishu_platform_value(args, result, platform_type);
    let rendered = toml::to_string_pretty(&root)?;
    if let Some(parent) = std::path::Path::new(&args.config_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(&args.config_path, rendered)?;
    Ok(())
}

fn default_project(name: &str, work_dir: &str) -> toml::Value {
    let mut project = toml::map::Map::new();
    project.insert("name".to_string(), toml::Value::String(name.to_string()));
    let mut agent = toml::map::Map::new();
    agent.insert("type".to_string(), toml::Value::String("codex".to_string()));
    let mut options = toml::map::Map::new();
    options.insert(
        "work_dir".to_string(),
        toml::Value::String(work_dir.to_string()),
    );
    options.insert(
        "mode".to_string(),
        toml::Value::String("suggest".to_string()),
    );
    options.insert(
        "codex_bin".to_string(),
        toml::Value::String("codex".to_string()),
    );
    agent.insert("options".to_string(), toml::Value::Table(options));
    project.insert("agent".to_string(), toml::Value::Table(agent));
    project.insert(
        "default_platforms".to_string(),
        toml::Value::Array(Vec::new()),
    );
    project.insert("platforms".to_string(), toml::Value::Array(Vec::new()));
    toml::Value::Table(project)
}

fn ensure_default_platform(
    project: &mut toml::map::Map<String, toml::Value>,
    platform_id: &str,
) -> Result<()> {
    let defaults = project
        .entry("default_platforms".to_string())
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| anyhow!("project.default_platforms must be an array"))?;
    if !defaults
        .iter()
        .any(|value| value.as_str() == Some(platform_id))
    {
        defaults.push(toml::Value::String(platform_id.to_string()));
    }
    Ok(())
}

fn feishu_platform_value(
    args: &FeishuSetupArgs,
    result: &RegistrationResult,
    platform_type: &str,
) -> toml::Value {
    let mut platform = toml::map::Map::new();
    platform.insert(
        "id".to_string(),
        toml::Value::String(platform_type.to_string()),
    );
    platform.insert(
        "type".to_string(),
        toml::Value::String(platform_type.to_string()),
    );
    let mut options = toml::map::Map::new();
    options.insert(
        "name".to_string(),
        toml::Value::String(platform_type.to_string()),
    );
    options.insert(
        "listen".to_string(),
        toml::Value::String(args.listen.clone()),
    );
    options.insert(
        "callback_path".to_string(),
        toml::Value::String(args.callback_path.clone()),
    );
    options.insert(
        "connection_mode".to_string(),
        toml::Value::String("websocket".to_string()),
    );
    options.insert(
        "app_id".to_string(),
        toml::Value::String(result.app_id.clone()),
    );
    options.insert(
        "app_secret".to_string(),
        toml::Value::String(result.app_secret.clone()),
    );
    if platform_type == "lark" {
        options.insert(
            "api_base".to_string(),
            toml::Value::String(OPEN_LARK_BASE.to_string()),
        );
    }
    if args.dry_run {
        options.insert("dry_run".to_string(), toml::Value::Boolean(true));
    }
    platform.insert("options".to_string(), toml::Value::Table(options));
    toml::Value::Table(platform)
}

fn print_qr(content: &str) -> Result<()> {
    let code = QrCode::new(content.as_bytes())?;
    let image = code
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Dark)
        .light_color(unicode::Dense1x2::Light)
        .build();
    println!("{image}");
    Ok(())
}

fn print_feishu_help() {
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

#[cfg(test)]
mod tests {
    use super::*;

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
