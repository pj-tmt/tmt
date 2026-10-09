use super::*;
use crate::process::{CommandError, CommandOutput};
use std::cell::Cell;

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

struct Status<'a> {
    calls: Cell<u32>,
    result: &'a str,
    failure: Option<CommandFailure>,
    cleanup_failed: bool,
}

impl CommandRunner for Status<'_> {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(request.program, "/usr/bin/env");
        assert_eq!(
            request.args,
            [
                "TMT_EXECUTABLE=/verified/core with spaces/tmt",
                "/verified/release with spaces/tmt-remote",
                "status",
                "--json",
            ]
        );
        assert!(request.input.is_empty());
        assert!(request.deadline > Instant::now());
        assert!(request.deadline <= Instant::now() + BUDGET);
        assert_eq!(request.max_output_bytes, 64 * 1024);
        let output = CommandOutput {
            stdout: self.result.as_bytes().to_vec(),
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

fn status(result: &str) -> Status<'_> {
    Status {
        calls: Cell::new(0),
        result,
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

fn hint(runner: &Status<'_>) -> Option<&'static str> {
    let value = observe(
        Product::Remote,
        &report(true),
        true,
        Some(Path::new("/verified/core with spaces/tmt")),
        runner,
    );
    assert_eq!(
        runner.calls.get(),
        1,
        "one bounded status check, no stop or serve"
    );
    value
}

#[test]
fn only_an_actual_replacement_of_a_declared_product_checks_status() {
    let runner = status("invalid");
    for product in Product::ALL {
        assert_eq!(observe(product, &report(true), false, None, &runner), None);
        assert_eq!(observe(product, &report(false), true, None, &runner), None);
        if product != Product::Remote {
            assert_eq!(observe(product, &report(true), true, None, &runner), None);
        }
    }
    assert_eq!(runner.calls.get(), 0);
    assert_eq!(
        observe(Product::Remote, &report(true), true, None, &runner),
        Some(UNKNOWN)
    );
    assert_eq!(runner.calls.get(), 0);
}

#[test]
fn running_stopped_and_changed_shapes_remain_distinct() {
    assert_eq!(
        hint(&status(
            r#"{"running":true,"origin":"http://127.0.0.1:49152","path":"/r/prefix"}"#
        )),
        Some(RUNNING)
    );
    for value in [
        r#"{"running":false,"lastPort":null}"#,
        r#"{"running":false,"lastPort":49152}"#,
    ] {
        assert_eq!(hint(&status(value)), None);
    }
    for value in [
        "invalid",
        "[]",
        r#"{"running":false}"#,
        r#"{"running":false,"lastPort":0}"#,
        r#"{"running":false,"lastPort":65536}"#,
        r#"{"running":"false","lastPort":null}"#,
        r#"{"running":false,"lastPort":null,"newField":true}"#,
        r#"{"running":true}"#,
    ] {
        assert_eq!(hint(&status(value)), Some(UNKNOWN));
    }
}

#[test]
fn legacy_requires_an_observed_clean_exit_and_other_failures_are_unknown() {
    let mut runner =
        status(r#"{"error":{"code":"REMOTE_SERVE_OUTDATED","message":"private child output"}}"#);
    runner.failure = Some(CommandFailure::Exit {
        code: Some(1),
        signal: None,
    });
    assert_eq!(hint(&runner), Some(OUTDATED));
    for kind in [
        CommandFailure::Spawn,
        CommandFailure::Timeout,
        CommandFailure::OutputLimit,
        CommandFailure::Io,
        CommandFailure::Exit {
            code: Some(2),
            signal: None,
        },
        CommandFailure::Exit {
            code: None,
            signal: Some(15),
        },
    ] {
        let mut runner = status(runner.result);
        runner.failure = Some(kind);
        assert_eq!(hint(&runner), Some(UNKNOWN));
    }
    let mut runner = status(runner.result);
    runner.failure = Some(CommandFailure::Exit {
        code: Some(1),
        signal: None,
    });
    runner.cleanup_failed = true;
    assert_eq!(hint(&runner), Some(UNKNOWN));
    assert_eq!(
        hint(&status(runner.result)),
        Some(UNKNOWN),
        "error-shaped success is not an outdated door"
    );
}
