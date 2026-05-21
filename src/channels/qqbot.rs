use crate::channels::support::ws::{resolve_platform_token, run_simple_ws};
use crate::channels::support::*;

pub use crate::channels::support::{qqbot_config_from_options, PollPlatformConfig, QqBotPlatform};

#[async_trait]
impl Platform for QqBotPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let (tx, rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(tx);
        tokio::spawn(run_simple_ws(self.clone_for_task(), rx, true));
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: SimpleReplyContext = serde_json::from_str(&reply_ctx.value)?;
        emit_outbound(self, &ctx, &content).await;
        if !self.config.dry_run {
            let token = resolve_platform_token(&self.config).await?;
            let kind = ctx
                .extra
                .get("target_kind")
                .and_then(Value::as_str)
                .unwrap_or("group");
            let base = self.config.api_base.trim_end_matches('/');
            let endpoint = match kind {
                "user" => format!("{base}/v2/users/{}/messages", ctx.target),
                "channel" => format!("{base}/channels/{}/messages", ctx.target),
                _ => format!("{base}/v2/groups/{}/messages", ctx.target),
            };
            self.client
                .post(endpoint)
                .bearer_auth(token)
                .json(&json!({ "content": content, "msg_id": ctx.message_id }))
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
