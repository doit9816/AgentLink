use crate::channels::support::*;

pub(super) async fn wecom_access_token(platform: &WeComPlatform) -> Result<String> {
    let corp_id = platform
        .config
        .corp_id
        .as_ref()
        .or(platform.config.secret.as_ref())
        .ok_or_else(|| anyhow!("wecom corp_id is required"))?;
    let corp_secret = platform
        .config
        .corp_secret
        .as_ref()
        .or(platform.config.token.as_ref())
        .ok_or_else(|| anyhow!("wecom corp_secret is required"))?;
    let base = if platform.config.api_base.trim().is_empty() {
        "https://qyapi.weixin.qq.com".to_string()
    } else {
        platform.config.api_base.trim_end_matches('/').to_string()
    };
    let value: Value = platform
        .client
        .get(format!(
            "{base}/cgi-bin/gettoken?corpid={corp_id}&corpsecret={corp_secret}"
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    value
        .get("access_token")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("wecom gettoken response missing access_token"))
}
