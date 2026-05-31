use super::Engine;
use crate::core::{DirCommand, Message, Platform, SessionCommand, SessionStartRequest};
use crate::store::ConversationSlot;
use crate::util::work_dir::{path_display, resolve_work_dir_path};
use anyhow::{anyhow, Result};
use std::path::Path;
use std::sync::Arc;

impl Engine {
    pub(crate) async fn handle_session_command(
        &self,
        platform: Arc<dyn Platform>,
        message: Message,
        command: SessionCommand,
    ) -> Result<()> {
        let platform_name = message
            .platform
            .clone()
            .unwrap_or_else(|| platform.name().to_string());
        let reply = match command {
            SessionCommand::New { label } => self.cmd_new(&platform_name, &message, label).await?,
            SessionCommand::List => self.cmd_list(&platform_name, &message.session_key)?,
            SessionCommand::Switch { slot_id } => {
                self.cmd_switch(&platform_name, &message, &slot_id).await?
            }
            SessionCommand::Current => self.cmd_current(&platform_name, &message.session_key)?,
            SessionCommand::Dir(dir) => {
                self.cmd_dir(&platform_name, &message.session_key, dir)
                    .await?
            }
        };
        platform.reply(message.reply_ctx, reply).await?;
        Ok(())
    }

    async fn cmd_new(
        &self,
        platform: &str,
        message: &Message,
        label: Option<String>,
    ) -> Result<String> {
        self.ensure_conversation_slots(platform, &message.session_key)?;
        self.drop_runtime_session(platform, &message.session_key)
            .await;
        let slot_id =
            self.store
                .next_conversation_slot_id(&self.project, platform, &message.session_key)?;
        let work_dir = self.effective_work_dir(platform, &message.session_key)?;
        let session = self
            .agent
            .start_session(SessionStartRequest::fresh(Some(work_dir)))
            .await?;
        let agent_session_id = session.current_session_id();
        let now = crate::store::unix_now();
        let slot = ConversationSlot {
            project: self.project.clone(),
            platform: platform.to_string(),
            session_key: message.session_key.clone(),
            slot_id: slot_id.clone(),
            label: label.unwrap_or_default(),
            agent_session_id: agent_session_id.clone(),
            is_active: true,
            created_at: now,
            updated_at: now,
        };
        self.store.insert_conversation_slot(&slot)?;
        self.store.set_active_conversation_slot(
            &self.project,
            platform,
            &message.session_key,
            &slot_id,
        )?;
        self.store.upsert_session(
            &self.project,
            platform,
            &message.session_key,
            self.agent.name(),
            &agent_session_id,
        )?;
        let key = self.runtime_session_key(platform, &message.session_key);
        self.sessions.lock().await.insert(key, session);
        Ok(format!(
            "已创建会话 #{slot_id}{label_line}\nAgent 会话 ID：{agent_session_id}\n工作目录：{work_dir}",
            label_line = if slot.label.is_empty() {
                String::new()
            } else {
                format!("（{}）", slot.label)
            },
            work_dir = self.effective_work_dir(platform, &message.session_key)?
        ))
    }

    fn cmd_list(&self, platform: &str, session_key: &str) -> Result<String> {
        self.ensure_conversation_slots(platform, session_key)?;
        let slots = self
            .store
            .list_conversation_slots(&self.project, platform, session_key)?;
        if slots.is_empty() {
            return Ok("当前没有已保存的会话。发送 /new 创建。".to_string());
        }
        let mut lines = vec!["会话列表：".to_string()];
        for slot in slots {
            let marker = if slot.is_active { "*" } else { " " };
            let label = if slot.label.is_empty() {
                String::new()
            } else {
                format!(" — {}", slot.label)
            };
            let agent_id = if slot.agent_session_id.is_empty() {
                "(未连接)".to_string()
            } else {
                slot.agent_session_id.clone()
            };
            lines.push(format!(
                "{marker} #{slot_id}{label} — {agent_id}",
                marker = marker,
                slot_id = slot.slot_id,
                label = label,
                agent_id = agent_id
            ));
        }
        lines.push("使用 /switch <id> 切换。".to_string());
        Ok(lines.join("\n"))
    }

