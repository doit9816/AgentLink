use super::models::{
    ApprovalRecord, ConversationPrefs, ConversationSlot, SessionRecord, TargetRecord,
};
use super::time::unix_now;
use super::SessionStore;
use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use serde_json;

impl SessionStore {
    pub(crate) fn migrate(&self) -> Result<()> {
        self.conn.lock().unwrap().execute_batch(
            r#"
CREATE TABLE IF NOT EXISTS sessions (
  project TEXT NOT NULL,
  platform TEXT NOT NULL,
  session_key TEXT NOT NULL,
  agent_type TEXT NOT NULL,
  agent_session_id TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(project, platform, session_key)
);
CREATE TABLE IF NOT EXISTS approvals (
  approval_id TEXT PRIMARY KEY,
  project TEXT NOT NULL,
  platform TEXT NOT NULL,
  session_key TEXT NOT NULL,
  agent_session_id TEXT NOT NULL,
  agent_request_id TEXT NOT NULL,
  status TEXT NOT NULL,
  summary TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  resolved_at INTEGER,
  decision TEXT
);
CREATE TABLE IF NOT EXISTS targets (
  project TEXT NOT NULL,
  platform TEXT NOT NULL,
  session_key TEXT NOT NULL,
  user_id TEXT NOT NULL,
  user_name TEXT,
  message_id TEXT,
  reply_ctx TEXT NOT NULL,
  content_preview TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(project, platform, session_key)
);
CREATE TABLE IF NOT EXISTS conversation_slots (
  project TEXT NOT NULL,
  platform TEXT NOT NULL,
  session_key TEXT NOT NULL,
  slot_id TEXT NOT NULL,
  label TEXT NOT NULL DEFAULT '',
  agent_session_id TEXT NOT NULL DEFAULT '',
  is_active INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(project, platform, session_key, slot_id)
);
CREATE TABLE IF NOT EXISTS conversation_prefs (
  project TEXT NOT NULL,
  platform TEXT NOT NULL,
  session_key TEXT NOT NULL,
  work_dir_override TEXT,
  work_dir_history TEXT NOT NULL DEFAULT '[]',
  PRIMARY KEY(project, platform, session_key)
);
"#,
        )?;
        Ok(())
    }

    pub fn record_target(&self, record: &TargetRecord) -> Result<()> {
        let now = unix_now();
        self.conn.lock().unwrap().execute(
            r#"
INSERT INTO targets(project, platform, session_key, user_id, user_name, message_id, reply_ctx, content_preview, created_at, updated_at)
VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
ON CONFLICT(project, platform, session_key)
DO UPDATE SET user_id = excluded.user_id,
              user_name = excluded.user_name,
              message_id = excluded.message_id,
              reply_ctx = excluded.reply_ctx,
              content_preview = excluded.content_preview,
              updated_at = excluded.updated_at
"#,
            params![
                record.project,
                record.platform,
                record.session_key,
                record.user_id,
                record.user_name,
                record.message_id,
                record.reply_ctx,
                record.content_preview,
                now
            ],
        )?;
        Ok(())
    }

