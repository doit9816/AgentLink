use crate::core::{AttachmentKind, Message};

pub(crate) fn log_preview(value: &str, max_chars: usize) -> String {
    crate::util::preview::preview_text(value, max_chars)
}

pub(super) fn prompt_with_attachments(message: &Message) -> String {
    let mut prompt = if message.content.trim().is_empty() {
        format!("[{:?} message]", message.message_type)
    } else {
        message.content.clone()
    };
    let mut lines = Vec::new();
    for (index, attachment) in message.attachments.iter().enumerate() {
        let kind = match attachment.kind {
            AttachmentKind::File => "file",
            AttachmentKind::Image => "image",
            AttachmentKind::Audio => "audio",
            AttachmentKind::Video => "video",
            AttachmentKind::Location => "location",
            AttachmentKind::Card => "card",
            AttachmentKind::Sticker => "sticker",
            AttachmentKind::Raw => "raw",
        };
        let name = attachment
            .file_name
            .as_deref()
            .or(attachment.path.as_deref())
            .or(attachment.url.as_deref())
            .unwrap_or("");
        let metadata_text = attachment.metadata.as_ref().map(|value| value.to_string());
        let detail = attachment
            .text
            .as_deref()
            .or(metadata_text.as_deref())
            .unwrap_or("");
        lines.push(format!(
            "{}. type={}, mime={}, name={}, detail={}",
            index + 1,
            kind,
            attachment.mime_type.as_deref().unwrap_or(""),
            name,
            detail
        ));
    }
    if !message.images.is_empty() {
        lines.push(format!("legacy_images={}", message.images.len()));
    }
    if !message.files.is_empty() {
        lines.push(format!("legacy_files={}", message.files.len()));
    }
    if !lines.is_empty() {
        prompt.push_str("\n\n[attachments]\n");
        prompt.push_str(&lines.join("\n"));
    }
    prompt
}
