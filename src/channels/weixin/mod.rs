mod helpers;
mod polling;

use crate::channels::support::*;

pub use crate::channels::support::{
    weixin_config_from_options, PollPlatformConfig, WeixinPlatform,
};
use helpers::weixin_business_request;
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
