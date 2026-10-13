//! The database-rename phase of `tmt upgrade` (#2292). Only the newly activated
//! executable renames; the updater reads its bounded report and never fails the
//! upgrade because of it.

use super::*;

const STATUSES: [&str; 5] = ["absent", "current", "renamed", "in-use", "refused"];

/// Runs the phase and returns the document the upgrade reports as
/// `databaseRename`: a `status`, plus a `reason` when there is one to give.
pub(super) fn run(executable: &Path, runner: &impl CommandRunner) -> Value {
    let result = runner.execute(CommandRequest {
        program: executable.as_os_str(),
        args: &["__native-rename-database".into(), "--json".into()],
        input: &[],
        deadline: Instant::now() + Duration::from_secs(60),
        max_output_bytes: 16 * 1024,
    });
    let (output, failed) = match result {
        Ok(output) => (output, false),
        Err(mut error) => {
            let exited = matches!(
                error.kind,
                CommandFailure::Exit {
                    code: Some(1),
                    signal: None
                }
            ) && !error.cleanup_failed();
            match error.output.take().filter(|_| exited) {
                Some(output) => (output, true),
                None => return failed_document("the new executable did not complete"),
            }
        }
    };
    if !output.stderr.is_empty() {
        return failed_document("the new executable returned an invalid report");
    }
    let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) else {
        return failed_document("the new executable returned an invalid report");
    };
    if failed {
        let message = value["error"]["message"].as_str().unwrap_or("it failed");
        return failed_document(message);
    }
    let status = value["status"].as_str().unwrap_or_default();
    let reason = value.get("reason").and_then(Value::as_str);
    let expected_keys = 1 + usize::from(reason.is_some());
    if !STATUSES.contains(&status)
        || value.as_object().map(|object| object.len()) != Some(expected_keys)
        || reason.is_some() != (status == "refused")
    {
        return failed_document("the new executable returned an invalid report");
    }
    value
}

fn failed_document(reason: &str) -> Value {
    json!({"status": "failed", "reason": reason})
}

/// The one line the user needs, when the rename did not complete.
pub(super) fn hint(document: &Value) -> Option<String> {
    let reason = document["reason"].as_str().unwrap_or_default();
    match document["status"].as_str()? {
        "in-use" => Some(
            "The Core database keeps its former name while another process holds it. Stop the processes that use it (channel sessions and serves), then finish the rename with: tmt upgrade"
                .into(),
        ),
        "refused" => Some(format!(
            "The Core database keeps its former name: {reason}. Resolve that, then finish the rename with: tmt upgrade"
        )),
        "failed" => Some(format!(
            "The Core database rename did not run: {reason}. Retry with: tmt upgrade"
        )),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "The Core database keeps its former name while another process holds it. Stop the processes that use it (channel sessions and serves), then finish the rename with: tmt upgrade",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "The Core database keeps its former name: {reason}. Resolve that, then finish the rename with: tmt upgrade",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "The Core database rename did not run: {reason}. Retry with: tmt upgrade",
        &[""],
        &[],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_adapters::process::{CommandError, CommandOutput};

    struct Scripted<F>(F);
    impl<F: Fn() -> Result<CommandOutput, CommandError>> CommandRunner for Scripted<F> {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            assert_eq!(request.program, "/new/tmt");
            assert_eq!(
                request.args,
                &[
                    std::ffi::OsString::from("__native-rename-database"),
                    "--json".into()
                ]
            );
            assert!(request.input.is_empty());
            (self.0)()
        }
    }

    /// A real child, so failures carry the process runner's own evidence.
    struct Shell(&'static str);
    impl CommandRunner for Shell {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            UnixCommandRunner.execute(CommandRequest {
                program: std::ffi::OsStr::new("/bin/sh"),
                args: &["-c".into(), self.0.into()],
                input: &[],
                deadline: request.deadline,
                max_output_bytes: request.max_output_bytes,
            })
        }
    }

    fn run_with(stdout: &'static str) -> Value {
        run(
            Path::new("/new/tmt"),
            &Scripted(|| {
                Ok(CommandOutput {
                    stdout: stdout.as_bytes().to_vec(),
                    stderr: Vec::new(),
                })
            }),
        )
    }

    #[test]
    fn a_valid_report_passes_through_and_only_unfinished_work_has_a_hint() {
        for status in ["absent", "current", "renamed"] {
            let stdout: &'static str = Box::leak(format!(r#"{{"status":"{status}"}}"#).into());
            let document = run_with(stdout);
            assert_eq!(document, json!({"status": status}));
            assert_eq!(hint(&document), None);
        }
        let held = run_with(r#"{"status":"in-use"}"#);
        assert!(
            hint(&held)
                .unwrap()
                .contains("finish the rename with: tmt upgrade")
        );
        let refused = run_with(r#"{"status":"refused","reason":"a second database exists"}"#);
        assert!(hint(&refused).unwrap().contains("a second database exists"));
    }

    #[test]
    fn anything_else_is_a_failed_phase_that_never_fails_the_upgrade() {
        for stdout in [
            "not json",
            r#"{"status":"renamed","extra":true}"#,
            r#"{"status":"refused"}"#,
            r#"{"status":"in-use","reason":"x"}"#,
            r#"{"status":"mystery"}"#,
            r#"[]"#,
        ] {
            let document = run_with(stdout);
            assert_eq!(document["status"], "failed", "{stdout}");
            assert!(hint(&document).unwrap().contains("did not run"));
        }
        for (script, reason) in [
            ("exit 3", "did not complete"),
            (
                r#"printf '{"error":{"code":"DATABASE_RENAME_FAILED","message":"boom"}}'; exit 1"#,
                "boom",
            ),
        ] {
            let failed = run(Path::new("/new/tmt"), &Shell(script));
            assert_eq!(failed["status"], "failed", "{script}");
            assert!(
                failed["reason"].as_str().unwrap().contains(reason),
                "{failed}"
            );
        }
        let noisy = run(
            Path::new("/new/tmt"),
            &Scripted(|| {
                Ok(CommandOutput {
                    stdout: br#"{"status":"renamed"}"#.to_vec(),
                    stderr: b"warning".to_vec(),
                })
            }),
        );
        assert_eq!(noisy["status"], "failed");
    }
}
