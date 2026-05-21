use super::shared::{attachment_path_or_temp, ensure_work_dir_exists};
use super::CodexAgent;
use crate::core::{FileAttachment, ImageAttachment};
use anyhow::Result;

pub(crate) async fn codex_exec_prompt_and_images(
    agent: &CodexAgent,
    mut prompt: String,
    images: Vec<ImageAttachment>,
    files: Vec<FileAttachment>,
) -> Result<(String, Vec<String>)> {
    let mut image_paths = Vec::new();
    for (index, image) in images.into_iter().enumerate() {
        if let Some(path) =
            attachment_path_or_temp("image", index, image.file_name, image.data).await?
        {
            image_paths.push(path);
        }
    }
    let mut file_notes = Vec::new();
    for (index, file) in files.into_iter().enumerate() {
        if let Some(path) =
            attachment_path_or_temp("file", index, file.file_name, file.data).await?
        {
            file_notes.push(format!("- {path}"));
        }
    }
    if !file_notes.is_empty() {
        prompt.push_str("\n\n[local files]\n");
        prompt.push_str(&file_notes.join("\n"));
    }
    if prompt.trim().is_empty() && !image_paths.is_empty() {
        prompt = "Please analyze the attached image(s).".to_string();
    }
    ensure_work_dir_exists(&agent.work_dir)?;
    Ok((prompt, image_paths))
}
