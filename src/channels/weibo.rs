use crate::channels::support::*;
use crate::channels::support::ws::{resolve_platform_token, run_simple_ws};

pub use crate::channels::support::{weibo_config_from_options, PollPlatformConfig, WeiboPlatform};

#[async_trait]
impl Platform for WeiboPlatform {
    fn name(&self) -> &str {
        &self.config.name
    }

    async fn start(&self, handler: MessageHandler) -> Result<()> {
        *self.handler.lock().await = Some(handler);
        let (tx, rx) = oneshot::channel();
        *self.shutdown.lock().await = Some(tx);
        tokio::spawn(run_simple_ws(self.clone_for_task(), rx, false));
        Ok(())
    }

    async fn reply(&self, reply_ctx: ReplyContext, content: String) -> Result<()> {
        let ctx: SimpleReplyContext = serde_json::from_str(&reply_ctx.value)?;
        emit_outbound(self, &ctx, &content).await;
        if !self.config.dry_run {
            let token = resolve_platform_token(&self.config).await?;
            self.client
                .post(format!(
                    "{}/messages",
                    self.config.api_base.trim_end_matches('/')
                ))
                .bearer_auth(token)
                .json(&json!({ "receiver_id": ctx.target, "text": content }))
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
