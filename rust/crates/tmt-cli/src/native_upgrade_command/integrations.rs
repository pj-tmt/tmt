//! The activated executable owns provider refresh; the old updater only
//! validates and presents its bounded advisory report. Never retry this child.
use super::*;
use tmt_adapters::setup::{IntegrationRefreshReport, RefreshStatus};

pub(super) fn run(executable: &Path, runner: &impl CommandRunner) -> IntegrationRefreshReport {
    let output = runner.execute(CommandRequest {
        program: executable.as_os_str(),
        args: &["__native-refresh-integrations".into(), "--json".into()],
        input: &[],
        deadline: Instant::now() + Duration::from_secs(30),
        max_output_bytes: 4 * 1024 * 1024,
    });
    match output {
        Ok(output) if output.stderr.is_empty() => crate::setup_command::refresh::parse(&output.stdout)
            .unwrap_or_else(|| crate::setup_command::refresh::unavailable("The new executable returned an invalid integration report; provider settings may already have been updated. Review setup before retrying.".into())),
        _ => crate::setup_command::refresh::unavailable("The new executable did not confirm integration refresh; provider settings may already have been updated. Review setup before retrying.".into()),
    }
}

pub(super) fn hints(report: &IntegrationRefreshReport) -> Vec<String> {
    report
        .integrations
        .iter()
        .filter_map(|row| {
            row.next_step.as_ref().map(|next| {
                format!(
                    "{} integration ({}): {}; run {next}",
                    row.provider,
                    row.settings_path.display(),
                    row.message
                )
            })
        })
        .chain(report.error.iter().map(|error| {
            format!(
                "Driver integrations: {}; run {}",
                error.message, error.next_step
            )
        }))
        .collect()
}

pub(super) fn write(
    output: &mut impl Write,
    terminal: tmt_cli_style::Terminal,
    report: &IntegrationRefreshReport,
) -> io::Result<()> {
    use tmt_cli_style::table::{Cell, Column, Table};
    let mut table = Table::new(&[Column::Name, Column::Fixed]);
    for row in &report.integrations {
        let status = match row.status {
            RefreshStatus::Unchanged => "unchanged",
            RefreshStatus::Updated => "updated",
            RefreshStatus::NeedsApproval => "needs approval",
            RefreshStatus::Skipped => "skipped",
        };
        table.row([
            Cell::from(format!("{} integration", row.provider)),
            Cell::from(status),
        ]);
    }
    table.write(output, terminal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, ffi::OsStr};
    use tmt_adapters::process::{CommandError, CommandOutput};

    struct Runner {
        calls: Cell<u32>,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        fail: bool,
    }
    impl CommandRunner for Runner {
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            self.calls.set(self.calls.get() + 1);
            assert_eq!(request.program, OsStr::new("/new/release/tmt"));
            assert_eq!(
                request.args,
                ["__native-refresh-integrations", "--json"].map(std::ffi::OsString::from)
            );
            assert!(request.input.is_empty());
            assert!(request.deadline <= Instant::now() + Duration::from_secs(30));
            assert_eq!(request.max_output_bytes, 4 * 1024 * 1024);
            if self.fail {
                return UnixCommandRunner.execute(CommandRequest {
                    program: OsStr::new("/bin/sh"),
                    args: &["-c".into(), "exit 2".into()],
                    input: &[],
                    deadline: request.deadline,
                    max_output_bytes: 1024,
                });
            }
            Ok(CommandOutput {
                stdout: self.stdout.clone(),
                stderr: self.stderr.clone(),
            })
        }
    }

    #[test]
    fn only_the_selected_new_cli_refreshes_and_its_report_is_preserved() {
        let runner = Runner {
            calls: Cell::new(0),
            stdout: br#"{"integrations":[],"error":null}"#.to_vec(),
            stderr: Vec::new(),
            fail: false,
        };
        let report = run(Path::new("/new/release/tmt"), &runner);
        assert!(report.error.is_none());
        assert_eq!(runner.calls.get(), 1);
    }

    #[test]
    fn malformed_failed_or_noisy_refresh_is_advisory_and_never_replayed() {
        for (stdout, stderr, fail) in [
            (b"bad".to_vec(), Vec::new(), false),
            (
                br#"{"integrations":[],"error":null}"#.to_vec(),
                b"unexpected".to_vec(),
                false,
            ),
            (Vec::new(), Vec::new(), true),
        ] {
            let runner = Runner {
                calls: Cell::new(0),
                stdout,
                stderr,
                fail,
            };
            let report = run(Path::new("/new/release/tmt"), &runner);
            assert!(report.error.is_some());
            assert_eq!(runner.calls.get(), 1);
            assert!(hints(&report)[0].ends_with("run tmt setup"));
        }
    }

    #[test]
    fn provider_statuses_and_recovery_steps_share_the_typed_report() {
        let report = crate::setup_command::refresh::parse(br#"{"integrations":[
            {"provider":"claude","settingsPath":"/private/claude/settings.json","status":"updated","reason":"refreshed","message":"","nextStep":null},
            {"provider":"codex","settingsPath":"/private/codex/hooks.json","status":"needsApproval","reason":"edited_hook","message":"Edited hook preserved","nextStep":"tmt setup codex"}
        ],"error":null}"#).unwrap();
        let mut output = Vec::new();
        write(&mut output, tmt_cli_style::Terminal::PLAIN, &report).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("claude integration"));
        assert!(output.contains("updated"));
        assert!(output.contains("needs approval"));
        let hints = hints(&report);
        assert_eq!(hints.len(), 1);
        assert!(hints[0].contains("/private/codex/hooks.json"));
        assert!(hints[0].ends_with("run tmt setup codex"));
    }
}
