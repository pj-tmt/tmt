//! The one-time database compaction phase of `tmt upgrade` (#5036). It runs
//! after the rename, in the newly activated executable; a refusal or failure
//! never fails the upgrade, and `tmt storage` keeps showing the step as pending.

use super::*;

/// Runs the phase and returns the document the upgrade reports as
/// `databaseCompaction`: a `status`, plus `reason` for a refusal and the sizes
/// for a compaction.
pub(super) fn run(executable: &Path, runner: &impl CommandRunner) -> Value {
    database::run_step(executable, runner, "__native-compact-database", valid)
}

fn valid(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let has = |keys: &[&str]| {
        object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key))
    };
    match object.get("status").and_then(Value::as_str) {
        Some("absent" | "current" | "in-use") => has(&["status"]),
        Some("compacted") => {
            has(&["status", "bytesBefore", "bytesAfter"])
                && object["bytesBefore"].is_u64()
                && object["bytesAfter"].is_u64()
        }
        Some("refused") => has(&["status", "reason"]) && object["reason"].is_string(),
        _ => false,
    }
}

/// The one line the user needs when the compaction did not run.
pub(super) fn hint(document: &Value) -> Option<String> {
    let reason = document["reason"].as_str().unwrap_or_default();
    match document["status"].as_str()? {
        "in-use" => Some(
            "The one-time tmt.db compaction did not run because another process was writing; nothing changed. Check tmt storage, then finish it with: tmt upgrade"
                .into(),
        ),
        "refused" => Some(format!(
            "The one-time tmt.db compaction did not run: {reason}; nothing changed. Check tmt storage, then finish it with: tmt upgrade"
        )),
        "failed" => Some(format!(
            "The one-time tmt.db compaction did not run: {reason}. Check tmt storage, then retry with: tmt upgrade"
        )),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "The one-time tmt.db compaction did not run because another process was writing; nothing changed. Check tmt storage, then finish it with: tmt upgrade",
        &[", then", ""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "The one-time tmt.db compaction did not run: {reason}; nothing changed. Check tmt storage, then finish it with: tmt upgrade",
        &[", then", ""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "The one-time tmt.db compaction did not run: {reason}. Check tmt storage, then retry with: tmt upgrade",
        &[", then", ""],
        &[],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_adapters::process::{CommandError, CommandOutput};

    struct Scripted(&'static str);
    impl CommandRunner for Scripted {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            assert_eq!(request.program, "/new/tmt");
            assert_eq!(
                request.args,
                &[
                    std::ffi::OsString::from("__native-compact-database"),
                    "--json".into()
                ]
            );
            Ok(CommandOutput {
                stdout: self.0.as_bytes().to_vec(),
                stderr: Vec::new(),
            })
        }
    }

    fn run_with(stdout: &'static str) -> Value {
        run(Path::new("/new/tmt"), &Scripted(stdout))
    }

    #[test]
    fn a_valid_report_passes_through_and_only_unfinished_work_has_a_hint() {
        for stdout in [
            r#"{"status":"absent"}"#,
            r#"{"status":"current"}"#,
            r#"{"status":"compacted","bytesBefore":150000000,"bytesAfter":9000000}"#,
        ] {
            let document = run_with(stdout);
            assert_eq!(
                document,
                serde_json::from_str::<Value>(stdout).unwrap(),
                "{stdout}"
            );
            assert_eq!(hint(&document), None);
        }
        let held = hint(&run_with(r#"{"status":"in-use"}"#)).unwrap();
        assert!(held.contains("another process was writing"), "{held}");
        assert!(held.ends_with("Check tmt storage, then finish it with: tmt upgrade"));
        let refused = hint(&run_with(
            r#"{"status":"refused","reason":"compaction needs 2 free bytes"}"#,
        ))
        .unwrap();
        assert!(refused.contains("compaction needs 2 free bytes; nothing changed"));
    }

    #[test]
    fn anything_else_is_a_failed_phase_that_never_fails_the_upgrade() {
        for stdout in [
            "not json",
            r#"{"status":"compacted"}"#,
            r#"{"status":"compacted","bytesBefore":"1","bytesAfter":2}"#,
            r#"{"status":"current","extra":true}"#,
            r#"{"status":"refused"}"#,
            r#"{"status":"in-use","reason":"x"}"#,
            r#"{"status":"renamed"}"#,
            r#"[]"#,
        ] {
            let document = run_with(stdout);
            assert_eq!(document["status"], "failed", "{stdout}");
            assert!(hint(&document).unwrap().contains("retry with: tmt upgrade"));
        }
    }
}
