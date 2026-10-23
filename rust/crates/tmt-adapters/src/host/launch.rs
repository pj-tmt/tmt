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

/// `built_in` first, then each approved host driver in the registry's order. A driver offers
/// its template only while its executable is the one that was approved: otherwise the command
/// line is not one the user saw. A stored template is validated again, as the registry file
/// is not trusted. The built-in host is named by `host.rs`, which owns the terminal hosts.
pub(super) fn compose(
    built_in: HostLaunch,
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
    std::iter::once(built_in).chain(hosts).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(name: &str, kind: &str, launch: Option<serde_json::Value>) -> DriverRecord {
        let mut capabilities = json!({"kind": kind, "name": name, "version": "test",
            "protocols": [1], "ops": ["snapshot"], "paneId": {"prefix": "term_"},
            "target": "w{n}:p{n}", "callerEnv": [], "executables": [name]});
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

    fn first() -> HostLaunch {
        HostLaunch {
            host: "first".into(),
            source: HostSource::BuiltIn,
            launch: LaunchTemplate::new(&["run".into(), "{session}".into(), "{command}".into()])
                .ok(),
        }
    }

    fn launch_templates(drivers: &[(DriverRecord, ApprovalState)]) -> Vec<HostLaunch> {
        compose(first(), drivers)
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
