use super::*;
use crate::process::{CommandError, CommandOutput};
use std::cell::RefCell;

#[test]
fn partial_failure_evidence_requires_an_actual_activation_report() {
    let error = std::io::Error::other("before activation");
    assert!(super::super::activated_report(&error).is_none());
    let error = std::io::Error::other(super::super::ActivatedInstallation {
        report: report(true),
        cause: std::io::Error::other("after activation"),
    });
    assert_eq!(
        super::super::activated_report(&error)
            .unwrap()
            .active_executable,
        report(true).active_executable
    );
    let mut failure = super::super::UpgradeFailure::from(std::io::Error::other("finalization"));
    failure.activated = Some(Box::new(super::super::UpgradeReport {
        installation: report(true),
        state: tmt_core::native_install::InstalledVersion {
            version: "0.1.0-alpha.2".parse().unwrap(),
            channel: tmt_core::native_install::Channel::Alpha,
            pinned_version: None,
        },
        skipped_pinned: false,
    }));
    assert_eq!(
        super::super::activated_report(&std::io::Error::other(failure))
            .unwrap()
            .version,
        "0.1.0-alpha.2"
    );
}

struct Remote<'a> {
    status: &'a str,
    restart: &'a str,
    calls: RefCell<Vec<String>>,
    failure: Option<CommandFailure>,
    cleanup_failed: bool,
}
impl CommandRunner for Remote<'_> {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        assert_eq!(request.program, "/usr/bin/env");
        assert_eq!(
            &request.args[..5],
            [
                "-u",
                "GH_TOKEN",
                "-u",
                "GITHUB_TOKEN",
                "TMT_EXECUTABLE=/verified/core with spaces/tmt"
            ]
        );
        let command = request.args[6].to_str().unwrap();
        self.calls.borrow_mut().push(command.into());
        assert!(request.input.is_empty());
        assert!(request.deadline > Instant::now());
        assert_eq!(request.max_output_bytes, 64 * 1024);
        let (bytes, budget) = match command {
            "status" => {
                assert_eq!(&request.args[6..], ["status", "--json"]);
                (self.status, BUDGET)
            }
            "restart" => {
                assert_eq!(&request.args[6..], ["restart", "--json", "--no-open"]);
                assert_eq!(request.args[5], "/verified/release with spaces/tmt-remote");
                (self.restart, POST_UPGRADE_RESTART_BOUND)
            }
            _ => panic!("Core never calls stop or serve"),
        };
        assert!(request.deadline <= Instant::now() + budget);
        let output = CommandOutput {
            stdout: bytes.as_bytes().to_vec(),
            stderr: b"never expose child diagnostics".to_vec(),
        };
        match self.failure {
            None => Ok(output),
            Some(kind) => {
                let mut error = CommandError::new(kind);
                if matches!(kind, CommandFailure::Exit { .. }) {
                    error.output = Some(output);
                }
                if self.cleanup_failed {
                    error.cleanup_error = Some(std::io::Error::other("cleanup failed"));
                }
                Err(error)
            }
        }
    }
}
fn remote<'a>(status: &'a str, restart: &'a str) -> Remote<'a> {
    Remote {
        status,
        restart,
        calls: RefCell::new(vec![]),
        failure: None,
        cleanup_failed: false,
    }
}
fn report(changed: bool) -> InstallReport {
    InstallReport {
        executable: "/unselected/bin/tmt-remote".into(),
        active_executable: "/verified/release with spaces/tmt-remote".into(),
        version: "0.1.0-alpha.2".into(),
        changed,
    }
}
const CORE: &str = "/verified/core with spaces/tmt";
const LIVE: &str = r#"{"running":true,"origin":"http://127.0.0.1:49152","path":"/r/prefix"}"#;
fn hint(runner: &Remote<'_>) -> Option<&'static str> {
    let before = capture(
        Product::Remote,
        Path::new("/verified/old release/tmt-remote"),
        Some(Path::new(CORE)),
        runner,
    );
    observe(
        Product::Remote,
        &report(true),
        true,
        Some(Path::new(CORE)),
        before,
        runner,
    )
}
#[test]
fn replaced_running_remote_uses_its_restart_owner_and_reports_success() {
    let runner = remote(
        LIVE,
        r#"{"outcome":"restarted","reason":null,"additive":true}"#,
    );
    assert_eq!(hint(&runner), Some(RESTARTED));
    assert_eq!(*runner.calls.borrow(), ["status", "restart"]);
}
#[test]
fn only_actual_replacement_admits_the_restart() {
    let runner = remote("invalid", "invalid");
    for product in Product::ALL {
        assert_eq!(
            observe(
                product,
                &report(true),
                false,
                None,
                PostUpgradeObservation::Running,
                &runner
            ),
            None
        );
        assert_eq!(
            observe(
                product,
                &report(false),
                true,
                None,
                PostUpgradeObservation::Running,
                &runner
            ),
            None
        );
        if product != Product::Remote {
            assert_eq!(
                capture(product, Path::new("/must-not-run"), None, &runner),
                PostUpgradeObservation::NotApplicable
            );
            assert_eq!(
                observe(
                    product,
                    &report(true),
                    true,
                    None,
                    PostUpgradeObservation::Running,
                    &runner
                ),
                None
            );
        }
    }
    assert!(runner.calls.borrow().is_empty());
    assert_eq!(
        observe(
            Product::Remote,
            &report(true),
            true,
            None,
            PostUpgradeObservation::Running,
            &runner
        ),
        Some(UNKNOWN)
    );
}
#[test]
fn stopped_and_unknown_pre_observations_never_restart() {
    for status in [
        r#"{"running":false,"lastPort":null}"#,
        r#"{"running":false,"lastPort":49152,"additive":true}"#,
    ] {
        let runner = remote(status, "must not run");
        assert_eq!(hint(&runner), None);
        assert_eq!(*runner.calls.borrow(), ["status"]);
    }
    for status in [
        "invalid",
        "[]",
        r#"{"running":false}"#,
        r#"{"running":false,"lastPort":0}"#,
        r#"{"running":false,"lastPort":65536}"#,
        r#"{"running":"false","lastPort":null}"#,
        r#"{"running":true}"#,
    ] {
        let runner = remote(status, "must not run");
        assert_eq!(hint(&runner), Some(UNKNOWN));
        assert_eq!(*runner.calls.borrow(), ["status"]);
    }
    assert_eq!(
        remote_status(r#"{"running":true,"origin":"x","path":"y","extra":1}"#.as_bytes()),
        PostUpgradeObservation::Running
    );
}
#[test]
fn fixed_restart_outcomes_preserve_success_quiet_and_fallbacks() {
    for (reply, expected) in [
        (r#"{"outcome":"not-running","reason":null}"#, None),
        (r#"{"outcome":"deferred","reason":"busy"}"#, Some(RUNNING)),
        (
            r#"{"outcome":"deferred","reason":"foreground","extra":1}"#,
            Some(RUNNING),
        ),
        (
            r#"{"outcome":"deferred","reason":"legacy"}"#,
            Some(OUTDATED),
        ),
        (
            r#"{"outcome":"unconfirmed","reason":"candidate"}"#,
            Some(UNKNOWN),
        ),
        (
            r#"{"outcome":"unconfirmed","reason":"startup"}"#,
            Some(UNKNOWN),
        ),
        (
            r#"{"outcome":"unconfirmed","reason":"unknown"}"#,
            Some(UNKNOWN),
        ),
        (r#"{"outcome":"restarted"}"#, Some(UNKNOWN)),
        (r#"{"outcome":"restarted","reason":"busy"}"#, Some(UNKNOWN)),
        (r#"{"outcome":true,"reason":null}"#, Some(UNKNOWN)),
        (
            r#"{"outcome":"deferred","reason":"invalid"}"#,
            Some(UNKNOWN),
        ),
        ("invalid", Some(UNKNOWN)),
    ] {
        assert_eq!(hint(&remote(LIVE, reply)), expected);
    }
}
#[test]
fn lost_or_malformed_restart_reply_is_observed_without_retry() {
    let runner = remote(LIVE, "not a reply");
    assert_eq!(
        hint(&runner),
        Some(UNKNOWN),
        "a valid running status after uncertainty cannot prove restart completion"
    );
    assert_eq!(*runner.calls.borrow(), ["status", "restart", "status"]);
    for kind in [
        CommandFailure::Spawn,
        CommandFailure::Timeout,
        CommandFailure::OutputLimit,
        CommandFailure::Io,
        CommandFailure::Exit {
            code: Some(1),
            signal: None,
        },
    ] {
        let mut runner = remote(LIVE, "");
        runner.failure = Some(kind);
        assert_eq!(
            observe(
                Product::Remote,
                &report(true),
                true,
                Some(Path::new(CORE)),
                PostUpgradeObservation::Running,
                &runner
            ),
            Some(UNKNOWN)
        );
        assert_eq!(*runner.calls.borrow(), ["restart", "status"]);
    }
}
#[test]
fn legacy_requires_clean_observed_exit_and_other_failures_are_unknown() {
    let mut runner = remote(
        r#"{"error":{"code":"REMOTE_SERVE_OUTDATED","message":"private"}}"#,
        "must not run",
    );
    runner.failure = Some(CommandFailure::Exit {
        code: Some(1),
        signal: None,
    });
    assert_eq!(hint(&runner), Some(OUTDATED));
    assert_eq!(*runner.calls.borrow(), ["status"]);
    runner.cleanup_failed = true;
    assert_eq!(hint(&runner), Some(UNKNOWN));
}
#[test]
fn restart_bound_matches_the_remote_phase_contract() {
    // Remote independently pins its named phase sum in control's owner test.
    assert_eq!(POST_UPGRADE_RESTART_BOUND, Duration::from_secs(155));
}

#[test]
fn additive_members_do_not_change_the_observed_door_state() {
    for extra in ["null", "false", "[]", r#"{"future":"opaque"}"#] {
        let running = format!(
            r#"{{"running":true,"origin":"http://127.0.0.1:49152","path":"/r/prefix","futureProjection":{extra}}}"#
        );
        assert_eq!(remote_status(running.as_bytes()), PostUpgradeObservation::Running);
        let stopped = format!(r#"{{"running":false,"lastPort":49152,"futureProjection":{extra}}}"#);
        assert_eq!(remote_status(stopped.as_bytes()), PostUpgradeObservation::Stopped);
    }
}

#[test]
fn additive_members_cannot_supply_missing_or_mistyped_required_members() {
    for value in [
        r#"{"futureProjection":{"running":true,"origin":"here","path":"there"}}"#,
        r#"{"running":null,"futureProjection":true}"#,
        r#"{"running":"true","origin":"here","path":"there","futureProjection":true}"#,
        r#"{"running":true,"path":"there","futureProjection":true}"#,
        r#"{"running":true,"origin":17,"path":"there","futureProjection":true}"#,
        r#"{"running":true,"origin":"here","futureProjection":true}"#,
        r#"{"running":true,"origin":"here","path":false,"futureProjection":true}"#,
        r#"{"running":false,"futureProjection":true}"#,
        r#"{"running":false,"lastPort":"49152","futureProjection":true}"#,
        r#"{"running":false,"lastPort":[],"futureProjection":true}"#,
        r#"{"running":false,"lastPort":null,"futureProjection":true"#,
    ] {
        assert_eq!(remote_status(value.as_bytes()), PostUpgradeObservation::Unknown, "{value}");
    }
}

