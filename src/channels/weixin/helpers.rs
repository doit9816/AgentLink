use crate::channels::support::*;

pub(super) fn weixin_business_request(
    request: reqwest::RequestBuilder,
    config: &PollPlatformConfig,
) -> reqwest::RequestBuilder {
    let uin = base64::engine::general_purpose::STANDARD
        .encode(rand::thread_rng().gen::<u32>().to_string());
    let mut request = request
        .bearer_auth(&config.token)
        .header("AuthorizationType", "ilink_bot_token")
        .header("X-WECHAT-UIN", uin);
    if let Some(route_tag) = config
        .route_tag
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request = request.header("SKRouteTag", route_tag);
    }
    request
}

pub(super) fn weixin_sender_allowed(allow_from: Option<&str>, from: &str) -> bool {
    let Some(allow_from) = allow_from.map(str::trim).filter(|value| !value.is_empty()) else {
        return true;
    };
    allow_from == "*"
        || allow_from
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .any(|value| value == from)
}

pub(super) fn weixin_api_business_error(value: &Value) -> Option<String> {
    let ret = value.get("ret").and_then(Value::as_i64);
    let errcode = value.get("errcode").and_then(Value::as_i64);
    if matches!(ret, Some(code) if code != 0) || matches!(errcode, Some(code) if code != 0) {
        return Some(truncate_json(value, 500));
    }
    None
}

pub(super) fn weixin_check_api_response(value: &Value) -> Result<()> {
    if value.as_object().is_some_and(|object| object.is_empty()) {
        return Err(anyhow!(
            "weixin api returned empty object; message may not have been delivered (try sending a new inbound message and refresh targets)"
        ));
    }
    if let Some(detail) = weixin_api_business_error(value) {
        return Err(anyhow!("weixin api business error: {detail}"));
    }
    Ok(())
}

pub(super) fn weixin_generate_client_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("agentlink_{}_{}", rand::thread_rng().gen::<u64>(), ts)
}

pub(super) fn truncate_json(value: &Value, limit: usize) -> String {
    let text = value.to_string();
    if text.chars().count() <= limit {
        text
    } else {
        format!("{}...", text.chars().take(limit).collect::<String>())
    }
}
