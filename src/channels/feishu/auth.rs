use super::FeishuPlatform;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub(crate) struct CachedToken {
    pub(crate) value: String,
    pub(crate) expires_at: Instant,
}

pub(crate) async fn tenant_access_token(platform: &FeishuPlatform) -> Result<String> {
    if let Some(cached) = platform.token.lock().await.clone() {
        if Instant::now() < cached.expires_at {
            return Ok(cached.value);
        }
    }
    let resp: Value = platform
        .client
        .post(format!(
            "{}/open-apis/auth/v3/tenant_access_token/internal",
            platform.config.api_base
        ))
        .json(&json!({
            "app_id": platform.config.app_id,
            "app_secret": platform.config.app_secret
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let token = resp
        .get("tenant_access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("feishu token response missing tenant_access_token"))?
        .to_string();
    let expire = resp.get("expire").and_then(Value::as_u64).unwrap_or(7200);
    *platform.token.lock().await = Some(CachedToken {
        value: token.clone(),
        expires_at: Instant::now() + Duration::from_secs(expire.saturating_sub(300)),
    });
    Ok(token)
}
