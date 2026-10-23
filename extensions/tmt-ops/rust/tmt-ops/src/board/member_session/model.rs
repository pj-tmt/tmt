//! Read-only public observations. Remembered coordinates never grant launch authority.
use crate::board::{app::App, workspace::OpsTarget};
use serde_json::Value;

/// Stable session observations fence the task; changing activity is painted live.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::board) struct Evidence {
    pub host: Option<String>,
    pub host_session: Option<String>,
    pub window: Option<String>,
    pub driver: Option<String>,
    pub session: Option<String>,
    pub stale: Option<bool>,
    pub unbound: Option<bool>,
    pub support: Option<String>,
    pub error: Option<String>,
}
fn text(value: &Value) -> Option<String> {
    value.as_str().filter(|v| !v.is_empty()).map(str::to_owned)
}
impl Evidence {
    pub fn from_agent(agent: Option<&crate::board::inventory::Agent>, error: Option<&str>) -> Self {
        let Some(agent) = agent else {
            return Self {
                error: Some(error.unwrap_or("Member observation unavailable").into()),
                ..Self::default()
            };
        };
        let v = &agent.parameters;
        Self {
            host: text(&v["location"]["host"]),
            host_session: text(&v["location"]["session"]),
            window: text(&v["location"]["window"]),
            driver: text(&v["resume"]["driver"]),
            session: text(&v["resume"]["session"]),
            stale: match v["resume"].get("staleAtMs") {
                Some(Value::Null) => Some(false),
                Some(value) if value.as_u64().is_some() => Some(true),
                _ => None,
            },
            support: text(&v["driverSupport"]),
            unbound: v["session"]["unbound"].as_bool(),
            error: error.map(str::to_owned),
        }
    }
    pub fn remembered(&self) -> bool {
        self.error.is_none()
            && self.support.as_deref() == Some("full")
            && self.stale == Some(false)
            && self.driver.is_some()
            && self.session.is_some()
    }
    pub fn is_unbound(&self) -> bool {
        self.error.is_none() && self.unbound == Some(true)
    }
    pub fn compact(&self) -> String {
        if self.is_unbound() {
            return format!(
                "{} unbound · {}",
                tmt_cli_style::mark::Mark::Warning.symbol(),
                self.host.as_deref().unwrap_or("—")
            );
        }
        format!(
            "{} · {}",
            self.host.as_deref().unwrap_or("—"),
            if self.remembered() {
                "remembered"
            } else {
                "unavailable"
            }
        )
    }
    pub fn lines(&self, now_ms: u64, last_ms: Option<u64>) -> Vec<String> {
        let mut lines = vec![format!(
            "Host: {}",
            self.host.as_deref().unwrap_or("unavailable")
        )];
        if let Some(session) = &self.host_session {
            lines.push(format!("Host session: {session}"));
        }
        if let Some(window) = &self.window {
            lines.push(format!("Host window: {window}"));
        }
        let status = if self.error.is_some() {
            "Session unavailable · observation failed"
        } else if self.is_unbound() {
            "Session unbound · the provider session is not bound to this identity"
        } else if self.remembered() {
            "Session remembered · checked on launch"
        } else if self.stale == Some(true) {
            "Session unavailable · remembered session is stale"
        } else if self.support.as_deref() == Some("basic") {
            "Session unavailable · basic driver"
        } else {
            "Session unavailable · no admitted remembered session"
        };
        lines.push(status.into());
        if self.is_unbound() && !self.remembered() {
            lines.push(
                "Copy unavailable · needs a fresh remembered session from a full driver.".into(),
            );
        }
        if let Some(driver) = &self.driver {
            lines.push(format!("Remembered driver: {driver}"));
        }
        if let Some(session) = &self.session {
            lines.push(format!("Remembered session: {session}"));
        }
        lines.push(format!(
            "Last activity: {}",
            last_ms.map_or_else(
                || "unavailable".into(),
                |at| format!("{} ago", crate::requests::age(now_ms, at))
            )
        ));
        if let Some(error) = &self.error {
            lines.push(format!("Observation unavailable: {error}"));
        }
        lines
    }
}
impl App {
    pub(in crate::board) fn member_session_evidence(&self, id: &str) -> Evidence {
        Evidence::from_agent(
            self.references.agents.get(id),
            self.references.error.as_deref(),
        )
    }
    pub(in crate::board) fn member_last_activity(&self, id: &str) -> Option<u64> {
        self.workspace_snapshot()
            .and_then(|s| s.roster.members.get(id))
            .and_then(|m| m.activity.last_ms)
    }
    pub(super) fn session_target_matches(&self, target: &OpsTarget, evidence: &Evidence) -> bool {
        let OpsTarget::Member { identity_id, .. } = target else {
            return false;
        };
        self.workspace_selected_member() == Some(target)
            && self.member_session_evidence(identity_id) == *evidence
    }
}
