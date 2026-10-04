use super::*;
use std::cell::Cell;
use tmt_adapters::process::{CommandError, CommandOutput};

struct Runner {
    calls: Cell<usize>,
    bytes: &'static [u8],
}
impl CommandRunner for Runner {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(request.program, "/managed/releases/new/tmt");
        assert_eq!(
            request.args,
            &[
                std::ffi::OsString::from("__native-refresh-skills"),
                "--managed".into(),
                "--json".into()
            ]
        );
        assert!(request.input.is_empty());
        assert!(request.deadline > Instant::now());
        Ok(CommandOutput {
            stdout: self.bytes.to_vec(),
            stderr: vec![],
        })
    }
}

#[test]
fn refresh_runs_new_immutable_executable_and_validates_its_report() {
    let runner = Runner {
        calls: Cell::new(0),
        bytes:
            br#"{"refreshed":[{"target":"/agent/tmt","changed":true}],"skipped":[],"conflicts":[]}"#,
    };
    let report = refresh(Path::new("/managed/releases/new/tmt"), &runner).unwrap();
    assert_eq!(runner.calls.get(), 1);
    assert_eq!(report["refreshed"][0]["changed"], true);
    for bytes in [b"not JSON".as_slice(), br#"{"refreshed":[],"skipped":[],"conflicts":[],"unexpected":"field"}"#, br#"{"refreshed":[],"skipped":[],"conflicts":[],"error":{"code":"FAIL","message":"failed"}}"#] {
        let runner = Runner { calls: Cell::new(0), bytes };
        assert!(refresh(Path::new("/managed/releases/new/tmt"), &runner).is_err());
    }
}

#[test]
fn completed_nonzero_child_keeps_its_valid_partial_skill_report() {
    struct ExitingRunner;
    impl CommandRunner for ExitingRunner {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            assert_eq!(request.program, "/managed/releases/new/tmt");
            UnixCommandRunner.execute(CommandRequest {
                program: std::ffi::OsStr::new("/bin/sh"),
                args: &["-c".into(), "printf '%s' '{\"refreshed\":[{\"target\":\"/valid\",\"changed\":true}],\"skipped\":[],\"conflicts\":[\"/modified\"],\"error\":{\"code\":\"SKILL_REFRESH_FAILED\",\"message\":\"conflict\"}}'; exit 1".into()],
                input: &[], deadline: request.deadline, max_output_bytes: request.max_output_bytes,
            })
        }
    }
    let (document, error) =
        refresh(Path::new("/managed/releases/new/tmt"), &ExitingRunner).unwrap_err();
    let document = document.expect("preserve the validated partial report");
    assert_eq!(document["refreshed"][0]["target"], "/valid");
    assert_eq!(document["conflicts"][0], "/modified");
    assert!(!error.to_string().contains("/modified"));
}

#[test]
fn retry_guidance_retains_an_explicit_pin() {
    let mut report = UpgradeReport {
        installation: native_install::InstallReport {
            executable: "/managed/bin/tmt".into(),
            active_executable: "/managed/releases/new/tmt".into(),
            version: "5.0.0".into(),
            changed: true,
        },
        state: tmt_core::native_install::InstalledVersion {
            version: "5.0.0".parse().unwrap(),
            channel: Channel::Stable,
            pinned_version: None,
        },
        skipped_pinned: false,
    };
    assert_eq!(retry_hint(&report), "tmt upgrade");
    report.state.pinned_version = Some(report.state.version.clone());
    assert_eq!(retry_hint(&report), "tmt upgrade --to 5.0.0");
}

