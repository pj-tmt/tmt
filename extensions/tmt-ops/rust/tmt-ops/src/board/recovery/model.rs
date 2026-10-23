//! Private projections of the public workspace JSON. Core alone owns recovery.
use crate::core::SquadError;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecoverySnapshotSummary {
    pub name: Option<String>,
    pub digest: String,
    pub captured_at_ms: u64,
}
impl RecoverySnapshotSummary {
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or("Latest")
    }
    pub fn id(&self) -> String {
        format!("snapshot:{}", self.name.as_deref().unwrap_or("latest"))
    }
}
#[derive(Debug)]
pub(super) struct Inventory {
    pub version: u64,
    pub socket: String,
    pub snapshots: Vec<RecoverySnapshotSummary>,
}
#[derive(Clone, Debug)]
pub(super) struct Preview {
    pub version: u64,
    pub name: Option<String>,
    pub digest: String,
    pub snapshot: Topology,
    pub sessions: Vec<SessionPlan>,
    pub panes: Vec<PanePlan>,
}
#[derive(Clone, Debug)]
pub(super) struct Topology {
    pub version: u64,
    pub captured_at_ms: u64,
    pub server: Server,
    pub sessions: Vec<Session>,
    pub windows: Vec<Window>,
    pub panes: Vec<Pane>,
}
#[derive(Clone, Debug)]
pub(super) struct Server {
    pub socket: String,
}
#[derive(Clone, Debug)]
pub(super) struct Session {
    pub id: String,
    pub name: String,
    pub windows: Vec<Link>,
}
#[derive(Clone, Debug)]
pub(super) struct Link {
    pub index: u64,
    pub window: String,
    pub active: bool,
}
#[derive(Clone, Debug)]
pub(super) struct Window {
    pub id: String,
    pub name: String,
    pub layout: String,
    pub visible_layout: String,
    pub width: u64,
    pub height: u64,
    pub active_pane: String,
}
#[derive(Clone, Debug)]
pub(super) struct Pane {
    pub id: String,
    pub window: String,
    pub index: u64,
    pub left: u64,
    pub top: u64,
    pub width: u64,
    pub height: u64,
    pub cwd: String,
    pub identity: Option<RecoveryIdentityEvidence>,
    pub command: Option<Command>,
}
#[derive(Clone, Debug)]
pub(super) struct RecoveryIdentityEvidence {
    pub id: String,
    pub name: String,
    pub lifetime: String,
}
#[derive(Clone, Debug)]
pub(super) struct Command {
    pub argv: Vec<String>,
}
#[derive(Clone, Debug)]
pub(super) struct SessionPlan {
    pub session: String,
    pub action: String,
}
#[derive(Clone, Debug)]
pub(super) struct PanePlan {
    pub pane: String,
    pub action: String,
    pub identity: Option<RecoveryIdentityEvidence>,
}
#[derive(Clone, Debug)]
pub(super) struct Restore {
    pub version: u64,
    pub socket: String,
    pub name: Option<String>,
    pub digest: String,
    pub layout_only: bool,
    pub status: String,
    pub sessions: Vec<SessionResult>,
    pub windows: Vec<Mapping>,
    pub panes: Vec<Mapping>,
    pub failures: Vec<String>,
    pub revival: Vec<Revival>,
}
#[derive(Clone, Debug)]
pub(super) struct SessionResult {
    pub recorded: String,
    pub name: String,
    pub action: String,
    pub native: Option<String>,
    /// The live session's name when it is not the recorded one.
    pub restored_as: Option<String>,
    pub retained_bootstrap: Option<String>,
}
#[derive(Clone, Debug)]
pub(super) struct Mapping {
    pub recorded: String,
    pub native: String,
}
#[derive(Clone, Debug)]
pub(super) struct Revival {
    pub recorded: String,
    pub native: Option<String>,
    pub name: Option<String>,
    pub status: String,
    pub message: String,
}

