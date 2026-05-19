use crate::channels::support::*;

pub use crate::channels::support::{weixin_config_from_options, PollPlatformConfig, WeixinPlatform};

#[async_trait]
impl Platform for WeixinPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let (tx, rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(tx);
        tokio::spawn(run_weixin_polling(self.clone_for_task(), rx));
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: SimpleReplyContext = serde_json::from_str(&reply_ctx.value)?;
        emit_outbound(self, &ctx, &content).await;
        if !self.config.dry_run {
            let request = self.client.post(format!(
                "{}/ilink/bot/sendmessage",
                self.config.api_base.trim_end_matches('/')
            ));
            weixin_business_request(request, &self.config)
                .json(&json!({
                    "msg": {
                        "to_user_id": ctx.target,
                        "client_id": ctx.extra.get("client_id").and_then(Value::as_str).unwrap_or(""),
                        "message_type": 2,
                        "message_state": 2,
                        "context_token": ctx.extra.get("context_token").and_then(Value::as_str).unwrap_or(""),
                        "item_list": [{ "type": 1, "text_item": { "text": content }}]
                    },
                    "base_info": { "channel_version": "agentlink-weixin/1.0" }
                }))
                .send()
                .await?
                .error_for_status()?;
        }
        Ok(())
    }

    async fn send(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        self.reply(reply_ctx, content).await
    }

    async fn stop(&self) -> Result<()> {
        if let Some(tx) = self.shutdown.lock().await.take() {
            let _ = tx.send(());
        }
        Ok(())
    }
}

async fn run_weixin_polling(platform: Arc<WeixinPlatform>, mut shutdown_rx: oneshot::Receiver<()>) {
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
        .or_else(|| item.get("content").and_then(Value::as_str).map(str::to_string))
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
                "context_token": item.get("context_token").and_then(Value::as_str).unwrap_or(""),
                "client_id": item.get("client_id").and_then(Value::as_str).unwrap_or("")
            }),
        },
    )
}

fn weixin_business_request(
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

fn weixin_sender_allowed(allow_from: Option<&str>, from: &str) -> bool {
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

fn truncate_json(value: &Value, limit: usize) -> String {
    let text = value.to_string();
    if text.chars().count() <= limit {
        text
    } else {
        format!("{}...", text.chars().take(limit).collect::<String>())
    }
}