    pub fn list_targets(
        &self,
        project: &str,
        platform: &str,
        limit: usize,
    ) -> Result<Vec<TargetRecord>> {
        let limit = limit.clamp(1, 500) as i64;
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            r#"
SELECT project, platform, session_key, user_id, user_name, message_id, reply_ctx, content_preview, updated_at
FROM targets
WHERE project = ?1 AND platform = ?2
ORDER BY updated_at DESC
LIMIT ?3
"#,
        )?;
        let rows = stmt.query_map(params![project, platform, limit], |row| {
            Ok(TargetRecord {
                project: row.get(0)?,
                platform: row.get(1)?,
                session_key: row.get(2)?,
                user_id: row.get(3)?,
                user_name: row.get(4)?,
                message_id: row.get(5)?,
                reply_ctx: row.get(6)?,
                content_preview: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn upsert_session(
        &self,
        project: &str,
        platform: &str,
        session_key: &str,
        agent_type: &str,
        agent_session_id: &str,
    ) -> Result<()> {
        let now = unix_now();
        self.conn.lock().unwrap().execute(
            r#"
INSERT INTO sessions(project, platform, session_key, agent_type, agent_session_id, created_at, updated_at)
VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?6)
ON CONFLICT(project, platform, session_key)
DO UPDATE SET agent_type = excluded.agent_type, agent_session_id = excluded.agent_session_id, updated_at = excluded.updated_at
"#,
            params![project, platform, session_key, agent_type, agent_session_id, now],
        )?;
        Ok(())
    }

    pub fn get_session(
        &self,
        project: &str,
        platform: &str,
        session_key: &str,
    ) -> Result<Option<SessionRecord>> {
        let record = self
            .conn
            .lock()
            .unwrap()
            .query_row(
                r#"
SELECT project, platform, session_key, agent_type, agent_session_id
FROM sessions
WHERE project = ?1 AND platform = ?2 AND session_key = ?3
"#,
                params![project, platform, session_key],
                |row| {
                    Ok(SessionRecord {
                        project: row.get(0)?,
                        platform: row.get(1)?,
                        session_key: row.get(2)?,
                        agent_type: row.get(3)?,
                        agent_session_id: row.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(record)
    }

    pub fn create_approval(&self, record: &ApprovalRecord) -> Result<()> {
        self.conn.lock().unwrap().execute(
            r#"
INSERT INTO approvals(approval_id, project, platform, session_key, agent_session_id, agent_request_id, status, summary, expires_at, created_at)
VALUES(?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7, ?8, ?9)
"#,
            params![
                record.approval_id,
                record.project,
                record.platform,
                record.session_key,
                record.agent_session_id,
                record.agent_request_id,
                record.summary,
                record.expires_at,
                unix_now()
            ],
        )?;
        Ok(())
    }

    pub fn get_approval(&self, approval_id: &str) -> Result<Option<ApprovalRecord>> {
        let record = self
            .conn
            .lock()
            .unwrap()
            .query_row(
                r#"
SELECT approval_id, project, platform, session_key, agent_session_id, agent_request_id, status, summary, expires_at, decision
FROM approvals
WHERE approval_id = ?1
"#,
                params![approval_id],
                |row| {
                    Ok(ApprovalRecord {
                        approval_id: row.get(0)?,
                        project: row.get(1)?,
                        platform: row.get(2)?,
                        session_key: row.get(3)?,
                        agent_session_id: row.get(4)?,
                        agent_request_id: row.get(5)?,
                        status: row.get(6)?,
                        summary: row.get(7)?,
                        expires_at: row.get(8)?,
                        decision: row.get(9)?,
                    })
                },
            )
            .optional()?;
        Ok(record)
    }

    pub fn resolve_approval(&self, approval_id: &str, decision: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            r#"
UPDATE approvals
SET status = 'resolved', resolved_at = ?1, decision = ?2
WHERE approval_id = ?3 AND status = 'pending'
"#,
            params![unix_now(), decision, approval_id],
        )?;
        Ok(())
    }

    pub fn list_conversation_slots(
        &self,
        project: &str,
        platform: &str,
        session_key: &str,
    ) -> Result<Vec<ConversationSlot>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            r#"
SELECT project, platform, session_key, slot_id, label, agent_session_id, is_active, created_at, updated_at
FROM conversation_slots
WHERE project = ?1 AND platform = ?2 AND session_key = ?3
ORDER BY CAST(slot_id AS INTEGER), slot_id
"#,
        )?;
        let rows = stmt.query_map(params![project, platform, session_key], |row| {
            Ok(ConversationSlot {
                project: row.get(0)?,
                platform: row.get(1)?,
                session_key: row.get(2)?,
                slot_id: row.get(3)?,
                label: row.get(4)?,
                agent_session_id: row.get(5)?,
                is_active: row.get::<_, i64>(6)? != 0,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_active_conversation_slot(
        &self,
        project: &str,
        platform: &str,
        session_key: &str,
    ) -> Result<Option<ConversationSlot>> {
        let slots = self.list_conversation_slots(project, platform, session_key)?;
        Ok(slots.into_iter().find(|slot| slot.is_active))
    }

    pub fn get_conversation_slot(
        &self,
        project: &str,
        platform: &str,
        session_key: &str,
        slot_id: &str,
    ) -> Result<Option<ConversationSlot>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            r#"
SELECT project, platform, session_key, slot_id, label, agent_session_id, is_active, created_at, updated_at
FROM conversation_slots
WHERE project = ?1 AND platform = ?2 AND session_key = ?3 AND slot_id = ?4
"#,
            params![project, platform, session_key, slot_id],
            |row| {
                Ok(ConversationSlot {
                    project: row.get(0)?,
                    platform: row.get(1)?,
                    session_key: row.get(2)?,
                    slot_id: row.get(3)?,
                    label: row.get(4)?,
                    agent_session_id: row.get(5)?,
                    is_active: row.get::<_, i64>(6)? != 0,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub fn next_conversation_slot_id(
        &self,
        project: &str,
        platform: &str,
        session_key: &str,
    ) -> Result<String> {
        let slots = self.list_conversation_slots(project, platform, session_key)?;
        let max = slots
            .iter()
            .filter_map(|slot| slot.slot_id.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        Ok((max + 1).to_string())
    }

    pub fn insert_conversation_slot(&self, slot: &ConversationSlot) -> Result<()> {
        self.conn.lock().unwrap().execute(
            r#"
INSERT INTO conversation_slots(
  project, platform, session_key, slot_id, label, agent_session_id, is_active, created_at, updated_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
"#,
            params![
                slot.project,
                slot.platform,
                slot.session_key,
                slot.slot_id,
                slot.label,
                slot.agent_session_id,
                i64::from(slot.is_active),
                slot.created_at,
                slot.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn set_active_conversation_slot(
        &self,
        project: &str,
        platform: &str,
        session_key: &str,
        slot_id: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
UPDATE conversation_slots
SET is_active = 0
WHERE project = ?1 AND platform = ?2 AND session_key = ?3
"#,
            params![project, platform, session_key],
        )?;
        conn.execute(
            r#"
UPDATE conversation_slots
SET is_active = 1, updated_at = ?5
WHERE project = ?1 AND platform = ?2 AND session_key = ?3 AND slot_id = ?4
"#,
            params![project, platform, session_key, slot_id, unix_now()],
        )?;
        Ok(())
    }

    pub fn update_conversation_slot_agent_id(
        &self,
        project: &str,
        platform: &str,
        session_key: &str,
        slot_id: &str,
        agent_session_id: &str,
    ) -> Result<()> {
        self.conn.lock().unwrap().execute(
            r#"
UPDATE conversation_slots
SET agent_session_id = ?5, updated_at = ?6
WHERE project = ?1 AND platform = ?2 AND session_key = ?3 AND slot_id = ?4
"#,
            params![
                project,
                platform,
                session_key,
                slot_id,
                agent_session_id,
                unix_now()
            ],
        )?;
        Ok(())
    }

    pub fn get_conversation_prefs(
        &self,
        project: &str,
        platform: &str,
        session_key: &str,
    ) -> Result<ConversationPrefs> {
        let record = self
            .conn
            .lock()
            .unwrap()
            .query_row(
                r#"
SELECT work_dir_override, work_dir_history
FROM conversation_prefs
WHERE project = ?1 AND platform = ?2 AND session_key = ?3
"#,
                params![project, platform, session_key],
                |row| {
                    let history_raw: String = row.get(1)?;
                    let work_dir_history: Vec<String> =
                        serde_json::from_str(&history_raw).unwrap_or_default();
                    Ok(ConversationPrefs {
                        project: project.to_string(),
                        platform: platform.to_string(),
                        session_key: session_key.to_string(),
                        work_dir_override: row.get(0)?,
                        work_dir_history,
                    })
                },
            )
            .optional()?;
        if let Some(prefs) = record {
            return Ok(prefs);
        }
        Ok(ConversationPrefs {
            project: project.to_string(),
            platform: platform.to_string(),
            session_key: session_key.to_string(),
            work_dir_override: None,
            work_dir_history: Vec::new(),
        })
    }

    pub fn save_conversation_prefs(&self, prefs: &ConversationPrefs) -> Result<()> {
        let history = serde_json::to_string(&prefs.work_dir_history)?;
        self.conn.lock().unwrap().execute(
            r#"
INSERT INTO conversation_prefs(project, platform, session_key, work_dir_override, work_dir_history)
VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT(project, platform, session_key)
DO UPDATE SET work_dir_override = excluded.work_dir_override,
              work_dir_history = excluded.work_dir_history
"#,
            params![
                prefs.project,
                prefs.platform,
                prefs.session_key,
                prefs.work_dir_override,
                history,
            ],
        )?;
        Ok(())
    }
}
