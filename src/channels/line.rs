use crate::channels::support::*;

pub use crate::channels::support::{LinePlatform, LinePlatformConfig};

#[async_trait]
impl Platform for LinePlatform {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let listener = TcpListener::bind(&self.config.listen).await?;
        *self.local_addr.lock().await = Some(listener.local_addr()?);
        let (tx, rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(tx);
        let path = self.config.callback_path.clone();
        let app = Router::new()
            .route(&path, post(line_webhook))
            .with_state(Arc::new(self.clone_for_server()));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: SimpleReplyContext = serde_json::from_str(&reply_ctx.value)?;
        emit_outbound(self, &ctx, &content).await;
        if self.config.dry_run {
            return Ok(());
        }
        let token = self
            .config
            .token
            .as_ref()
            .ok_or_else(|| anyhow!("line channel_token is required"))?;
        let base = if self.config.api_base.trim().is_empty() {
            "https://api.line.me".to_string()
        } else {
            self.config.api_base.trim_end_matches('/').to_string()
        };
        self.client
            .post(format!("{base}/v2/bot/message/push"))
            .bearer_auth(token)
            .json(&json!({
                "to": ctx.target,
                "messages": [{ "type": "text", "text": content }]
            }))
            .send()
            .await?
            .error_for_status()?;
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

async fn line_webhook(
    State(platform): State<Arc<LinePlatform>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(secret) = &platform.config.secret {
        if !platform.config.dry_run && !verify_line_signature(secret, &headers, &body) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    let payload: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(err) => return (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    };
    for event in payload
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        if let Ok(Some(message)) = line_message_from_event(&platform, &event) {
            dispatch_webhook(
                Arc::clone(&platform) as Arc<dyn Platform>,
                &platform.handler,
                message,
            )
            .await;
        }
    }
    Json(json!({ "ok": true })).into_response()
}

fn line_message_from_event(platform: &LinePlatform, event: &Value) -> Result<Option<Message>> {
    if event.get("type").and_then(Value::as_str) != Some("message")
        || event.pointer("/message/type").and_then(Value::as_str) != Some("text")
    {
        return Ok(None);
    }
    let content = event
        .pointer("/message/text")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Ok(None);
    }
    let source = event
        .get("source")
        .ok_or_else(|| anyhow!("missing source"))?;
    let user_id = source
        .get("userId")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let target = source
        .get("groupId")
        .or_else(|| source.get("roomId"))
        .or_else(|| source.get("userId"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing line target"))?
        .to_string();
    let session_key = if platform.config.share_session_in_channel {
        format!("{}:{}", platform.config.name, target)
    } else {
        format!("{}:{}:{}", platform.config.name, target, user_id)
    };
    simple_message(
        &platform.config.name,
        session_key,
        user_id,
        event.pointer("/message/id").and_then(Value::as_str),
        content,
        SimpleReplyContext {
            channel: platform.config.name.clone(),
            target,
            message_id: event
                .pointer("/message/id")
                .and_then(Value::as_str)
                .map(str::to_string),
            extra: Value::Null,
        },
    )
}