pub(super) fn invalid() -> SquadError {
    SquadError::new(
        "SQUAD_WORKSPACE_INVALID",
        "Workspace returned an unsupported or inconsistent document.",
    )
}
pub(super) fn valid_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name != "latest"
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}
pub(super) fn valid_socket(socket: &str) -> bool {
    std::path::Path::new(socket).is_absolute() && !socket.contains('\0')
}
/// The manual saves `tmt workspace save` keeps, and the unnamed rows `ls` lists beside them: the
/// retained generations (or the legacy latest). Counted apart, since unnamed ones are many.
const MAX_NAMED: usize = 16;
const MAX_UNNAMED: usize = 35;
impl Inventory {
    pub fn validate(&self, socket: Option<&str>) -> Result<(), SquadError> {
        let mut names = BTreeSet::new();
        if self.version != 1
            || !valid_socket(&self.socket)
            || socket.is_some_and(|s| s != self.socket)
            || self.snapshots.iter().filter(|e| e.name.is_some()).count() > MAX_NAMED
            || self.snapshots.iter().filter(|e| e.name.is_none()).count() > MAX_UNNAMED
            || self.snapshots.iter().any(|e| {
                !valid_digest(&e.digest)
                    || e.name.as_deref().is_some_and(|s| !valid_name(s))
                    // Unnamed generations are many and share no name; only real names are unique.
                    || e.name.as_ref().is_some_and(|name| !names.insert(name.clone()))
            })
        {
            return Err(invalid());
        }
        Ok(())
    }
}
impl Preview {
    pub fn validate(&self, socket: &str, name: Option<&str>) -> Result<(), SquadError> {
        let s = &self.snapshot;
        let sessions: BTreeSet<_> = s.sessions.iter().map(|s| s.id.as_str()).collect();
        let windows: BTreeSet<_> = s.windows.iter().map(|w| w.id.as_str()).collect();
        let panes: BTreeSet<_> = s.panes.iter().map(|p| p.id.as_str()).collect();
        if self.version != 1
            || s.version != 1
            || self.name.as_deref() != name
            || s.server.socket != socket
            || !valid_digest(&self.digest)
            || s.sessions.len() > 1024
            || s.windows.len() > 1024
            || s.panes.len() > 1024
            || sessions.len() != s.sessions.len()
            || windows.len() != s.windows.len()
            || panes.len() != s.panes.len()
            || self.sessions.len() != sessions.len()
            || self.panes.len() != panes.len()
            || self
                .sessions
                .iter()
                .map(|p| p.session.as_str())
                .collect::<BTreeSet<_>>()
                != sessions
            || self
                .panes
                .iter()
                .map(|p| p.pane.as_str())
                .collect::<BTreeSet<_>>()
                != panes
            || self
                .sessions
                .iter()
                .any(|p| !matches!(p.action.as_str(), "create" | "name_in_use" | "unknown"))
            || self.panes.iter().any(|p| {
                !matches!(
                    p.action.as_str(),
                    "shell"
                        | "relaunch_command"
                        | "identity_missing"
                        | "no_remembered_session"
                        | "resumable"
                        | "stale_requires_retry"
                        | "floating"
                )
            })
            || s.sessions.iter().any(|s| {
                s.windows
                    .iter()
                    .any(|l| !windows.contains(l.window.as_str()))
            })
            || s.windows
                .iter()
                .any(|w| !panes.contains(w.active_pane.as_str()))
            || s.panes.iter().any(|p| !windows.contains(p.window.as_str()))
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn counts(&self) -> String {
        let count = |action| self.sessions.iter().filter(|s| s.action == action).count();
        format!(
            "{} sessions · {} create · {} name in use · {} unknown · {} windows · {} panes",
            self.snapshot.sessions.len(),
            count("create"),
            count("name_in_use"),
            count("unknown"),
            self.snapshot.windows.len(),
            self.snapshot.panes.len()
        )
    }
}

// The extension already owns serde_json; no new dependency or Core type edge.
pub(super) trait Decode: Sized {
    fn decode(value: &serde_json::Value) -> Result<Self, SquadError>;
}
impl Decode for String {
    fn decode(v: &serde_json::Value) -> Result<Self, SquadError> {
        v.as_str()
            .filter(|s| s.len() <= 32 * 1024)
            .map(str::to_owned)
            .ok_or_else(invalid)
    }
}
impl Decode for u64 {
    fn decode(v: &serde_json::Value) -> Result<Self, SquadError> {
        v.as_u64().ok_or_else(invalid)
    }
}
impl Decode for bool {
    fn decode(v: &serde_json::Value) -> Result<Self, SquadError> {
        v.as_bool().ok_or_else(invalid)
    }
}
impl<T: Decode> Decode for Option<T> {
    fn decode(v: &serde_json::Value) -> Result<Self, SquadError> {
        if v.is_null() {
            Ok(None)
        } else {
            T::decode(v).map(Some)
        }
    }
}
impl<T: Decode> Decode for Vec<T> {
    fn decode(v: &serde_json::Value) -> Result<Self, SquadError> {
        v.as_array()
            .filter(|a| a.len() <= 1024)
            .ok_or_else(invalid)?
            .iter()
            .map(T::decode)
            .collect()
    }
}
macro_rules! document {
    ($ty:ident { $($field:ident: $key:literal),* $(,)? }) => {
        impl Decode for $ty {
            fn decode(v: &serde_json::Value) -> Result<Self, SquadError> {
                Ok(Self { $($field: Decode::decode(v.get($key).ok_or_else(invalid)?)?),* })
            }
        }
    };
}
document!(RecoverySnapshotSummary {
    name: "name",
    digest: "digest",
    captured_at_ms: "capturedAtMs"
});
document!(Inventory {
    version: "version",
    socket: "socket",
    snapshots: "snapshots"
});
document!(Preview {
    version: "version",
    name: "name",
    digest: "digest",
    snapshot: "snapshot",
    sessions: "sessions",
    panes: "panes"
});
document!(Topology {
    version: "version",
    captured_at_ms: "capturedAtMs",
    server: "server",
    sessions: "sessions",
    windows: "windows",
    panes: "panes"
});
document!(Server { socket: "socket" });
document!(Session {
    id: "id",
    name: "name",
    windows: "windows"
});
document!(Link {
    index: "index",
    window: "window",
    active: "active"
});
document!(Window {
    id: "id",
    name: "name",
    layout: "layout",
    visible_layout: "visibleLayout",
    width: "width",
    height: "height",
    active_pane: "activePane"
});
document!(Pane {
    id: "id",
    window: "window",
    index: "index",
    left: "left",
    top: "top",
    width: "width",
    height: "height",
    cwd: "cwd",
    identity: "identity",
    command: "command"
});
document!(RecoveryIdentityEvidence {
    id: "id",
    name: "name",
    lifetime: "lifetime"
});
document!(Command { argv: "argv" });
document!(SessionPlan {
    session: "session",
    action: "action"
});
document!(PanePlan {
    pane: "pane",
    action: "action",
    identity: "identity"
});
impl Decode for SessionResult {
    fn decode(v: &serde_json::Value) -> Result<Self, SquadError> {
        let field = |key| v.get(key).ok_or_else(invalid);
        Ok(Self {
            recorded: Decode::decode(field("recorded")?)?,
            name: Decode::decode(field("name")?)?,
            action: Decode::decode(field("action")?)?,
            native: Decode::decode(field("native")?)?,
            // Absent from an older tmt, which only ever restored under the recorded name.
            restored_as: v
                .get("restoredAs")
                .map(Decode::decode)
                .transpose()?
                .flatten(),
            retained_bootstrap: Decode::decode(field("retainedBootstrap")?)?,
        })
    }
}
document!(Mapping {
    recorded: "recorded",
    native: "native"
});
document!(Revival {
    recorded: "recorded",
    native: "native",
    name: "name",
    status: "status",
    message: "message"
});
impl Decode for Restore {
    fn decode(v: &serde_json::Value) -> Result<Self, SquadError> {
        let field = |key| v.get(key).ok_or_else(invalid);
        Ok(Self {
            version: Decode::decode(field("version")?)?,
            socket: Decode::decode(field("socket")?)?,
            name: Decode::decode(field("name")?)?,
            digest: Decode::decode(field("digest")?)?,
            layout_only: Decode::decode(field("layoutOnly")?)?,
            status: Decode::decode(field("status")?)?,
            sessions: Decode::decode(field("sessions")?)?,
            windows: Decode::decode(field("windows")?)?,
            panes: Decode::decode(field("panes")?)?,
            failures: Decode::decode(field("failures")?)?,
            revival: v
                .get("revival")
                .map(Decode::decode)
                .transpose()?
                .unwrap_or_default(),
        })
    }
}
