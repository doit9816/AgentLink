mod helpers;
mod polling;

use crate::channels::support::*;

pub use crate::channels::support::{
    weixin_config_from_options, PollPlatformConfig, WeixinPlatform,
};
use helpers::{
    truncate_json, weixin_business_request, weixin_check_api_response, weixin_generate_client_id,
};
use polling::run_weixin_polling;

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
        if self.config.dry_run {
            tracing::info!(
                platform = %self.config.name,
                target = %ctx.target,
                content_len = content.len(),
                "weixin outbound skipped (dry_run)"
            );
            return Ok(());
        }
        tracing::info!(
            platform = %self.config.name,
            target = %ctx.target,
            content_len = content.len(),
            has_context_token = ctx
                .extra
                .get("context_token")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|v| !v.is_empty()),
            "weixin outbound api request"
        );
        let response = weixin_business_request(
            self.client.post(format!(
                "{}/ilink/bot/sendmessage",
                self.config.api_base.trim_end_matches('/')
            )),
            &self.config,
        )
        .json(&json!({
            "msg": {
                "from_user_id": "",
                "to_user_id": ctx.target,
                "client_id": weixin_generate_client_id(),
                "message_type": 2,
                "message_state": 2,
                "context_token": ctx.extra.get("context_token").and_then(Value::as_str).unwrap_or(""),
                "item_list": [{ "type": 1, "text_item": { "text": content }}]
            },
            "base_info": { "channel_version": "agentlink-weixin/1.0" }
        }))
        .send()
        .await?;
        let status = response.status();
        let body_text = response.text().await.unwrap_or_default();
        let value: Value = serde_json::from_str(&body_text).unwrap_or(Value::Null);
        if !status.is_success() {
            tracing::warn!(
                platform = %self.config.name,
                target = %ctx.target,
                status = %status,
                body = %truncate_json(&value, 200),
                "weixin outbound api failed"
            );
            return Err(anyhow!("weixin sendmessage failed: {status} {body_text}"));
        }
        if let Err(err) = weixin_check_api_response(&value) {
            tracing::warn!(
                platform = %self.config.name,
                target = %ctx.target,
                body = %truncate_json(&value, 200),
                error = %err,
                "weixin outbound api business error"
            );
            return Err(err);
        }
        tracing::info!(
            platform = %self.config.name,
            target = %ctx.target,
            status = %status,
            "weixin outbound api ok"
        );
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
