//! Private projections of public discovery JSON; native admission stays in Core.
use crate::core::SquadError;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct PaneKey {
    pub host: String,
    pub socket: Option<String>,
    pub pane: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NativeObservation {
    pub pid: u64,
    pub start: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ExistingAgentEvidence {
    pub id: String,
    pub name: String,
    pub binding: Option<String>,
}
#[derive(Clone, Debug)]
pub(super) struct ObservedPane {
    pub key: PaneKey,
    pub server: Option<NativeObservation>,
    pub server_identity: Option<String>,
    pub native: Option<NativeObservation>,
    pub session: Option<String>,
    pub window: Option<String>,
    pub target: Option<String>,
    pub runtime: Option<String>,
    pub runtime_state: String,
    pub runtime_reason: Option<String>,
    pub recorded_state: Option<String>,
    pub identity: Option<ExistingAgentEvidence>,
    pub state: String,
    pub reason: Option<String>,
    pub duplicate: bool,
    pub title: String,
    pub command: String,
    pub cwd: String,
    pub text_truncated: bool,
    pub agent_name: Option<String>,
    pub adoptable: bool,
    pub expect: Option<String>,
}
#[derive(Clone, Debug)]
pub(super) struct SourceObservation {
    pub host: String,
    pub socket: Option<String>,
    pub status: String,
    pub reason: Option<String>,
}
#[derive(Clone, Debug)]
pub(super) struct ScanObservation {
    pub observed_at_ms: u64,
    pub sources: Vec<SourceObservation>,
    pub panes: Vec<ObservedPane>,
    pub truncated: bool,
}
pub(super) fn invalid() -> SquadError {
    SquadError::new(
        "SQUAD_DISCOVERY_INVALID",
        "tmt did not return a complete result; check the current state before deciding what to do next.",
    )
}
fn text(value: &Value) -> Result<String, SquadError> {
    value.as_str().map(str::to_owned).ok_or_else(invalid)
}
fn optional(value: &Value) -> Result<Option<String>, SquadError> {
    if value.is_null() {
        Ok(None)
    } else {
        text(value).map(Some)
    }
}
fn flag(value: &Value) -> Result<bool, SquadError> {
    value.as_bool().ok_or_else(invalid)
}
fn native(value: &Value) -> Result<Option<NativeObservation>, SquadError> {
    if value.is_null() {
        return Ok(None);
    }
    let pid = value["pid"]
        .as_u64()
        .filter(|pid| *pid > 0)
        .ok_or_else(invalid)?;
    let start = text(&value["start"])?;
    if start.is_empty() {
        return Err(invalid());
    }
    Ok(Some(NativeObservation { pid, start }))
}
pub(super) fn canonical_uuid(value: &str) -> bool {
    crate::config::uuid_like(value) && !value.bytes().any(|byte| byte.is_ascii_uppercase())
}
fn uuid(value: &Value) -> Result<String, SquadError> {
    let id = text(value)?;
    if !canonical_uuid(&id) {
        return Err(invalid());
    }
    Ok(id)
}
fn optional_uuid(value: &Value) -> Result<Option<String>, SquadError> {
    if value.is_null() {
        Ok(None)
    } else {
        uuid(value).map(Some)
    }
}
/// tmux pane keys are `%N`. Another host's key is in its driver's syntax,
/// which Core admits; Ops checks only a bounded printable literal of 1 to
/// 80 bytes that `tmt add` gets after `--`.
fn pane_key(host: &str, pane: &str) -> bool {
    if host == "tmux" {
        return pane
            .strip_prefix('%')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    }
    (1..=80).contains(&pane.len()) && pane.bytes().all(|b| b.is_ascii_graphic())
}
fn digest(value: &Value) -> Result<Option<String>, SquadError> {
    let result = optional(value)?;
    if result.as_ref().is_some_and(|s| {
        s.len() != 64
            || !s
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        return Err(invalid());
    }
    Ok(result)
}
impl ScanObservation {
    pub fn decode(value: &Value) -> Result<Self, SquadError> {
        if value["version"] != 1 {
            return Err(invalid());
        }
        let observed_at_ms = value["observedAtMs"].as_u64().ok_or_else(invalid)?;
        let truncated = flag(&value["truncated"])?;
        let sources = value["sources"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(|s| {
                let status = text(&s["status"])?;
                if !matches!(
                    status.as_str(),
                    "ok" | "unsupported" | "permission_denied" | "offline" | "unknown"
                ) {
                    return Err(invalid());
                }
                Ok(SourceObservation {
                    host: text(&s["host"])?,
                    socket: optional(&s["socket"])?,
                    status,
                    reason: optional(&s["reason"])?,
                })
            })
            .collect::<Result<Vec<_>, SquadError>>()?;
        if sources.is_empty() {
            return Err(invalid());
        }
        let rows = value["candidates"].as_array().ok_or_else(invalid)?;
        // These are the published Core bounds, not a second acquisition policy.
        if rows.len() > 256 {
            return Err(invalid());
        }
        let mut keys = BTreeSet::new();
        let mut panes = Vec::with_capacity(rows.len());
        for row in rows {
            let key = PaneKey {
                host: text(&row["host"])?,
                socket: optional(&row["socket"])?,
                pane: text(&row["pane"])?,
            };
            if !pane_key(&key.host, &key.pane) || !keys.insert(key.clone()) {
                return Err(invalid());
            }
            let state = text(&row["state"])?;
            if !matches!(
                state.as_str(),
                "unknown" | "known" | "stale" | "hook_only" | "duplicate"
            ) {
                return Err(invalid());
            }
            let identity = if row["identity"].is_null() {
                None
            } else {
                Some(ExistingAgentEvidence {
                    id: uuid(&row["identity"]["id"])?,
                    name: text(&row["identity"]["name"])?,
                    binding: optional_uuid(&row["identity"]["binding"])?,
                })
            };
            let pane = ObservedPane {
                server: native(&row["server"])?,
                server_identity: optional_uuid(&row["server"]["identity"])?,
                native: native(&row["native"])?,
                session: optional(&row["session"])?,
                window: optional(&row["window"])?,
                target: optional(&row["target"])?,
                runtime: optional(&row["runtime"]["driver"])?,
                runtime_state: text(&row["runtime"]["state"])?,
                runtime_reason: optional(&row["runtime"]["reason"])?,
                recorded_state: optional(&row["runtime"]["recordedState"])?,
                identity,
                state,
                reason: optional(&row["reason"])?,
                duplicate: flag(&row["duplicate"])?,
                title: text(&row["display"]["title"])?,
                command: text(&row["display"]["command"])?,
                cwd: text(&row["display"]["cwd"])?,
                text_truncated: flag(&row["display"]["truncated"])?,
                agent_name: optional(&row["display"]["agentName"])?,
                adoptable: flag(&row["adoptable"])?,
                expect: digest(&row["expect"])?,
                key,
            };
            if [&pane.title, &pane.command, &pane.cwd]
                .into_iter()
                .chain(&pane.agent_name)
                .any(|s| s.len() > 1024)
            {
                return Err(invalid());
            }
            if pane.adoptable
                && (truncated
                    || pane.key.socket.as_ref().is_none_or(|s| s.is_empty())
                    || pane.server.is_none()
                    || pane.native.is_none()
                    || pane.expect.is_none()
                    || pane.reason.is_some()
                    || !sources.iter().any(|s| {
                        s.status == "ok" && s.host == pane.key.host && s.socket == pane.key.socket
                    }))
            {
                return Err(invalid());
            }
            if !pane.adoptable && pane.expect.is_some() {
                return Err(invalid());
            }
            panes.push(pane);
        }
        Ok(Self {
            observed_at_ms,
            sources,
            panes,
            truncated,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AgentSelector {
    NewName(String),
    ExistingUuid(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BindSelection {
    pub key: PaneKey,
    pub expect: String,
    pub selector: AgentSelector,
    pub save: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum BindReport {
    Bound {
        id: String,
        name: String,
        pane: String,
        lifetime: String,
        already_bound: bool,
        agent_name: Option<HostAgentName>,
    },
    Refused {
        code: String,
        message: String,
    },
    Partial {
        code: String,
        message: String,
    },
}
/// What an external host said about its agent name after a bind. Display
/// only: it never names or selects an identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum HostAgentName {
    Set(String),
    NotLinked,
    NotSet(String),
    Unconfirmed(String),
}
impl HostAgentName {
    /// The `agentName` member, or its `AGENT_NAME_*` warning. A tmux bind has
    /// neither. An unknown form is left out, since the bind itself succeeded.
    fn decode(value: &Value) -> Option<Self> {
        let member = &value["agentName"];
        let text = |value: &Value| value.as_str().map(str::to_owned);
        match member["status"].as_str() {
            Some("set") => text(&member["name"]).map(Self::Set),
            Some("not_linked") => Some(Self::NotLinked),
            Some("refused") => text(&member["message"]).map(Self::NotSet),
            _ => value["warnings"]
                .as_array()?
                .iter()
                .find(|w| {
                    w["code"]
                        .as_str()
                        .is_some_and(|c| c.starts_with("AGENT_NAME_"))
                })
                .and_then(|w| text(&w["message"]))
                .map(Self::Unconfirmed),
        }
    }
}
impl BindReport {
    pub fn decode(
        value: &Value,
        selection: &BindSelection,
        success: bool,
    ) -> Result<Self, SquadError> {
        if value["pane"] != selection.key.pane {
            return Err(invalid());
        }
        match value["status"].as_str() {
            Some("bound" | "already_bound")
                if success && value["version"] == 1 && value["bound"] == true =>
            {
                let id = uuid(&value["id"])?;
                if let AgentSelector::ExistingUuid(expected) = &selection.selector
                    && *expected != id
                {
                    return Err(invalid());
                }
                let lifetime = text(&value["lifetime"])?;
                if !matches!(lifetime.as_str(), "saved" | "temporary")
                    || selection.save && lifetime != "saved"
                {
                    return Err(invalid());
                }
                Ok(Self::Bound {
                    id,
                    name: text(&value["name"])?,
                    pane: selection.key.pane.clone(),
                    lifetime,
                    already_bound: value["status"] == "already_bound",
                    agent_name: HostAgentName::decode(value),
                })
            }
            Some("refused" | "partial") if !success => {
                let code = text(&value["error"]["code"])?;
                let message = text(&value["error"]["message"])?;
                if value["status"] == "partial" {
                    Ok(Self::Partial { code, message })
                } else {
                    Ok(Self::Refused { code, message })
                }
            }
            _ => Err(invalid()),
        }
    }
}
