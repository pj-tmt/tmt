//! `tmt workspace restore --host pty`: start the recorded built-in members one at a time,
//! each through ordinary `tmt resume --host pty` in its recorded folder. Restore never
//! stops, signals or rebinds a running member, so a rerun is safe and skips the live ones.

use super::write_report;
use crate::{invocation::OutputMode, output::Failure, output::shell_word};
use std::{
    collections::HashSet,
    ffi::OsString,
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    process::{CommandFailure, CommandRequest, UnixCommandRunner},
    workspace::{self, Store},
};
use tmt_core::{
    host::HostKind,
    workspace::{
        SnapshotSelector, StoredIdentity,
        builtin::{BuiltinMemberAction, BuiltinSnapshot, member_plan},
    },
};

/// The last built-in restore's report, beside the tmux one. Each run replaces it.
const LAST_REPORT: &str = "restore-last-pty.json";
/// One member's start, and the whole restore, are bounded; members not reached wait.
const MEMBER_BUDGET: Duration = Duration::from_secs(20);
const TOTAL_BUDGET: Duration = Duration::from_secs(120);
/// Less than this left is not worth a start that would only end unconfirmed.
const MINIMUM_START: Duration = Duration::from_secs(1);
const MAX_START_OUTPUT: usize = 64 * 1024;
const TAIL_BYTES: usize = 2 * 1024;

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "failed (start not confirmed in time; check tmt workspace show --host pty before retrying)",
        &[" before"],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "tmt resume --host {}{} -- {}",
        &[""],
        &[("--host {}{}", "--host pty --new"), ("-- {}", "-- alpha")],
    ),
    crate::cli_style_tests::HintSpec::core(
        "Selected {selection} has {} members; protected {} has {}. Nothing was restored. To choose the smaller set explicitly, run tmt workspace restore {selection} --expect {} --host pty{}.",
        &["."],
        &[
            ("{selection}", "work"),
            (
                "--expect {}",
                "--expect 0000000000000000000000000000000000000000000000000000000000000000",
            ),
            ("{}.", "."),
        ],
    ),
    crate::cli_style_tests::HintSpec::core(
        "Restored, but tmt could not record this as the recovery choice: {error}. Check tmt workspace ls --host pty before restoring again.",
        &[" before"],
        // The first mention names the program in prose, not a command.
        &[("tmt could", "it could")],
    ),
];

/// What restore did with one recorded member.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Restored {
    identity: String,
    name: String,
    cwd: Option<String>,
    /// `started`, `skipped`, `needs_you` or `failed`.
    status: &'static str,
    reason: &'static str,
    message: String,
    /// The exact command that does this member's part by hand.
    retry: Option<String>,
    /// The end of a failed start's output.
    tail: Option<String>,
}

/// One start through ordinary resume.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Start {
    Started,
    Failed(String),
    /// The start did not answer in time; the member may still be starting.
    Unconfirmed,
}

fn retry(cwd: Option<&str>, flag: Option<&str>, name: &str) -> String {
    let resume = format!(
        "tmt resume --host {}{} -- {}",
        HostKind::pty().as_str(),
        flag.map(|flag| format!(" {flag}")).unwrap_or_default(),
        shell_word(name)
    );
    match cwd {
        Some(cwd) => format!("cd {} && {resume}", shell_word(cwd)),
        None => resume,
    }
}

