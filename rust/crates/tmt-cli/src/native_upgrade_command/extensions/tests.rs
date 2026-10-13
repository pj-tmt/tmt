use super::*;
use std::{cell::RefCell, collections::VecDeque};
use tmt_adapters::process::{CommandError, CommandOutput};

struct Runner {
    replies: RefCell<VecDeque<(Vec<u8>, i32)>>,
    inputs: RefCell<Vec<Vec<u8>>>,
}

impl Runner {
    fn new(replies: Vec<(Value, i32)>) -> Self {
        Self {
            replies: RefCell::new(
                replies
                    .into_iter()
                    .map(|(value, code)| (serde_json::to_vec(&value).unwrap(), code))
                    .collect(),
            ),
            inputs: RefCell::new(Vec::new()),
        }
    }
}

impl CommandRunner for Runner {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        assert_eq!(request.program, "/managed/releases/new/tmt");
        let applying = !self.inputs.borrow().is_empty();
        assert_eq!(
            request.args,
            [
                "__native-upgrade-extensions",
                "--json",
                if applying { "--yes" } else { "--plan" }
            ]
            .map(std::ffi::OsString::from)
        );
        assert!(request.deadline > Instant::now());
        assert!(
            request.deadline <= Instant::now() + Duration::from_secs(180),
            "non-Remote plans retain their existing bound"
        );
        assert_eq!(request.max_output_bytes, upgrade_all::LIMIT);
        if applying {
            let plan = Plan::parse(request.input).unwrap();
            assert!(plan.products.is_empty());
            assert_eq!(plan.pending[1].2, "1.1.0");
        } else {
            assert!(request.input.is_empty());
        }
        self.inputs.borrow_mut().push(request.input.to_vec());
        let (stdout, status) = self
            .replies
            .borrow_mut()
            .pop_front()
            .expect("unexpected child invocation");
        if status == 0 {
            return Ok(CommandOutput {
                stdout,
                stderr: Vec::new(),
            });
        }
        UnixCommandRunner.execute(CommandRequest {
            program: std::ffi::OsStr::new("/bin/sh"),
            args: &[
                "-c".into(),
                "printf '%s' \"$1\"; exit \"$2\"".into(),
                "fixture".into(),
                String::from_utf8(stdout).unwrap().into(),
                status.to_string().into(),
            ],
            input: &[],
            deadline: request.deadline,
            max_output_bytes: request.max_output_bytes,
        })
    }
}

fn plan() -> Value {
    json!({
        "products": [],
        "pending": [
            {"product": "digest", "version": "1.0.0", "selected": "1.1.0"},
            {"product": "ops", "version": "1.0.0", "selected": "1.1.0"},
        ],
    })
}

#[test]
fn remote_pending_apply_gets_restart_budget_and_timeout_never_becomes_success() {
    struct Deadline(RefCell<usize>);
    impl CommandRunner for Deadline {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            let mut calls = self.0.borrow_mut();
            *calls += 1;
            if *calls == 1 {
                return Ok(CommandOutput { stdout: serde_json::to_vec(&json!({"products":[],"pending":[{"product":"remote","version":"1.0.0","selected":"1.1.0"}]})).unwrap(), stderr: vec![] });
            }
            assert_eq!(*calls, 2, "no apply retry after timeout");
            assert!(
                request.deadline
                    >= Instant::now()
                        + Duration::from_secs(180)
                        + native_install::POST_UPGRADE_RESTART_BOUND
                        - Duration::from_secs(1)
            );
            // Valid-looking success bytes before expiry cannot override the
            // actual process timeout and cleanup result.
            UnixCommandRunner.execute(CommandRequest {
                program: std::ffi::OsStr::new("/bin/sh"),
                args: &["-c".into(), "printf '%s' '{\"products\":[{\"product\":\"remote\",\"status\":\"changed\",\"version\":\"1.1.0\"}]}'; sleep 2".into()],
                input: &[], deadline: Instant::now() + Duration::from_millis(20), max_output_bytes: request.max_output_bytes,
            })
        }
    }
    let runner = Deadline(RefCell::new(0));
    let rows = upgrade_with(Path::new("/managed/releases/new/tmt"), &runner, |_| {
        Ok(true)
    });
    assert_eq!(*runner.0.borrow(), 2);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["status"], "failed");
    assert_eq!(rows[0]["error"]["code"], "EXTENSION_UPGRADE_FAILED");
    assert!(
        rows[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Timeout")
    );
}

#[test]
fn new_executable_plans_and_applies_exact_consented_versions_with_partial_failure() {
    let runner = Runner::new(vec![
        (plan(), 0),
        (
            json!({"products": [
                {"product": "digest", "status": "failed", "error": {"code": "EXTENSION_UPGRADE_FAILED", "message": "new verifier refused"}},
                {"product": "ops", "status": "changed", "version": "1.1.0"},
            ]}),
            1,
        ),
    ]);
    let rows = upgrade_with(Path::new("/managed/releases/new/tmt"), &runner, |plan| {
        assert_eq!(plan.pending.len(), 2);
        assert_eq!(plan.pending[0].1, "1.0.0");
        Ok(true)
    });
    assert_eq!(runner.inputs.borrow().len(), 2);
    assert_eq!(rows[0]["error"]["message"], "new verifier refused");
    assert_eq!(rows[1]["status"], "changed");
}

#[test]
fn no_consent_never_invokes_the_apply_child() {
    let runner = Runner::new(vec![(plan(), 0)]);
    let rows = upgrade_with(Path::new("/managed/releases/new/tmt"), &runner, |_| {
        Ok(false)
    });
    assert_eq!(runner.inputs.borrow().len(), 1);
    assert!(
        rows.iter()
            .all(|row| row["status"] == "consentRequired" && row["hint"] == "tmt upgrade --yes")
    );
}

#[test]
fn unsupported_or_invalid_plan_fails_without_old_process_fallback() {
    for (reply, status) in [
        (
            json!({"error": {"code": "UNKNOWN_COMMAND", "message": "old target"}}),
            2,
        ),
        (
            json!({"products": [], "pending": [], "unexpected": true}),
            0,
        ),
    ] {
        let runner = Runner::new(vec![(reply, status)]);
        let rows = upgrade_with(Path::new("/managed/releases/new/tmt"), &runner, |_| {
            panic!("no consent for invalid plan")
        });
        assert_eq!(runner.inputs.borrow().len(), 1);
        assert_eq!(rows[0]["status"], "failed");
        assert_eq!(rows[0]["hint"], "tmt upgrade");
        assert!(
            rows[0]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("Run tmt upgrade again")
        );
    }
}

#[test]
fn apply_report_must_match_selected_products_versions_and_exit_status() {
    for reply in [
        json!({"products": []}),
        json!({"products": [
            {"product": "digest", "status": "changed", "version": "1.1.0"},
            {"product": "ops", "status": "changed", "version": "9.9.9"},
        ]}),
        json!({"products": [
            {"product": "digest", "status": "failed", "error": {"code": "REFUSED", "message": "no"}},
            {"product": "ops", "status": "changed", "version": "1.1.0"},
        ]}),
    ] {
        let runner = Runner::new(vec![(plan(), 0), (reply, 0)]);
        let rows = upgrade_with(Path::new("/managed/releases/new/tmt"), &runner, |_| {
            Ok(true)
        });
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row["status"] == "failed"));
    }
}
