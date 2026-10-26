use super::*;
use std::{cell::RefCell, collections::VecDeque};
use tmt_adapters::process::{CommandError, CommandOutput};

fn plan() -> Plan {
    Plan {
        installed: upgrade_all::Plan {
            products: vec![
                json!({"product":"digest","status":"skippedPinned","version":"0.1.0-alpha.1"}),
            ],
            pending: vec![],
        },
        missing: vec![
            json!({"product":"ops","action":"install","channel":"alpha","version":"0.1.0-alpha.2","status":"pending","selected":true,"hint":"tmt extension install ops --yes"}),
        ],
        providers: vec![
            json!({"provider":"claude","status":"pending","selected":false,"skills":[],"hooks":[],"hint":"tmt setup claude"}),
        ],
    }
}
struct Runner {
    replies: RefCell<VecDeque<(Value, Vec<u8>, u32)>>,
    calls: RefCell<Vec<(Vec<std::ffi::OsString>, Vec<u8>)>>,
}
impl Runner {
    fn new(replies: Vec<(Value, Vec<u8>, u32)>) -> Self {
        Self {
            replies: RefCell::new(replies.into()),
            calls: RefCell::new(vec![]),
        }
    }
}
impl CommandRunner for Runner {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        assert_eq!(request.program, "/active/new/tmt");
        self.calls
            .borrow_mut()
            .push((request.args.to_vec(), request.input.to_vec()));
        let (document, stderr, status) = self
            .replies
            .borrow_mut()
            .pop_front()
            .expect("unexpected child/apply retry");
        if status == 0 {
            return Ok(CommandOutput {
                stdout: serde_json::to_vec(&document).unwrap(),
                stderr,
            });
        }
        assert!(stderr.is_empty());
        UnixCommandRunner.execute(CommandRequest {
            program: std::ffi::OsStr::new("/bin/sh"),
            args: &[
                "-c".into(),
                "printf '%s' \"$1\"; exit \"$2\"".into(),
                "fixture".into(),
                document.to_string().into(),
                status.to_string().into(),
            ],
            input: &[],
            deadline: request.deadline,
            max_output_bytes: request.max_output_bytes,
        })
    }
}
fn result() -> Value {
    json!({"products":[{"product":"digest","status":"skippedPinned","version":"0.1.0-alpha.1"},
    {"product":"ops","status":"changed","version":"0.1.0-alpha.2"}],
    "providers":[{"provider":"claude","status":"consentRequired","hint":"tmt setup claude"}],"interrupted":false})
}

#[test]
fn no_consent_never_invokes_the_activated_executable() {
    let runner = Runner::new(vec![]);
    let outcome = run(
        Path::new("/active/new/tmt"),
        &plan(),
        false,
        OutputMode { json: true },
        &runner,
    );
    assert!(runner.calls.borrow().is_empty());
    assert_eq!(outcome.products[1]["status"], "consentRequired");
    assert_eq!(outcome.providers[0]["hint"], "tmt setup claude");
}
#[test]
fn negotiated_apply_receives_the_exact_selection_once_and_reports_missing_provider_consent() {
    let approved = plan();
    let runner = Runner::new(vec![
        (
            approved.document(),
            upgrade_all::complete::CAPABILITY.to_vec(),
            0,
        ),
        (result(), vec![], 0),
    ]);
    let outcome = run(
        Path::new("/active/new/tmt"),
        &approved,
        true,
        OutputMode { json: true },
        &runner,
    );
    assert_eq!(outcome.products[1]["status"], "changed");
    assert_eq!(outcome.providers[0]["status"], "consentRequired");
    let calls = runner.calls.borrow();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].1.is_empty());
    assert_eq!(
        calls[1].1,
        serde_json::to_vec(&approved.document()).unwrap()
    );
    assert!(calls[1].0.contains(&"--complete".into()));
}
#[test]
fn scope_drift_refuses_before_apply_and_malformed_apply_never_replays() {
    let approved = plan();
    let mut changed = approved.document();
    changed["missing"][0]["version"] = "0.1.0-alpha.3".into();
    let runner = Runner::new(vec![(
        changed,
        upgrade_all::complete::CAPABILITY.to_vec(),
        0,
    )]);
    let outcome = run(
        Path::new("/active/new/tmt"),
        &approved,
        true,
        OutputMode { json: true },
        &runner,
    );
    assert_eq!(runner.calls.borrow().len(), 1);
    assert_eq!(outcome.products[1]["status"], "failed");
    let runner = Runner::new(vec![
        (
            approved.document(),
            upgrade_all::complete::CAPABILITY.to_vec(),
            0,
        ),
        (json!({"unconfirmed":true}), vec![], 0),
    ]);
    let outcome = run(
        Path::new("/active/new/tmt"),
        &approved,
        true,
        OutputMode { json: true },
        &runner,
    );
    assert_eq!(runner.calls.borrow().len(), 2);
    assert_eq!(outcome.products[1]["status"], "failed");
}
#[test]
fn an_old_target_gets_only_legacy_planning_and_no_missing_item_apply() {
    let approved = plan();
    let runner = Runner::new(vec![
        (json!({}), vec![], 2),
        (
            approved.installed.document(),
            crate::native_progress::child::CAPABILITY.to_vec(),
            0,
        ),
    ]);
    let outcome = run(
        Path::new("/active/new/tmt"),
        &approved,
        true,
        OutputMode { json: true },
        &runner,
    );
    assert_eq!(outcome.products[0]["status"], "skippedPinned");
    assert_eq!(outcome.products[1]["status"], "consentRequired");
    assert!(
        runner
            .calls
            .borrow()
            .iter()
            .all(|(_, input)| input.is_empty())
    );
    assert_eq!(runner.calls.borrow().len(), 2);
}
#[test]
fn apply_results_cannot_claim_unapproved_provider_changes_or_new_targets() {
    let approved = plan();
    assert!(parse_results(&serde_json::to_vec(&result()).unwrap(), 0, &approved).is_some());
    let mut document = result();
    document["providers"][0]["status"] = "changed".into();
    assert!(parse_results(&serde_json::to_vec(&document).unwrap(), 0, &approved).is_none());
    let mut document = result();
    document["products"][1]["version"] = "0.1.0-alpha.3".into();
    assert!(parse_results(&serde_json::to_vec(&document).unwrap(), 0, &approved).is_none());
    let mut document = result();
    document["products"][1]["status"] = "failed".into();
    assert!(parse_results(&serde_json::to_vec(&document).unwrap(), 0, &approved).is_none());
    assert!(parse_results(&serde_json::to_vec(&document).unwrap(), 1, &approved).is_some());
}

#[test]
fn yes_with_only_pinned_products_and_unselected_provider_scopes_needs_no_child() {
    let mut approved = plan();
    approved.missing.clear();
    let runner = Runner::new(vec![]);
    let outcome = run(
        Path::new("/active/new/tmt"),
        &approved,
        true,
        OutputMode { json: true },
        &runner,
    );
    assert!(runner.calls.borrow().is_empty());
    assert_eq!(outcome.products[0]["status"], "skippedPinned");
    assert_eq!(outcome.providers[0]["status"], "consentRequired");
}
