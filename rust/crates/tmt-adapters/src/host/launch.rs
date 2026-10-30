//! The default launch template of each host TMT can drive: the argv that opens a new pane in a
//! session and runs one command there. Core publishes the data; Ops expands and runs it.

use crate::driver_protocol::registry::{ApprovalState, DriverRecord};
pub use tmt_driver_protocol::LaunchTemplate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSource {
    /// Built into this tmt.
    BuiltIn,
    /// An approved host driver, in the state it would be found in when next run.
    Driver(ApprovalState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostLaunch {
    pub host: String,
    pub source: HostSource,
    /// `None` when the host publishes no template (an older driver) or its driver no longer
    /// runs as approved.
    pub launch: Option<LaunchTemplate>,
}

/// The `built_in` hosts first, then each approved host driver in the registry's order. A driver
/// offers its template only while its executable is the one that was approved: otherwise the
/// command line is not one the user saw. A stored template is validated again, as the registry
/// file is not trusted. The built-in hosts are named by `host.rs`, which owns the terminal hosts.
pub(super) fn compose(
    built_in: Vec<HostLaunch>,
    drivers: &[(DriverRecord, ApprovalState)],
) -> Vec<HostLaunch> {
    let hosts = drivers.iter().filter_map(|(record, state)| {
        let capabilities = record.capabilities.host()?;
        let launch = capabilities
            .launch
            .as_deref()
            .filter(|_| *state == ApprovalState::Ok)
            .and_then(|argv| LaunchTemplate::new(argv).ok());
        Some(HostLaunch {
            host: record.name.clone(),
            source: HostSource::Driver(*state),
            launch,
        })
    });
    built_in.into_iter().chain(hosts).collect()
}

impl super::CallerEnvironment {
    /// The host a client may preselect for a new pane: the one the caller's locators name
    /// (`callerHost` of `tmt host templates`). Advisory only: it is not evidence and selects,
    /// binds or launches nothing; implicit selection keeps its own rules. `None` when no
    /// locator names a host, or more than one external host is named.
    pub fn advisory_host(&self, records: &[DriverRecord]) -> Option<String> {
        use tmt_core::host::HostKind;
        match self.named_host(records) {
            super::NamedHost::Pty | super::NamedHost::PtyNested => {
                Some(HostKind::pty().as_str().to_owned())
            }
            // tmux inside another host: the caller's own pane is tmux's.
            super::NamedHost::Builtin | super::NamedHost::Nested(_) if self.names_tmux() => {
                Some(HostKind::Tmux.as_str().to_owned())
            }
            super::NamedHost::External(record) => Some(record.name.clone()),
            super::NamedHost::Builtin
            | super::NamedHost::Nested(_)
            | super::NamedHost::Ambiguous => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(name: &str, kind: &str, launch: Option<serde_json::Value>) -> DriverRecord {
        located(name, kind, launch, &[])
    }

    fn located(
        name: &str,
        kind: &str,
        launch: Option<serde_json::Value>,
        caller_env: &[&str],
    ) -> DriverRecord {
        let mut capabilities = json!({"kind": kind, "name": name, "version": "test",
            "protocols": [1], "ops": ["snapshot"], "paneId": {"prefix": "term_"},
            "target": "w{n}:p{n}", "callerEnv": caller_env, "executables": [name]});
        if let Some(launch) = launch {
            capabilities["launch"] = launch;
        }
        serde_json::from_value(json!({
            "name": name, "path": "/tmp/driver", "digest": "test", "protocol": 1,
            "fingerprint": {"device": 0, "inode": 0, "size": 0, "modifiedNs": 0,
                "changedNs": 0, "uid": 0, "mode": 0}, "approvedAtMs": 0,
            "capabilities": capabilities
        }))
        .unwrap()
    }

    #[test]
    fn the_advisory_host_is_the_one_the_callers_locators_name() {
        use crate::host::CallerEnvironment;
        let records = [
            located("herdr", "host", None, &["HERDR_PANE_ID"]),
            located("other", "host", None, &["OTHER_PANE_ID"]),
        ];
        let caller = |tmux: bool, variables: &[&str]| CallerEnvironment {
            tmux: tmux.then(|| "/tmp/advisory.sock,1,0".into()),
            pane: tmux.then(|| "%1".into()),
            process_id: 1,
            driver_env: variables
                .iter()
                .map(|name| ((*name).to_owned(), "value".into()))
                .collect(),
        };
        let advisory = |tmux, variables: &[&str]| caller(tmux, variables).advisory_host(&records);
        assert_eq!(advisory(false, &[]), None, "a plain terminal names none");
        assert_eq!(advisory(true, &[]).as_deref(), Some("tmux"));
        assert_eq!(
            advisory(false, &["HERDR_PANE_ID"]).as_deref(),
            Some("herdr")
        );
        assert_eq!(
            advisory(true, &["HERDR_PANE_ID"]).as_deref(),
            Some("tmux"),
            "tmux inside Herdr"
        );
        for pty in [super::super::pty::PANE_ENV, super::super::pty::SOCKET_ENV] {
            assert_eq!(advisory(false, &[pty]).as_deref(), Some("pty"));
            assert_eq!(
                advisory(true, &[pty]).as_deref(),
                Some("pty"),
                "built-in in tmux"
            );
        }
        assert_eq!(
            advisory(false, &["HERDR_PANE_ID", "OTHER_PANE_ID"]),
            None,
            "two external hosts choose none"
        );
    }

    fn first() -> HostLaunch {
        HostLaunch {
            host: "first".into(),
            source: HostSource::BuiltIn,
            launch: LaunchTemplate::new(&["run".into(), "{session}".into(), "{command}".into()])
                .ok(),
        }
    }

    fn launch_templates(drivers: &[(DriverRecord, ApprovalState)]) -> Vec<HostLaunch> {
        compose(vec![first()], drivers)
    }

    fn herdr_template() -> serde_json::Value {
        json!(["herdr-open", "{session}", "{command}"])
    }

    #[test]
    fn the_built_in_host_comes_first_and_a_driver_offers_the_template_it_published() {
        let hosts = launch_templates(&[
            (
                record("screen", "host", Some(herdr_template())),
                ApprovalState::Ok,
            ),
            (record("old", "host", None), ApprovalState::Ok),
        ]);
        let names: Vec<&str> = hosts.iter().map(|host| host.host.as_str()).collect();
        assert_eq!(names, ["first", "screen", "old"]);
        assert_eq!(hosts[0].source, HostSource::BuiltIn);
        assert_eq!(hosts[0], first());
        assert_eq!(
            hosts[1].launch.as_ref().unwrap().argv(),
            ["herdr-open", "{session}", "{command}"]
        );
        // A driver from before launch templates reports none rather than a guess.
        assert_eq!(hosts[2].launch, None);
        assert_eq!(hosts[2].source, HostSource::Driver(ApprovalState::Ok));
    }

    #[test]
    fn a_driver_that_no_longer_runs_as_approved_offers_no_command_line() {
        for state in [ApprovalState::Changed, ApprovalState::Missing] {
            let hosts =
                launch_templates(&[(record("screen", "host", Some(herdr_template())), state)]);
            assert_eq!(hosts[1].launch, None, "{state:?}");
            assert_eq!(hosts[1].source, HostSource::Driver(state));
        }
    }

    #[test]
    fn a_stored_template_that_is_not_valid_is_not_offered_and_runtime_drivers_are_skipped() {
        let hosts = launch_templates(&[
            (
                record("bad", "host", Some(json!(["run", "{command}"]))),
                ApprovalState::Ok,
            ),
            (record("agent", "runtime", None), ApprovalState::Ok),
        ]);
        assert_eq!(hosts.len(), 2);
        assert_eq!((hosts[1].host.as_str(), &hosts[1].launch), ("bad", &None));
    }
}