    async fn cmd_switch(&self, platform: &str, message: &Message, slot_id: &str) -> Result<String> {
        self.ensure_conversation_slots(platform, &message.session_key)?;
        let slot = self
            .find_slot(platform, &message.session_key, slot_id)?
            .ok_or_else(|| anyhow!("未找到会话 #{slot_id}"))?;
        self.drop_runtime_session(platform, &message.session_key)
            .await;
        self.store.set_active_conversation_slot(
            &self.project,
            platform,
            &message.session_key,
            &slot.slot_id,
        )?;
        if !slot.agent_session_id.is_empty() {
            self.store.upsert_session(
                &self.project,
                platform,
                &message.session_key,
                self.agent.name(),
                &slot.agent_session_id,
            )?;
        }
        Ok(format!(
            "已切换到会话 #{}。下一条消息将使用该会话。",
            slot.slot_id
        ))
    }

    fn cmd_current(&self, platform: &str, session_key: &str) -> Result<String> {
        self.ensure_conversation_slots(platform, session_key)?;
        let slot = self
            .store
            .get_active_conversation_slot(&self.project, platform, session_key)?
            .ok_or_else(|| anyhow!("当前没有活动会话，请 /new 创建"))?;
        let work_dir = self.effective_work_dir(platform, session_key)?;
        Ok(format!(
            "当前会话：#{slot_id}{label}\nAgent 会话 ID：{agent_id}\n工作目录：{work_dir}",
            slot_id = slot.slot_id,
            label = if slot.label.is_empty() {
                String::new()
            } else {
                format!("（{}）", slot.label)
            },
            agent_id = if slot.agent_session_id.is_empty() {
                "(尚未启动)".to_string()
            } else {
                slot.agent_session_id
            },
            work_dir = work_dir
        ))
    }

    async fn cmd_dir(
        &self,
        platform: &str,
        session_key: &str,
        command: DirCommand,
    ) -> Result<String> {
        let mut prefs = self
            .store
            .get_conversation_prefs(&self.project, platform, session_key)?;
        let base = self.default_work_dir.clone();
        let current = self.effective_work_dir(platform, session_key)?;

        let reply = match command {
            DirCommand::Show => {
                let mut lines = vec![
                    format!("当前工作目录：{}", path_display(Path::new(&current))),
                    format!("配置默认目录：{}", path_display(Path::new(&base))),
                ];
                if !prefs.work_dir_history.is_empty() {
                    lines.push("目录历史：".to_string());
                    for (index, dir) in prefs.work_dir_history.iter().enumerate() {
                        lines.push(format!("  {} — {}", index + 1, dir));
                    }
                }
                lines.join("\n")
            }
            DirCommand::Reset => {
                prefs.work_dir_override = None;
                self.store.save_conversation_prefs(&prefs)?;
                format!(
                    "已恢复为配置默认工作目录：{}",
                    path_display(Path::new(&base))
                )
            }
            DirCommand::Back => {
                let previous = prefs.work_dir_history.pop();
                self.store.save_conversation_prefs(&prefs)?;
                match previous {
                    Some(dir) => {
                        prefs.work_dir_override = Some(dir.clone());
                        self.store.save_conversation_prefs(&prefs)?;
                        format!("已返回：{}", dir)
                    }
                    None => "没有更早的目录记录。".to_string(),
                }
            }
            DirCommand::Set(value) => {
                if let Ok(index) = value.parse::<usize>() {
                    if index == 0 || index > prefs.work_dir_history.len() {
                        return Err(anyhow!("历史目录序号无效：{value}"));
                    }
                    let dir = prefs.work_dir_history[index - 1].clone();
                    prefs.work_dir_override = Some(dir.clone());
                    self.store.save_conversation_prefs(&prefs)?;
                    format!("已切换到历史目录：{dir}")
                } else {
                    let resolved = path_display(&resolve_work_dir_path(&value, &current)?);
                    if prefs.work_dir_override.as_deref() != Some(resolved.as_str()) {
                        if let Some(prev) = prefs.work_dir_override.clone() {
                            prefs.work_dir_history.push(prev);
                        }
                    }
                    prefs.work_dir_override = Some(resolved.clone());
                    self.store.save_conversation_prefs(&prefs)?;
                    format!("工作目录已设为：{resolved}\n下次 /new 或重新连接 Agent 时生效。")
                }
            }
        };
        Ok(reply)
    }

    fn find_slot<'a>(
        &self,
        platform: &str,
        session_key: &str,
        slot_id: &str,
    ) -> Result<Option<ConversationSlot>> {
        let slots = self
            .store
            .list_conversation_slots(&self.project, platform, session_key)?;
        if let Some(slot) = slots.iter().find(|slot| slot.slot_id == slot_id).cloned() {
            return Ok(Some(slot));
        }
        Ok(slots
            .into_iter()
            .find(|slot| slot.slot_id.starts_with(slot_id)))
    }

}
