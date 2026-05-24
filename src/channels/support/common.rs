use super::*;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimpleOutboundRecord {
    pub channel: String,
    pub target: String,
    pub message_id: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SimpleReplyContext {
    pub(crate) channel: String,
    pub(crate) target: String,
    pub(crate) message_id: Option<String>,
    pub(crate) extra: Value,
}

pub(crate) trait HasOutbox {
    fn outbox(&self) -> &Arc<Mutex<Vec<SimpleOutboundRecord>>>;
    fn out_tx(&self) -> &tokio::sync::mpsc::UnboundedSender<SimpleOutboundRecord>;
}

pub(crate) async fn dispatch_webhook(
    platform: Arc<dyn Platform>,
    handler_ref: &Arc<Mutex<Option<MessageHandler>>>,
    message: Message,
) {
    let platform_name = message
        .platform
        .clone()
        .unwrap_or_else(|| platform.name().to_string());
    tracing::info!(
        platform = %platform_name,
        session_key = %message.session_key,
        user_id = %message.user_id,
        message_type = ?message.message_type,
        content_len = message.content.len(),
        attachments = message.attachments.len(),
        preview = %crate::util::preview::preview_text(&message.content, 80),
        "channel inbound dispatch to engine"
    );
    let handler = handler_ref.lock().await.clone();
    if let Some(handler) = handler {
        tokio::spawn(async move {
            handler(platform, message).await;
        });
    } else {
        tracing::warn!(
            platform = %platform_name,
            session_key = %message.session_key,
            "channel inbound dropped: engine handler not ready"
        );
    }
}

pub(crate) fn simple_message(
    platform: &str,
    session_key: String,
    user_id: String,
    message_id: Option<&str>,
    content: String,
    reply_ctx: SimpleReplyContext,
) -> Result<Option<Message>> {
    Ok(Some(Message {
        project: None,
        session_key,
        platform: Some(platform.to_string()),
        message_id: message_id.map(str::to_string),
        message_type: crate::core::MessageType::Text,
        user_id,
        user_name: None,
        content,
        attachments: Vec::new(),
        images: Vec::new(),
        files: Vec::new(),
        reply_ctx: ReplyContext {
            value: serde_json::to_string(&reply_ctx)?,
        },
        created_at: std::time::SystemTime::now(),
    }))
}

pub(crate) async fn emit_outbound<P>(platform: &P, ctx: &SimpleReplyContext, content: &str)
where
    P: HasOutbox + ?Sized,
{
    let record = SimpleOutboundRecord {
        channel: ctx.channel.clone(),
        target: ctx.target.clone(),
        message_id: ctx.message_id.clone(),
        content: content.to_string(),
    };
    tracing::info!(
        channel = %ctx.channel,
        target = %ctx.target,
        message_id = ctx.message_id.as_deref().unwrap_or(""),
        content_len = content.len(),
        preview = %crate::util::preview::preview_text(content, 80),
        "channel outbound message"
    );
    platform.outbox().lock().await.push(record.clone());
    let _ = platform.out_tx().send(record);
}

pub(crate) fn verify_line_signature(secret: &str, headers: &HeaderMap, body: &[u8]) -> bool {
    let Some(actual) = headers
        .get("x-line-signature")
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let mut mac = match Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
        Ok(mac) => mac,
        Err(_) => return false,
    };
    mac.update(body);
    let expected = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    actual == expected
}

pub(crate) fn xml_tag(input: &str, tag: &str) -> Option<String> {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    let (_, rest) = input.split_once(&start)?;
    let (value, _) = rest.split_once(&end)?;
    Some(
        value
            .trim()
            .trim_start_matches("<![CDATA[")
            .trim_end_matches("]]>")
            .to_string(),
    )
}

pub(crate) fn value_to_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

pub(crate) fn normalize_path(path: String) -> String {
    if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    }
}
