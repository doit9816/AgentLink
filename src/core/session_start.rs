#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionStartRequest {
    pub resume_session_id: Option<String>,
    pub work_dir: Option<String>,
}

impl SessionStartRequest {
    pub fn resume(session_id: Option<String>) -> Self {
        Self {
            resume_session_id: session_id,
            work_dir: None,
        }
    }

    pub fn fresh(work_dir: Option<String>) -> Self {
        Self {
            resume_session_id: None,
            work_dir,
        }
    }
}
