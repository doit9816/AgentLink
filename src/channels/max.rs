use crate::channels::support::*;

pub use crate::channels::support::{max_config_from_options, MaxPlatform, PollPlatformConfig};

#[async_trait]
impl Platform for MaxPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let (tx, rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(tx);
        if self.config.webhook_url.is_some() {
            tokio::spawn(run_max_webhook(self.clone_for_task(), rx));
        } else {
            tokio::spawn(run_max_polling(self.clone_for_task(), rx));
        }
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: SimpleReplyContext = serde_json::from_str(&reply_ctx.value)?;
        emit_outbound(self, &ctx, &content).await;
        if !self.config.dry_run {
            self.client
                .post(format!(
                    "{}/messages",
                    self.config.api_base.trim_end_matches('/')
                ))
                .bearer_auth(&self.config.token)
                .json(&json!({ "chat_id": ctx.target, "text": content }))
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

async fn run_max_polling(platform: Arc<MaxPlatform>, mut shutdown_rx: oneshot::Receiver<()>) {
    let mut marker: Option<String> = None;
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => return,
            result = fetch_max_updates(&platform, marker.clone()) => {
                match result {
                    Ok((updates, next)) => {
                        marker = next;
                        for update in updates {
                            if let Ok(Some(message)) = max_message_from_update(&platform, &update) {
                                dispatch_webhook(Arc::clone(&platform) as Arc<dyn Platform>, &platform.handler, message).await;
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "max poll failed");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

async fn run_max_webhook(platform: Arc<MaxPlatform>, shutdown_rx: oneshot::Receiver<()>) {
    if !platform.config.dry_run {
        if let Err(err) = max_subscribe_webhook(&platform).await {
            tracing::warn!(error = %err, "max webhook subscribe failed");
        }
    }
    let listener = match TcpListener::bind(&platform.config.webhook_listen).await {
        Ok(listener) => listener,
        Err(err) => {
            tracing::warn!(error = %err, "max webhook bind failed");
            return;
        }
    };
    if let Ok(addr) = listener.local_addr() {
        *platform.local_addr.lock().await = Some(addr);
    }
    let app = Router::new()
        .route(
            &normalize_path(platform.config.webhook_path.clone()),
            post(max_webhook),
        )
        .with_state(platform);
    let _ = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        })
        .await;
}

async fn max_subscribe_webhook(platform: &MaxPlatform) -> Result<()> {
    let mut body = json!({
        "url": platform.config.webhook_url.as_deref().unwrap_or_default(),
        "update_types": ["message_created", "message_callback"]
    });
    if let Some(secret) = &platform.config.webhook_secret {
        body["secret"] = json!(secret);
    }
    platform
        .client
        .post(format!(
            "{}/subscriptions",
            platform.config.api_base.trim_end_matches('/')
        ))
        .bearer_auth(&platform.config.token)
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn max_webhook(
    State(platform): State<Arc<MaxPlatform>>,
    Query(params): Query<BTreeMap<String, String>>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Response {
    if let Some(expected) = &platform.config.webhook_secret {
        let actual = headers
            .get("x-max-bot-api-secret")
            .and_then(|v| v.to_str().ok())
            .or_else(|| params.get("s").map(String::as_str));
        if actual != Some(expected.as_str()) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    match max_message_from_update(&platform, &payload) {
        Ok(Some(message)) => {
            dispatch_webhook(
                Arc::clone(&platform) as Arc<dyn Platform>,
                &platform.handler,
                message,
            )
            .await;
            Json(json!({ "ok": true })).into_response()
        }
        Ok(None) => Json(json!({ "ok": true, "ignored": true })).into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    }
}

async fn fetch_max_updates(
    platform: &MaxPlatform,
    marker: Option<String>,
) -> Result<(Vec<Value>, Option<String>)> {
    let mut req = platform
        .client
        .get(format!(
            "{}/updates",
            platform.config.api_base.trim_end_matches('/')
        ))
        .bearer_auth(&platform.config.token)
        .query(&[
            ("timeout", platform.config.poll_timeout_secs.to_string()),
            ("limit", "20".to_string()),
            ("types", "message_created".to_string()),
        ]);
    if let Some(marker) = marker {
        req = req.query(&[("marker", marker)]);
    }
    let value: Value = req.send().await?.error_for_status()?.json().await?;
    let updates = value
        .get("updates")
        .or_else(|| value.get("result"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let marker = value.get("marker").map(value_to_string);
    Ok((updates, marker))
}

fn max_message_from_update(platform: &MaxPlatform, update: &Value) -> Result<Option<Message>> {
    let msg = update.get("message").unwrap_or(update);
    let text = msg
        .pointer("/body/text")
        .or_else(|| msg.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Ok(None);
    }
    let chat_id = msg
        .pointer("/recipient/chat_id")
        .or_else(|| msg.get("chat_id"))
        .map(value_to_string)
        .ok_or_else(|| anyhow!("max message missing chat_id"))?;
    let user_id = msg
        .pointer("/sender/user_id")
        .or_else(|| msg.get("user_id"))
        .map(value_to_string)
        .unwrap_or_else(|| "unknown".to_string());
    let mid = msg
        .pointer("/body/mid")
        .or_else(|| msg.get("mid"))
        .map(value_to_string);
    let session_key = if platform.config.share_session_in_channel {
        format!("{}:{}", platform.config.name, chat_id)
    } else {
        format!("{}:{}:{}", platform.config.name, chat_id, user_id)
    };
    simple_message(
        &platform.config.name,
        session_key,
        user_id,
        mid.clone().as_deref(),
        text,
        SimpleReplyContext {
            channel: platform.config.name.clone(),
            target: chat_id,
            message_id: mid,
            extra: Value::Null,
        },
    )
}
