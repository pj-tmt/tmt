//! `tmt ls --detected` (#4471): one discovery scan after the member listing
//! has closed storage. A detected row is presence only: it has no identity,
//! no address and no talk target, and its agent name is display text.

use crate::{discovery_command, invocation::DetectedScope, output::Failure};
use serde_json::Value;
use tmt_core::host::HostKind;

/// A pane that a stored binding holds: its host, server socket and pane ID.
pub(super) type BoundPane = (HostKind, String, String);

/// The scan's discovery v1 document with only the rows `ls` shows, and the
/// cleanup failure that makes the list partial.
pub(super) struct Detected {
    pub(super) document: Value,
    pub(super) cleanup_failure: Option<String>,
}

/// Scans what `scope` selects and keeps the agent-like rows that no stored
/// binding holds. The approvals are read once, read only; the listing's own
/// read and registration are not reused, so the scan selects exactly as
/// `tmt discover` does.
pub(super) fn scan(scope: &DetectedScope, bound: &[BoundPane]) -> Detected {
    let admitted = tmt_adapters::host::external::admitted();
    let scan =
        discovery_command::observe(&admitted, scope.host.as_deref(), scope.socket.as_deref());
    from_scan(scan, bound)
}

pub(super) fn from_scan(scan: discovery_command::Scan, bound: &[BoundPane]) -> Detected {
    Detected {
        document: shown(scan.document, &scan.observed, bound),
        cleanup_failure: scan.cleanup_failure,
    }
}

impl Detected {
    /// The one error of a partial list: a scan child that was not reaped.
    /// The members stay complete; storage was closed before the scan.
    pub(super) fn failure(&self) -> Option<Failure> {
        self.cleanup_failure
            .as_ref()
            .map(|message| Failure::new("CLEANUP_ERROR", message.clone(), 1))
    }
}

/// `document` without the rows that are members: a stored binding holds the
/// exact (host, socket, pane ID) the row was observed at, or the row names an
/// identity. Only agent-like rows stay.
fn shown(mut document: Value, observed: &[BoundPane], bound: &[BoundPane]) -> Value {
    let rows = match document["candidates"].take() {
        Value::Array(rows) => rows,
        _ => Vec::new(),
    };
    let rows: Vec<Value> = rows
        .into_iter()
        .zip(observed)
        .filter(|(row, at)| !bound.contains(at) && row["identity"].is_null() && agent_like(row))
        .map(|(row, _)| row)
        .collect();
    document["candidates"] = rows.into();
    document
}

/// An advisory heuristic: the host names an agent, or the pane runs a
/// registered driver's executable. Neither hint is evidence, routes, or
/// suggests an identity name; a host can label a shell too.
fn agent_like(row: &Value) -> bool {
    row["display"]["agentName"].is_string()
        || row["runtime"]["driver"].as_str().is_some_and(|driver| {
            tmt_adapters::drivers::Registry::builtin()
                .names()
                .contains(&driver)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(pane: &str, agent: Option<&str>, driver: Option<&str>) -> Value {
        json!({"pane": pane, "identity": null, "state": "unknown", "reason": null,
            "runtime": {"driver": driver}, "display": {"agentName": agent}})
    }

    fn at(socket: &str, pane: &str) -> BoundPane {
        (
            HostKind::parse("herdr").unwrap(),
            socket.into(),
            pane.into(),
        )
    }

    fn panes(document: &Value) -> Vec<&str> {
        document["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["pane"].as_str().unwrap())
            .collect()
    }

    /// The composer seam (case 8): a stale or conflicting-owner row that no
    /// binding holds stays, with its reason; the same row on a bound tuple
    /// goes. Only the exact tuple hides a row.
    #[test]
    fn only_the_exact_bound_tuple_hides_a_row_and_its_reason_stays() {
        let mut stale = row("w1:p1", Some("Ada"), None);
        stale["state"] = "stale".into();
        stale["reason"] = "stale_binding".into();
        let mut duplicate = row("w1:p2", Some("Ada"), None);
        duplicate["state"] = "duplicate".into();
        duplicate["reason"] = "conflicting_bindings".into();
        let document = json!({"candidates": [stale, duplicate]});
        let observed = [at("/h.sock", "w1:p1"), at("/h.sock", "w1:p2")];
        let kept = shown(document.clone(), &observed, &[]);
        assert_eq!(panes(&kept), ["w1:p1", "w1:p2"]);
        assert_eq!(kept["candidates"][0]["reason"], "stale_binding");
        assert_eq!(kept["candidates"][1]["reason"], "conflicting_bindings");
        let kept = shown(document.clone(), &observed, &[at("/h.sock", "w1:p2")]);
        assert_eq!(panes(&kept), ["w1:p1"]);
        // The same pane ID on another socket, or another host, is not bound.
        let tmux = (HostKind::Tmux, "/h.sock".into(), "w1:p1".into());
        let kept = shown(document, &observed, &[at("/other.sock", "w1:p1"), tmux]);
        assert_eq!(panes(&kept), ["w1:p1", "w1:p2"]);
    }

    /// The filter (case 7): a plain shell goes; an agent name or a registered
    /// driver's executable keeps a row; a row naming an identity is a member.
    #[test]
    fn agent_like_rows_stay_and_shells_and_known_rows_go() {
        let mut known = row("w1:p4", Some("Ada"), None);
        known["identity"] = json!({"id": "x", "name": "worker", "binding": "b"});
        let document = json!({"candidates": [
            row("w1:p1", None, None),
            row("w1:p2", None, Some("codex")),
            row("w1:p3", Some("Ada"), None),
            known,
            row("w1:p5", None, Some("not-a-driver")),
        ]});
        let observed: Vec<BoundPane> = (1..=5)
            .map(|n| at("/h.sock", &format!("w1:p{n}")))
            .collect();
        assert_eq!(panes(&shown(document, &observed, &[])), ["w1:p2", "w1:p3"]);
    }
}
