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

pub(super) fn truncate_json(value: &Value, limit: usize) -> String {
    let text = value.to_string();
    if text.chars().count() <= limit {
        text
    } else {
        format!("{}...", text.chars().take(limit).collect::<String>())
    }
}
