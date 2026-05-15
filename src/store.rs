use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct SessionStore {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub project: String,
    pub platform: String,
    pub session_key: String,
    pub agent_type: String,
    pub agent_session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRecord {
    pub approval_id: String,
    pub project: String,
    pub platform: String,
    pub session_key: String,
    pub agent_session_id: String,
    pub agent_request_id: String,
    pub status: String,
    pub summary: String,
    pub expires_at: u64,
    pub decision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetRecord {
    pub project: String,
    pub platform: String,
    pub session_key: String,
    pub user_id: String,
    pub user_name: Option<String>,
    pub message_id: Option<String>,
    pub reply_ctx: String,
    pub content_preview: String,
    pub updated_at: u64,
}

impl SessionStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.migrate()?;
        Ok(store)
    }

    pub fn in_memory() -> Result<Self> {
        let store = Self {
            conn: Arc::new(Mutex::new(Connection::open_in_memory()?)),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
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
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs()
}
