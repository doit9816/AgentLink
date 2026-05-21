mod reply;
mod webhook;

use crate::channels::support::*;

pub use crate::channels::support::{WeComPlatform, WeComPlatformConfig};
use reply::wecom_access_token;
use webhook::{wecom_verify, wecom_webhook};

#[async_trait]
impl Platform for WeComPlatform {
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
            .route(&path, get(wecom_verify).post(wecom_webhook))
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
        let token = wecom_access_token(self).await?;
        let agent_id = ctx
            .extra
            .get("agent_id")
            .and_then(Value::as_str)
            .or_else(|| self.config.token.as_deref())
            .unwrap_or("0");
        let base = if self.config.api_base.trim().is_empty() {
            "https://qyapi.weixin.qq.com".to_string()
        } else {
            self.config.api_base.trim_end_matches('/').to_string()
        };
        self.client
            .post(format!("{base}/cgi-bin/message/send?access_token={token}"))
            .json(&json!({
                "touser": ctx.target,
                "msgtype": "text",
                "agentid": agent_id,
                "text": { "content": content }
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
