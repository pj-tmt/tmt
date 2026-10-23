//! Typed outcome of layout-only creation. Historical coordinates are mapping keys.

use serde::Serialize;

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutRestore {
    #[serde(skip)]
    pub server: Option<(tmt_core::endpoint::ProcessIncarnation, u64)>,
    pub sessions: Vec<RestoredSession>,
    pub windows: Vec<RestoredWindow>,
    pub panes: Vec<RestoredPane>,
    /// Panes opened in another directory because their recorded one is gone.
    pub fallbacks: Vec<CwdFallback>,
    pub failures: Vec<String>,
}

/// A recorded directory that no longer exists and the nearest existing parent used instead.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CwdFallback {
    /// The pane's recorded (historical) id.
    pub pane: String,
    pub wanted: String,
    pub used: String,
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
    /// `created`, `merged` (into the server's fresh default session), `skip_existing` (this
    /// snapshot's restore made it earlier), `partial` or `not_attempted`.
    pub action: String,
    pub native: Option<String>,
    /// The live session's name when it is not the recorded one: `<name>-restored` beside a
    /// session in use, or the fresh default session the windows were added to.
    pub restored_as: Option<String>,
    pub retained_bootstrap: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RestoredWindow {
    pub recorded: String,
    pub native: String,
}

#[derive(Debug, Serialize)]
pub struct RestoredPane {
    #[serde(skip)]
    pub shell: Option<tmt_core::endpoint::ProcessIncarnation>,
    pub recorded: String,
    pub native: String,
}