#[test]
fn aggregate_report_keeps_cli_and_extension_outcomes_independent() {
    let report = UpgradeReport {
        installation: native_install::InstallReport {
            executable: "/managed/bin/tmt".into(),
            active_executable: "/managed/releases/new/tmt".into(),
            version: "5.0.0".into(),
            changed: true,
        },
        state: tmt_core::native_install::InstalledVersion {
            version: "5.0.0".parse().unwrap(),
            channel: Channel::Stable,
            pinned_version: None,
        },
        skipped_pinned: false,
    };
    let squad = json!({"product":"squad","status":"changed","version":"1.1.0"});
    let rows = product_rows(Some(&report), None, vec![squad.clone()]);
    assert_eq!(rows[0]["status"], "changed");
    assert_eq!(rows[0]["version"], "5.0.0");
    assert_eq!(rows[1], squad);
    let mode = OutputMode { json: true };
    let consent = json!({"product":"squad","status":"consentRequired","hint":"tmt upgrade --yes"});
    assert_eq!(
        publish_products(Some(&report), None, None, vec![consent], mode).unwrap(),
        0
    );
    let failure = json!({"product":"office","status":"failed","error":{"code":"EXTENSION_UPGRADE_FAILED","message":"refused"}});
    assert_eq!(
        publish_products(Some(&report), None, None, vec![failure, squad], mode).unwrap(),
        1
    );
    // This nonexistent prefix would fail inspection if the extension phase ran.
    assert_eq!(
        finish(
            &report,
            None,
            Some(Failure::new("NATIVE_UPGRADE_FAILED", "CLI failed", 7)),
            true,
            mode
        )
        .unwrap(),
        7
    );
}

#[test]
fn upgrade_failure_document_always_keeps_a_nonempty_cause() {
    for cause in ["connection refused", ""] {
        let failure = Failure::new("NATIVE_UPGRADE_FAILED", "Cannot update installation", 1)
            .caused_by(io::Error::other(cause));
        let document = failure_document(&failure);
        let diagnostic = document["error"]["cause"].as_str().unwrap();
        assert!(!diagnostic.is_empty());
        assert_eq!(
            diagnostic,
            if cause.is_empty() {
                "Cannot update installation"
            } else {
                cause
            }
        );
    }
}

#[test]
fn conflict_commands_quote_paths_and_preserve_sources() {
    let target = Path::new("/isolated/agent's home/skills/tmt-inbox");
    let hint = crate::skill_refresh_command::conflict_hint(target);
    assert!(hint.contains("User-owned or modified skill preserved"));
    assert!(hint.contains("'\\''"));
    assert!(hint.contains(".tmt-skill-backups/upgrade-conflict.XXXXXXXX"));
    assert!(hint.contains("mv "));
    assert!(!hint.contains("rm "));
}

#[test]
fn upgrade_json_retains_rate_limit_reset_and_token_hint_in_cause() {
    let cause = "GitHub API rate limit: reset/earliest retry time 2030-01-01 UTC; the required wait exceeds the remaining deadline. Retry later or optionally set GITHUB_TOKEN.";
    let failure = Failure::new("NATIVE_UPGRADE_FAILED", "Native upgrade failed", 1)
        .caused_by(std::io::Error::other(cause));
    let document = failure_document(&failure);
    assert_eq!(document["error"]["code"], "NATIVE_UPGRADE_FAILED");
    assert_eq!(document["error"]["cause"], cause);
}

#[test]
fn skills_summary_counts_only_missing_core_skills() {
    let office = json!({
        "refreshed": vec![json!({"target": "/a/tmux-team", "changed": false}); 8],
        "skipped": [
            "/a/tmt-office", "/a/tmt-avatar-create", "/a/tmt-prop-create",
            "/b/tmt-office", "/b/tmt-avatar-create", "/b/tmt-prop-create"
        ],
        "conflicts": [],
    });
    assert_eq!(
        skills_summary(&office),
        "Managed skills: 8 current/refreshed."
    );
    let mut missing = office.clone();
    missing["skipped"]
        .as_array_mut()
        .unwrap()
        .push(json!("/c/tmux-team"));
    missing["conflicts"] = json!(["/d/tmt-inbox"]);
    assert_eq!(
        skills_summary(&missing),
        "Managed skills: 8 current/refreshed, 1 missing, 1 conflicts preserved."
    );
}