/// Members are rechecked and started in recorded order. `check` reads current identities and
/// the live built-in members at each member's turn; `start` runs one ordinary resume.
fn restore(
    snapshot: &BuiltinSnapshot,
    fresh: bool,
    deadline: Instant,
    mut check: impl FnMut() -> io::Result<(Vec<StoredIdentity>, HashSet<String>)>,
    folder_exists: impl Fn(&str) -> bool,
    mut start: impl FnMut(&str, &Path, bool, Instant) -> Start,
    mut progress: impl FnMut(usize, usize, &Restored),
) -> Vec<Restored> {
    let total = snapshot.members.len();
    let mut restored = Vec::with_capacity(total);
    for (index, member) in snapshot.members.iter().enumerate() {
        let cwd = member.cwd.as_deref();
        let mut row = Restored {
            identity: member.identity.id.clone(),
            name: member.identity.name.clone(),
            cwd: member.cwd.clone(),
            status: "needs_you",
            reason: "",
            message: String::new(),
            retry: None,
            tail: None,
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        let checked = if remaining < MINIMUM_START {
            None
        } else {
            Some(check())
        };
        let action = match &checked {
            None => None,
            Some(Err(_)) => {
                row.reason = "unchecked";
                row.message = "needs you (its current state could not be read)".into();
                row.retry = Some(retry(cwd, None, &row.name));
                None
            }
            Some(Ok((identities, live))) => {
                let plan = member_plan(snapshot, identities, live, &folder_exists);
                let planned = &plan[index];
                if let Some(record) = planned.identity {
                    row.name = record.entry.identity.name.clone();
                }
                Some(planned.action)
            }
        };
        let flag = fresh.then_some("--new");
        match (checked.is_none(), action) {
            (true, _) => {
                row.reason = "budget";
                let command = retry(cwd, flag, &row.name);
                row.message = format!("needs you (the restore ran out of time; {command})");
                row.retry = Some(command);
            }
            (false, None) => {}
            (false, Some(BuiltinMemberAction::IdentityMissing)) => {
                row.reason = "identity_missing";
                row.message = "needs you (original identity is missing)".into();
            }
            (false, Some(BuiltinMemberAction::Live)) => {
                row.status = "skipped";
                row.reason = "live";
                row.message = "skipped (already running)".into();
            }
            (false, Some(BuiltinMemberAction::FolderMissing)) => {
                row.reason = "folder_missing";
                let command = retry(cwd, flag, &row.name);
                row.message = format!(
                    "needs you (folder {} is missing; recreate it, then {command})",
                    shell_word(cwd.unwrap_or_default())
                );
                row.retry = Some(command);
            }
            (false, Some(BuiltinMemberAction::NoRememberedSession)) if !fresh => {
                row.reason = "no_remembered_session";
                let command = retry(cwd, Some("--new"), &row.name);
                row.message = format!("needs you (no remembered conversation; {command})");
                row.retry = Some(command);
            }
            (false, Some(BuiltinMemberAction::StaleRequiresRetry)) if !fresh => {
                row.reason = "stale";
                let command = retry(cwd, Some("--retry"), &row.name);
                row.message = format!("needs you (stale conversation; {command})");
                row.retry = Some(command);
            }
            (false, Some(_)) => {
                // A member whose folder was not recorded starts where the restore runs.
                let directory = Path::new(cwd.unwrap_or("."));
                let until = deadline.min(Instant::now() + MEMBER_BUDGET);
                match start(&row.name, directory, fresh, until) {
                    Start::Started => {
                        row.status = "started";
                        row.reason = if fresh { "fresh" } else { "resume_requested" };
                        row.message = if fresh {
                            "started (new conversation)".into()
                        } else {
                            "started (resume requested)".into()
                        };
                    }
                    Start::Failed(tail) => {
                        row.status = "failed";
                        row.reason = "start_failed";
                        row.message = "failed (start refused)".into();
                        row.retry = Some(retry(cwd, flag, &row.name));
                        row.tail = Some(tail);
                    }
                    Start::Unconfirmed => {
                        row.status = "failed";
                        row.reason = "unconfirmed";
                        row.message = "failed (start not confirmed in time; check tmt workspace show --host pty before retrying)".into();
                    }
                }
            }
        }
        progress(index + 1, total, &row);
        restored.push(row);
    }
    restored
}

/// The last bytes of a failed start's output, on a character boundary.
fn tail(output: &[u8]) -> String {
    let text = String::from_utf8_lossy(output);
    let text = text.trim();
    let mut start = text.len().saturating_sub(TAIL_BYTES);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_owned()
}

/// One ordinary resume, as a child of this command, in the member's folder.
fn resume(
    executable: &Path,
    global_dir: &Path,
    name: &str,
    directory: &Path,
    fresh: bool,
    deadline: Instant,
) -> Start {
    let mut args: Vec<OsString> = vec![
        "resume".into(),
        "--host".into(),
        HostKind::pty().as_str().into(),
    ];
    if fresh {
        args.push("--new".into());
    }
    args.extend(["--".into(), name.into()]);
    // The same Core data directory, even when TMT_HOME was relative to the caller's folder.
    let home = std::path::absolute(global_dir).unwrap_or_else(|_| global_dir.to_path_buf());
    let result = UnixCommandRunner.execute_in(
        CommandRequest {
            program: executable.as_os_str(),
            args: &args,
            input: &[],
            deadline,
            max_output_bytes: MAX_START_OUTPUT,
        },
        directory,
        &[("TMT_HOME".into(), home.into_os_string())],
    );
    match result {
        Ok(_) => Start::Started,
        Err(error) if error.kind == CommandFailure::Timeout => Start::Unconfirmed,
        Err(error) => Start::Failed(match &error.output {
            Some(output) => tail(if output.stderr.is_empty() {
                &output.stdout
            } else {
                &output.stderr
            }),
            None => error.to_string(),
        }),
    }
}

pub fn execute(
    name: Option<&SnapshotSelector>,
    expect: Option<&str>,
    fresh: bool,
    mode: OutputMode,
) -> io::Result<u8> {
    let started_at = Instant::now();
    let paths = match ConfigPaths::discover() {
        Ok(paths) => paths,
        Err(error) => return Failure::from(error).publish(mode),
    };
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    let outcome: Result<_, Failure> = (|| {
        let document = workspace::saved::acquire_selected::<BuiltinSnapshot>(
            &paths,
            Store::Builtin,
            name,
            expect,
        )
        .map_err(|error| {
            crate::workspace_snapshot_command::failure(
                error,
                "WORKSPACE_RESTORE_UNAVAILABLE",
                false,
            )
        })?;
        if document.requires_confirmation {
            let selection = name
                .map(|name| name.as_str().to_owned())
                .or_else(|| {
                    document
                        .generation
                        .as_ref()
                        .map(|generation| generation.id.as_str().to_owned())
                })
                .expect("selected recovery");
            let baseline = document
                .baseline
                .as_ref()
                .expect("smaller than protected generation");
            let refusal = format!(
                "Selected {selection} has {} members; protected {} has {}. Nothing was restored. To choose the smaller set explicitly, run tmt workspace restore {selection} --expect {} --host pty{}.",
                document.snapshot.agent_count(),
                baseline.id.as_str(),
                baseline.agents,
                document.digest,
                if fresh { " --fresh" } else { "" }
            );
            let question = format!(
                "Restore this smaller set ({} members)",
                document.snapshot.agent_count()
            );
            let mut prompt = tmt_cli_style::stream::stderr();
            if !crate::consent::ask(
                &mut prompt,
                false,
                mode,
                crate::consent::Consent {
                    code: "WORKSPACE_SMALLER_CONSENT_REQUIRED",
                    refusal: &refusal,
                    question: &question,
                    declined: "Nothing was restored.",
                },
                |error| Failure::new("WORKSPACE_RESTORE_UNAVAILABLE", error.to_string(), 1),
            )? {
                return Err(Failure::new(
                    "WORKSPACE_RESTORE_DECLINED",
                    "Nothing was restored.",
                    1,
                ));
            }
        }
        let executable = tmt_adapters::core_executable::selected()
            .map_err(|error| Failure::new("WORKSPACE_RESTORE_UNAVAILABLE", error.to_string(), 1))?;
        let mut progress_failure = None;
        let members = restore(
            &document.snapshot,
            fresh,
            started_at + TOTAL_BUDGET,
            || {
                workspace::builtin_plan_input(
                    &paths,
                    &document.snapshot,
                    Instant::now() + workspace::CAPTURE_BUDGET,
                )
            },
            |cwd| Path::new(cwd).is_dir(),
            |name, directory, fresh, deadline| {
                resume(
                    &executable,
                    &paths.global_dir,
                    name,
                    directory,
                    fresh,
                    deadline,
                )
            },
            |done, total, row| {
                // Progress is for a person; a script reads the one document at the end.
                if !mode.json && progress_failure.is_none() {
                    let line = format!("  [{done}/{total}] {}: {}", row.name, row.message);
                    let written = writeln!(output, "{line}").and_then(|()| output.flush());
                    if let Err(error) = written {
                        progress_failure = Some(error);
                    }
                }
            },
        );
        if let Some(error) = progress_failure {
            return Err(Failure::new(
                "WORKSPACE_RESTORE_UNAVAILABLE",
                error.to_string(),
                1,
            ));
        }
        let failed = members.iter().any(|row| row.status == "failed");
        let unreached = members.iter().any(|row| row.reason == "budget");
        let mut failures = Vec::new();
        // Only an explicit choice that restored cleanly becomes the recovery choice.
        if !failed
            && !unreached
            && name.is_some()
            && let Some(selected) = document.generation.as_ref()
            && let Err(error) = workspace::saved::accept_generation(
                &paths,
                Store::Builtin,
                &selected.id,
                document.baseline.as_ref().map(|generation| &generation.id),
            )
        {
            failures.push(format!("Restored, but tmt could not record this as the recovery choice: {error}. Check tmt workspace ls --host pty before restoring again."));
        }
        Ok((document, members, failures))
    })();
    let (document, members, failures) = match outcome {
        Ok(value) => value,
        Err(error) => return error.publish(mode),
    };
    let failed = members.iter().any(|row| row.status == "failed");
    let partial = failed || members.iter().any(|row| row.status == "needs_you");
    let mut report = serde_json::json!({
        "version": 1,
        "host": HostKind::pty().as_str(),
        "name": name.map(|name| name.as_str()),
        "generation": document.generation.as_ref().map(|generation| generation.id.as_str()),
        "digest": document.digest,
        "fresh": fresh,
        "status": if partial { "partial" } else { "completed" },
        "members": members.iter().map(|row| serde_json::json!({
            "identityId": row.identity, "name": row.name, "cwd": row.cwd,
            "status": row.status, "reason": row.reason, "message": row.message,
            "retry": row.retry, "tail": row.tail,
        })).collect::<Vec<_>>(),
        "failures": failures,
    });
    let written = write_report(&paths, LAST_REPORT, &mut report);
    if mode.json {
        report["report"] = written.into();
        writeln!(output, "{report}")?;
    } else {
        let count = |status: &str| members.iter().filter(|row| row.status == status).count();
        writeln!(
            output,
            "Built-in members: {} started, {} already running, {} need you, {} failed.",
            count("started"),
            count("skipped"),
            count("needs_you"),
            count("failed")
        )?;
        for row in members.iter().filter(|row| row.tail.is_some()) {
            writeln!(
                output,
                "  {}: {}",
                row.name,
                row.tail.as_deref().unwrap_or_default()
            )?;
        }
        for failure in &failures {
            writeln!(output, "  {failure}")?;
        }
        if let Some(path) = written.as_deref().filter(|_| partial) {
            writeln!(output, "  Full report: {path}")?;
        }
    }
    Ok(u8::from(failed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_core::{
        binding::{
            BindingEntry,
            session::{
                HarnessId, ProviderSessionId, RememberedSession, RuntimeMode, SessionPreferences,
            },
        },
        identity::{Identity, Lifetime},
        workspace::{WorkspaceIdentity, builtin::BuiltinMember},
    };

    fn snapshot(ids: &[&str]) -> BuiltinSnapshot {
        BuiltinSnapshot {
            captured_at_ms: 1,
            members: ids
                .iter()
                .map(|id| BuiltinMember {
                    cwd: Some(format!("/work/{id}")),
                    identity: WorkspaceIdentity {
                        id: format!("uuid-{id}"),
                        name: id.to_string(),
                        lifetime: "saved".into(),
                        binding: format!("binding-{id}"),
                        harness: Some("codex".into()),
                        session: None,
                        mode: None,
                        channel: None,
                    },
                })
                .collect(),
        }
    }

    fn record(id: &str, remembered: bool) -> StoredIdentity {
        StoredIdentity {
            entry: BindingEntry {
                identity: Identity {
                    id: format!("uuid-{id}"),
                    name: id.into(),
                    canonical_name: id.into(),
                    lifetime: Lifetime::Saved,
                    created_at: "date".into(),
                    updated_at: "date".into(),
                },
                binding: None,
            },
            preferences: SessionPreferences {
                remembered: remembered.then(|| RememberedSession {
                    harness: HarnessId::new("codex").unwrap(),
                    mode: RuntimeMode::new("independent").unwrap(),
                    provider_session: ProviderSessionId::new("session").unwrap(),
                    state: None,
                    stale_at_ms: None,
                    resume_pending_at_ms: None,
                }),
                ..SessionPreferences::default()
            },
        }
    }

    #[test]
    fn members_start_one_at_a_time_in_their_folders_and_a_rerun_skips_the_live_ones() {
        let snapshot = snapshot(&["a", "b", "c"]);
        let records = || vec![record("a", true), record("b", true), record("c", false)];
        let live = std::cell::RefCell::new(HashSet::from(["uuid-b".to_owned()]));
        let starts = std::cell::RefCell::new(Vec::new());
        let mut lines = Vec::new();
        let run = |lines: &mut Vec<String>| {
            restore(
                &snapshot,
                false,
                Instant::now() + TOTAL_BUDGET,
                || Ok((records(), live.borrow().clone())),
                |_| true,
                |name, directory, fresh, _| {
                    starts
                        .borrow_mut()
                        .push((name.to_owned(), directory.to_owned(), fresh));
                    // The started member is live at the next member's recheck.
                    live.borrow_mut().insert(format!("uuid-{name}"));
                    Start::Started
                },
                |done, total, row| lines.push(format!("{done}/{total} {}", row.status)),
            )
        };
        let first = run(&mut lines);
        assert_eq!(
            starts.borrow().as_slice(),
            [("a".to_owned(), Path::new("/work/a").to_owned(), false)]
        );
        assert_eq!(
            first
                .iter()
                .map(|row| (row.status, row.reason))
                .collect::<Vec<_>>(),
            [
                ("started", "resume_requested"),
                ("skipped", "live"),
                ("needs_you", "no_remembered_session"),
            ]
        );
        assert_eq!(
            first[2].retry.as_deref(),
            Some("cd '/work/c' && tmt resume --host pty --new -- c")
        );
        assert_eq!(lines, ["1/3 started", "2/3 skipped", "3/3 needs_you"]);
        // The same restore again: a and b are running and untouched; nothing new starts.
        let again = run(&mut Vec::new());
        assert_eq!(starts.borrow().len(), 1);
        assert_eq!(
            again.iter().map(|row| row.status).collect::<Vec<_>>(),
            ["skipped", "skipped", "needs_you"]
        );
    }

    #[test]
    fn members_not_reached_in_time_are_reported_with_their_retry_never_dropped() {
        let snapshot = snapshot(&["a", "b", "c"]);
        let records = || vec![record("a", true), record("b", true), record("c", true)];
        let deadline = Instant::now() + MINIMUM_START + Duration::from_millis(200);
        let mut started = Vec::new();
        let restored = restore(
            &snapshot,
            true,
            deadline,
            || Ok((records(), HashSet::new())),
            |_| true,
            |name, _, fresh, until| {
                assert!(fresh);
                assert!(until <= deadline);
                started.push(name.to_owned());
                // The first start uses up the rest of the budget.
                std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                Start::Started
            },
            |_, _, _| {},
        );
        assert_eq!(started, ["a"]);
        assert_eq!(restored.len(), 3);
        assert_eq!(
            (restored[0].status, restored[0].reason),
            ("started", "fresh")
        );
        for row in &restored[1..] {
            assert_eq!((row.status, row.reason), ("needs_you", "budget"));
            assert_eq!(
                row.retry.as_deref(),
                Some(
                    format!(
                        "cd '/work/{}' && tmt resume --host pty --new -- {}",
                        row.name, row.name
                    )
                    .as_str()
                )
            );
        }
    }

    #[test]
    fn a_refused_or_unconfirmed_start_fails_with_its_output_and_the_rest_continue() {
        let snapshot = snapshot(&["a", "b", "c"]);
        let records = || vec![record("a", true), record("b", true), record("c", true)];
        let mut outcomes = vec![
            Start::Failed("no such driver".into()),
            Start::Unconfirmed,
            Start::Started,
        ]
        .into_iter();
        let restored = restore(
            &snapshot,
            false,
            Instant::now() + TOTAL_BUDGET,
            || Ok((records(), HashSet::new())),
            |cwd| cwd != "/work/missing",
            |_, _, _, _| outcomes.next().unwrap(),
            |_, _, _| {},
        );
        assert_eq!(
            restored
                .iter()
                .map(|row| (row.status, row.reason))
                .collect::<Vec<_>>(),
            [
                ("failed", "start_failed"),
                ("failed", "unconfirmed"),
                ("started", "resume_requested"),
            ]
        );
        assert_eq!(restored[0].tail.as_deref(), Some("no such driver"));
        // An unconfirmed member may be starting: no retry command invites a second one.
        assert!(restored[1].retry.is_none());
    }

    #[test]
    fn identity_folder_and_stale_problems_wait_for_you_without_a_start() {
        let mut snapshot = snapshot(&["a", "b", "c"]);
        snapshot.members[1].cwd = Some("/work/missing".into());
        let records = || {
            let mut stale = record("c", true);
            stale.preferences.remembered.as_mut().unwrap().stale_at_ms = Some(5);
            vec![record("b", true), stale]
        };
        let restored = restore(
            &snapshot,
            false,
            Instant::now() + TOTAL_BUDGET,
            || Ok((records(), HashSet::new())),
            |cwd| cwd != "/work/missing",
            |_, _, _, _| unreachable!("nothing starts"),
            |_, _, _| {},
        );
        assert_eq!(
            restored.iter().map(|row| row.reason).collect::<Vec<_>>(),
            ["identity_missing", "folder_missing", "stale"]
        );
        assert_eq!(
            restored[2].retry.as_deref(),
            Some("cd '/work/c' && tmt resume --host pty --retry -- c")
        );
    }

    #[test]
    fn a_failed_start_keeps_only_the_end_of_its_output() {
        let long = format!("{}end", "é".repeat(TAIL_BYTES));
        let kept = tail(long.as_bytes());
        assert!(kept.len() <= TAIL_BYTES && kept.ends_with("end"));
    }
}
