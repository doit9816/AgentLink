use super::helpers::{truncate_json, weixin_business_request, weixin_sender_allowed};
use crate::channels::support::*;

pub(super) async fn run_weixin_polling(
    platform: Arc<WeixinPlatform>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let mut buf = String::new();
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => return,
            result = fetch_weixin_updates(&platform, &buf) => {
                match result {
                    Ok((updates, next)) => {
                        if let Some(next) = next {
                            buf = next;
                        }
                        tracing::debug!(
                            platform = %platform.config.name,
                            updates = updates.len(),
                            cursor_len = buf.len(),
                            "weixin poll response received"
                        );
                        for item in updates {
                            match weixin_message_from_item(&platform, &item) {
                                Ok(Some(message)) => {
                                    dispatch_webhook(Arc::clone(&platform) as Arc<dyn Platform>, &platform.handler, message).await;
                                }
                                Ok(None) => {
                                    tracing::debug!(platform = %platform.config.name, item = %truncate_json(&item, 240), "weixin update ignored");
                                }
                                Err(err) => {
                                    tracing::warn!(platform = %platform.config.name, error = %err, item = %truncate_json(&item, 240), "weixin update parse failed");
                                }
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "weixin poll failed");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

async fn fetch_weixin_updates(
    platform: &WeixinPlatform,
    buf: &str,
) -> Result<(Vec<Value>, Option<String>)> {
    let request = platform.client.post(format!(
        "{}/ilink/bot/getupdates",
        platform.config.api_base.trim_end_matches('/')
    ));
    let value: Value = weixin_business_request(request, &platform.config)
        .json(&json!({
            "get_updates_buf": buf,
            "base_info": { "channel_version": "agentlink-weixin/1.0" }
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let ret = value.get("ret").and_then(Value::as_i64);
    let errcode = value.get("errcode").and_then(Value::as_i64);
    if matches!(ret, Some(code) if code != 0) || matches!(errcode, Some(code) if code != 0) {
        return Err(anyhow!(
            "weixin getupdates error response: {}",
            truncate_json(&value, 500)
        ));
    }
    if ret == Some(0) {
        tracing::trace!(response = %truncate_json(&value, 500), "weixin getupdates ok");
    }
    let data = value.get("data").unwrap_or(&Value::Null);
    Ok((
        value
            .get("msgs")
            .or_else(|| value.get("messages"))
            .or_else(|| value.get("updates"))
            .or_else(|| data.get("msgs"))
            .or_else(|| data.get("messages"))
            .or_else(|| data.get("updates"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        value
            .get("get_updates_buf")
            .or_else(|| data.get("get_updates_buf"))
            .and_then(Value::as_str)
            .map(str::to_string),
    ))
}

fn weixin_message_from_item(platform: &WeixinPlatform, item: &Value) -> Result<Option<Message>> {
    if item.get("message_type").and_then(Value::as_i64) == Some(2) {
        return Ok(None);
    }
    let from = item
        .get("from_user_id")
        .or_else(|| item.get("from_user"))
        .or_else(|| item.get("user_id"))
        .or_else(|| item.pointer("/sender/user_id"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    if !weixin_sender_allowed(platform.config.allow_from.as_deref(), &from) {
        tracing::warn!(
            platform = %platform.config.name,
            user_id = %from,
            "weixin message rejected by allow_from"
        );
        return Ok(None);
    }
    let text = item
        .get("item_list")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find_map(|v| {
                v.pointer("/text_item/text")
                    .or_else(|| v.pointer("/text/text"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        })
        .or_else(|| item.get("text").and_then(Value::as_str).map(str::to_string))
        .or_else(|| {
            item.get("content")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .or_else(|| {
            item.pointer("/message/text")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    if text.trim().is_empty() {
        tracing::debug!(platform = %platform.config.name, item = %truncate_json(item, 240), "weixin message has no text content");
        return Ok(None);
    }
    let msg_id = item.get("message_id").map(value_to_string);
    let context_token = weixin_context_token(item);
    let client_id = weixin_string_field(item, &["client_id", "clientId", "ilink_bot_id", "bot_id"]);
    simple_message(
        &platform.config.name,
        format!("{}:{}", platform.config.name, from),
        from.clone(),
        msg_id.clone().as_deref(),
        text,
        SimpleReplyContext {
            channel: platform.config.name.clone(),
            target: from,
            message_id: msg_id,
            extra: json!({
                "context_token": context_token,
                "client_id": client_id
            }),
        },
    )
}

fn weixin_string_field(item: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(value) = item.get(*key).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                return value.to_string();
            }
        }
    }
    String::new()
}

fn weixin_context_token(item: &Value) -> String {
    weixin_string_field(
        item,
        &["context_token", "contextToken", "ctx_token", "context"],
    )
}
