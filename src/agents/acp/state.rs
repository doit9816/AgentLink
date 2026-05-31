use serde::Deserialize;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub(super) struct AcpModesState {
    pub available: Vec<AcpModeInfo>,
    pub current_mode_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct AcpModeInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct AcpModesBlock {
    #[serde(rename = "currentModeId", default)]
    pub current_mode_id: String,
    #[serde(rename = "availableModes", default)]
    pub available_modes: Vec<AcpModeInfo>,
}

#[derive(Debug, Default)]
pub(super) struct AcpAgentState {
    pub modes: Mutex<AcpModesState>,
    pub list_unsupported: AtomicBool,
}

impl AcpAgentState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn report_modes(self: &Arc<Self>, block: &AcpModesBlock) {
        if block.available_modes.is_empty() {
            return;
        }
        let mut guard = self.modes.lock().unwrap();
        guard.available = block.available_modes.clone();
        if !block.current_mode_id.is_empty() {
            guard.current_mode_id = block.current_mode_id.clone();
        }
    }

    pub fn match_mode_id(&self, input: &str) -> String {
        let input = input.trim();
        if input.is_empty() {
            return String::new();
        }
        let lower = input.to_ascii_lowercase();
        let guard = self.modes.lock().unwrap();
        for mode in &guard.available {
            if mode.id.to_ascii_lowercase() == lower || mode.name.to_ascii_lowercase() == lower {
                return mode.id.clone();
            }
        }
        String::new()
    }
}
