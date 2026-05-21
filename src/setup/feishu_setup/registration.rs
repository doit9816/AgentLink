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
pub(super) const OPEN_FEISHU_BASE: &str = "https://open.feishu.cn";
pub(super) const OPEN_LARK_BASE: &str = "https://open.larksuite.com";

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
pub(super) struct RegistrationResult {
    pub(super) app_id: String,
    pub(super) app_secret: String,
    pub(super) owner_open_id: Option<String>,
    pub(super) platform_type: String,
}

pub(super) async fn registration_flow(
    timeout_seconds: u64,
    debug: bool,
) -> Result<RegistrationResult> {
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
            .any(|value| value == "client_secret")
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

pub(super) async fn validate_credentials(
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
