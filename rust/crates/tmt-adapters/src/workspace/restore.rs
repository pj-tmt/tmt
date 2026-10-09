//! Typed outcome of layout-only creation. Historical coordinates are mapping keys.

use serde::Serialize;

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutRestore {
    pub sessions: Vec<RestoredSession>,
    pub windows: Vec<RestoredWindow>,
    pub panes: Vec<RestoredPane>,
    pub failures: Vec<String>,
}

impl LayoutRestore {
    pub fn partial(&self) -> bool {
        !self.failures.is_empty()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoredSession {
    pub recorded: String,
    pub name: String,
    pub action: String,
    pub native: Option<String>,
    pub retained_bootstrap: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RestoredWindow {
    pub recorded: String,
    pub native: String,
}

#[derive(Debug, Serialize)]
pub struct RestoredPane {
    pub recorded: String,
    pub native: String,
}
