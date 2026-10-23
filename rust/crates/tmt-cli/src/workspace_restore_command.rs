//! Two-phase recovery reuses ordinary resume and generic external dispatch.

use crate::{invocation::OutputMode, output::Failure};
use std::{
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::{config::ConfigPaths, host::Host, storage::Storage, workspace};
use tmt_core::workspace::{
    StoredIdentity, WorkspaceSnapshot,
    plan::{WorkspacePaneAction, restore_plan},
};

/// The last restore's report, in the TMT data directory. Each run replaces it.
const LAST_REPORT: &str = "restore-last.json";
const MAX_REPORT_BYTES: usize = 1024 * 1024;

/// A large workspace gets more time than a small one: 20 s plus half a second per pane, at most
/// two minutes. A fixed budget turned a long restore into a refusal that named no cause.
fn layout_deadline(panes: usize) -> Instant {
    let millis = (20_000 + 500 * panes as u64).min(120_000);
    Instant::now() + Duration::from_millis(millis)
}

/// Writes the report and returns its path, or nothing when it cannot be written.
fn write_report(paths: &ConfigPaths, document: &mut serde_json::Value) -> Option<String> {
    let path = paths.global_dir.join(LAST_REPORT);
    let mut bytes = serde_json::to_vec(document).ok()?;
    if bytes.len() > MAX_REPORT_BYTES
        && let Some(object) = document.as_object_mut()
    {
        for key in ["windows", "panes", "revival"] {
            object.remove(key);
        }
        object.insert("truncated".into(), true.into());
        bytes = serde_json::to_vec(document).ok()?;
    }
    tmt_adapters::private_file::replace(&path, &bytes).ok()?;
    Some(path.display().to_string())
}

/// Every failure points at the report, so the full account is one path away.
fn with_report(message: &str, report: Option<&str>) -> String {
    match report {
        Some(path) => format!("{}. Full report: {path}", message.trim_end_matches('.')),
        None => message.to_owned(),
    }
}

fn identities(
    paths: &ConfigPaths,
    snapshot: &WorkspaceSnapshot,
) -> Result<Vec<StoredIdentity>, Failure> {
    if !paths
        .database
        .try_exists()
        .map_err(|error| Failure::new("WORKSPACE_RESTORE_UNAVAILABLE", error.to_string(), 1))?
    {
        return Ok(Vec::new());
    }
    let ids = snapshot
        .panes
        .iter()
        .filter_map(|pane| pane.identity.as_ref().map(|identity| identity.id.as_str()))
        .collect::<Vec<_>>();
    Storage::workspace_plan_identities(&paths.database, &ids).map_err(|error| {
        Failure::new(
            "WORKSPACE_RESTORE_UNAVAILABLE",
            "Could not read current workspace identities.",
            1,
        )
        .caused_by(error)
    })
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "needs you (floating pane is not restored; tmt resume -- {name})",
        &[")"],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "needs you (stale conversation; tmt resume --retry -- {name})",
        &[")"],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "needs you (directory {dir} is missing; recreate it, then tmt resume -- {name})",
        &[")"],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "No saved workspace records any agents. Nothing was restored. To restore only the layout, pick one from tmt workspace ls and run tmt workspace restore g:<generation> --layout-only.",
        &[" and run ", "."],
        &[(
            "g:<generation>",
            "g:0000000000000000000000000000000000000000000000000000000000000000",
        )],
    ),
    crate::cli_style_tests::HintSpec::core(
        "Restored, but tmt could not record this as the recovery choice: {error}. Check tmt workspace ls before restoring again.",
        &[" before"],
        // The first mention names the program in prose, not a command.
        &[("tmt could", "it could")],
    ),
    crate::cli_style_tests::HintSpec::core(
        "Selected {selection} has {} panes and {} agents; protected {} has {} panes and {} agents. Nothing was restored. To choose the smaller workspace explicitly, run tmt workspace restore {selection} --expect {} --socket {socket_argument}{}.",
        &["."],
        &[
            ("{selection}", "work"),
            (
                "--expect {}",
                "--expect 0000000000000000000000000000000000000000000000000000000000000000",
            ),
            ("{socket_argument}", "/tmp/hint.sock"),
            ("{}", ""),
        ],
    ),
];

struct Revival {
    recorded: String,
    native: Option<String>,
    name: String,
    status: &'static str,
    message: String,
    argv: Option<Vec<String>>,
    failed: bool,
    /// A tmux 3.8 floating pane: the tiled layout is restored around it, so it has
    /// no native pane and nothing starts in it.
    floating: bool,
}

fn prepare(
    snapshot: &WorkspaceSnapshot,
    identities: &[StoredIdentity],
    executable: &str,
) -> Vec<Revival> {
    let plan = restore_plan(snapshot, identities, Some(&[]));
    snapshot
        .panes
        .iter()
        .zip(plan.panes)
        .map(|(saved, planned)| {
            let name = saved
                .identity
                .as_ref()
                .map(|identity| identity.name.clone())
                .or_else(|| {
                    saved
                        .command
                        .as_ref()
                        .and_then(|command| command.argv.get(1).cloned())
                })
                .unwrap_or_else(|| saved.id.clone());
            let mut pane = Revival {
                recorded: saved.id.clone(),
                native: None,
                name,
                status: "skipped",
                message: "skipped (ordinary shell)".into(),
                argv: None,
                failed: false,
                floating: planned.action == WorkspacePaneAction::Floating,
            };
            if pane.floating {
                // Not a failure: scripts read the per-pane status. An agent on it is the
                // user's to start, like any pane restore cannot resume.
                if saved.command.is_some() || saved.identity.is_some() {
                    pane.status = "needs_you";
                    pane.message = match (&saved.command, &saved.identity) {
                        (None, Some(_)) => format!(
                            "needs you (floating pane is not restored; tmt resume -- {name})",
                            name = crate::output::shell_word(&pane.name)
                        ),
                        _ => "needs you (floating pane is not restored; start it yourself)".into(),
                    };
                } else {
                    pane.message = "skipped (floating pane)".into();
                }
                return pane;
            }
            if saved.command.is_none()
                && planned.identity.is_some_and(|identity| {
                    identity.entry.identity.canonical_name
                        != tmt_core::names::normalize_name(&pane.name)
                })
            {
                pane.status = "needs_you";
                pane.message = "needs you (identity name changed; resume it explicitly)".into();
                return pane;
            }
            // A pane that starts an agent keeps refusing a missing directory: the agent would
            // run somewhere its work is not. Only this pane waits; the rest of the workspace
            // restores and the report gives the exact retry.
            if !Path::new(&saved.cwd).is_dir()
                && matches!(
                    planned.action,
                    WorkspacePaneAction::RelaunchCommand | WorkspacePaneAction::Resumable
                )
            {
                let dir = crate::output::shell_word(&saved.cwd);
                pane.status = "needs_you";
                pane.message = match &saved.command {
                    Some(command) => format!(
                        "needs you (directory {dir} is missing; recreate it, then run {})",
                        command
                            .argv
                            .iter()
                            .map(|word| crate::output::shell_word(word))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                    None => format!(
                        "needs you (directory {dir} is missing; recreate it, then tmt resume -- {name})",
                        name = crate::output::shell_word(&pane.name)
                    ),
                };
                return pane;
            }
            match planned.action {
                WorkspacePaneAction::RelaunchCommand => {
                    let mut argv = saved.command.as_ref().expect("command plan").argv.clone();
                    // Capture admits only literal tmt external commands. Use this
                    // invocation's executable rather than a possibly different PATH tmt.
                    argv[0] = executable.to_owned();
                    pane.argv = Some(argv);
                    pane.status = "relaunched";
                    pane.message = "relaunch started".into();
                }
                WorkspacePaneAction::Resumable => {
                    let saved_identity = saved.identity.as_ref().expect("saved annotation");
                    pane.argv = Some(vec![
                        executable.to_owned(),
                        "resume".into(),
                        "--".into(),
                        saved_identity.name.clone(),
                    ]);
                    pane.status = "started";
                    pane.message = "resume started".into();
                }
                WorkspacePaneAction::IdentityMissing => {
                    pane.status = "needs_you";
                    pane.message = "needs you (original identity is missing)".into();
                }
                WorkspacePaneAction::NoRememberedSession => {
                    pane.status = "needs_you";
                    pane.message = "needs you (no remembered conversation)".into();
                }
                WorkspacePaneAction::StaleRequiresRetry => {
                    pane.status = "needs_you";
                    pane.message = format!(
                        "needs you (stale conversation; tmt resume --retry -- {name})",
                        name = crate::output::shell_word(&pane.name)
                    );
                }
                WorkspacePaneAction::Shell => {}
                WorkspacePaneAction::Floating => unreachable!("returned above"),
            }
            pane
        })
        .collect()
}

fn revive(
    paths: &ConfigPaths,
    snapshot: &WorkspaceSnapshot,
    host: &Host,
    layout: &workspace::restore::LayoutRestore,
    prepared: Vec<Revival>,
) -> Vec<Revival> {
    let mut attempted = std::collections::HashSet::new();
    prepared
        .into_iter()
        .map(|mut pane| {
            if pane.floating {
                return pane;
            }
            let mapped = layout
                .panes
                .iter()
                .find(|mapped| mapped.recorded == pane.recorded);
            pane.native = mapped.map(|mapped| mapped.native.clone());
            if layout.partial() || mapped.is_none() {
                pane.status = "skipped";
                pane.message = if layout.partial() {
                    "skipped (layout incomplete)"
                } else {
                    "skipped (existing session)"
                }
                .into();
                return pane;
            }
            let Some(argv) = pane.argv.take() else {
                return pane;
            };
            let saved = snapshot
                .panes
                .iter()
                .find(|saved| saved.id == pane.recorded)
                .expect("prepared pane");
            // Recheck the name-to-UUID association immediately before ordinary resume.
            // Public resume keeps its existing name-only surface and launch policy.
            if saved.command.is_none() {
                let id = &saved.identity.as_ref().expect("identity launch").id;
                if attempted.contains(id) {
                    pane.status = "skipped";
                    pane.message = "skipped (identity already selected)".into();
                    return pane;
                }
                match identities(paths, snapshot) {
                    Ok(records)
                        if saved.identity.as_ref().is_some_and(|expected| {
                            records.iter().any(|record| {
                                record.entry.identity.id == expected.id
                                    && record.entry.identity.canonical_name
                                        == tmt_core::names::normalize_name(&expected.name)
                            })
                        }) =>
                    {
                        attempted.insert(id.clone());
                    }
                    _ => {
                        pane.status = "needs_you";
                        pane.message =
                            "needs you (identity changed or could not be checked)".into();
                        return pane;
                    }
                }
            }
            // Select the same Core data directory even when an existing host
            // server inherited a different TMT_HOME. env execs the ordinary CLI.
            let argv = [
                "/usr/bin/env".to_owned(),
                format!("TMT_HOME={}", paths.global_dir.display()),
            ]
            .into_iter()
            .chain(argv)
            .collect::<Vec<_>>();
            match host.workspace_start_pane(
                snapshot,
                layout,
                mapped.expect("created pane"),
                &argv,
                Instant::now() + Duration::from_secs(3),
            ) {
                Ok(true) => {}
                Ok(false) => {
                    pane.status = "needs_you";
                    pane.message = "needs you (host cannot start this pane)".into();
                    pane.failed = true;
                }
                Err(error) => {
                    pane.status = "needs_you";
                    pane.message = format!("needs you (startup refused or uncertain: {error})");
                    pane.failed = true;
                }
            }
            pane
        })
        .collect()
}

pub fn execute(
    socket: Option<&str>,
    name: Option<&tmt_core::workspace::SnapshotSelector>,
    expect: Option<&str>,
    layout_only: bool,
    mode: OutputMode,
) -> io::Result<u8> {
    let context = crate::workspace_snapshot_command::context(socket);
    let report_paths = context.as_ref().ok().map(|(paths, _, _)| paths.clone());
    // A refusal before the host is touched already says what to do; only a restore that reached
    // the server writes a failure report.
    let reached_host = std::cell::Cell::new(false);
    let outcome: Result<_, Failure> = (|| {
        let (paths, socket, host) = context?;
        let document =
            workspace::saved::acquire_selected(&paths, &socket, name, expect).map_err(|error| {
                crate::workspace_snapshot_command::failure(
                    error,
                    "WORKSPACE_RESTORE_UNAVAILABLE",
                    name.is_none(),
                )
            })?;
        let generation = document
            .generation
            .as_ref()
            .map(|generation| generation.id.as_str().to_owned());
        if name.is_none()
            && !layout_only
            && document
                .generation
                .as_ref()
                .is_some_and(|generation| generation.agents == 0)
        {
            return Err(Failure::new(
                "WORKSPACE_NO_AGENT_GENERATION",
                "No saved workspace records any agents. Nothing was restored. To restore only the layout, pick one from tmt workspace ls and run tmt workspace restore g:<generation> --layout-only.",
                1,
            ));
        }
        if document.requires_confirmation {
            let selection = name
                .map(|name| name.as_str())
                .or(generation.as_deref())
                .expect("selected recovery");
            let socket_argument = format!("'{}'", socket.replace('\'', "'\\''"));
            let baseline = document
                .baseline
                .as_ref()
                .expect("smaller than protected generation");
            let retry = format!(
                "Selected {selection} has {} panes and {} agents; protected {} has {} panes and {} agents. Nothing was restored. To choose the smaller workspace explicitly, run tmt workspace restore {selection} --expect {} --socket {socket_argument}{}.",
                document.snapshot.panes.len(),
                document.snapshot.agent_count(),
                baseline.id.as_str(),
                baseline.panes,
                baseline.agents,
                document.digest,
                if layout_only { " --layout-only" } else { "" }
            );
            let question = format!(
                "Restore this smaller workspace ({} panes, {} agents)",
                document.snapshot.panes.len(),
                document.snapshot.agent_count()
            );
            let mut prompt = tmt_cli_style::stream::stderr();
            if !crate::consent::ask(
                &mut prompt,
                false,
                mode,
                crate::consent::Consent {
                    code: "WORKSPACE_SMALLER_CONSENT_REQUIRED",
                    refusal: &retry,
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
        let accepted_generation = document
            .generation
            .as_ref()
            .filter(|_| name.is_some())
            .map(|generation| generation.id.clone());
        let snapshot = document.snapshot;
        let identities = if layout_only {
            Vec::new()
        } else {
            identities(&paths, &snapshot)?
        };
        let executable = tmt_adapters::core_executable::selected()
            .map_err(|error| Failure::new("WORKSPACE_RESTORE_UNAVAILABLE", error.to_string(), 1))?;
        let prepared = if layout_only {
            Vec::new()
        } else {
            prepare(&snapshot, &identities, &executable.to_string_lossy())
        };
        let targets = prepared
            .iter()
            .filter(|pane| pane.argv.is_some())
            .map(|pane| pane.recorded.clone())
            .collect::<Vec<_>>();
        reached_host.set(true);
        let mut result = host
            .workspace_restore_layout(&snapshot, layout_deadline(snapshot.panes.len()), &targets)
            .map_err(|error| {
                Failure::new(
                    "WORKSPACE_RESTORE_UNAVAILABLE",
                    format!("Could not restore workspace layout: {error}"),
                    1,
                )
            })?
            .ok_or_else(|| {
                Failure::new(
                    "WORKSPACE_RESTORE_UNAVAILABLE",
                    "The selected host does not support layout restoration.",
                    1,
                )
            })?;
        // A layout-only restore reports only what it left out: the floating panes.
        let revival = if layout_only {
            prepare(&snapshot, &[], &executable.to_string_lossy())
                .into_iter()
                .filter(|pane| pane.floating)
                .collect()
        } else {
            revive(&paths, &snapshot, &host, &result, prepared)
        };
        if !result.partial()
            && !revival.iter().any(|pane| pane.failed)
            && let Some(selected) = accepted_generation
            && let Err(error) = workspace::saved::accept_generation(
                &paths,
                &socket,
                &selected,
                document.baseline.as_ref().map(|generation| &generation.id),
            )
        {
            result.failures.push(format!("Restored, but tmt could not record this as the recovery choice: {error}. Check tmt workspace ls before restoring again."));
        }
        Ok((socket, document.digest, generation, result, revival))
    })();
    let (socket, digest, generation, result, revival) = match outcome {
        Ok(result) => result,
        Err(mut error) => {
            let mut failed = serde_json::json!({"version": 1, "status": "failed",
                "error": {"code": error.code, "message": error.message}});
            let report = report_paths
                .as_ref()
                .filter(|_| reached_host.get())
                .and_then(|paths| write_report(paths, &mut failed));
            error.message = with_report(&error.message, report.as_deref());
            return error.publish(mode);
        }
    };
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    let partial = result.partial() || revival.iter().any(|pane| pane.failed);
    let mut document = serde_json::json!({"version": 1, "socket": socket, "name": name.map(|name| name.as_str()), "generation": generation, "digest": digest, "layoutOnly": layout_only,
        "status": if partial { "partial" } else { "completed" }, "sessions": result.sessions,
        "windows": result.windows, "panes": result.panes, "fallbacks": result.fallbacks,
        "failures": result.failures,
        "revival": revival.iter().map(|pane| serde_json::json!({
            "recorded": pane.recorded, "native": pane.native, "name": pane.name,
            "status": pane.status, "message": pane.message})).collect::<Vec<_>>()});
    let report = report_paths
        .as_ref()
        .and_then(|paths| write_report(paths, &mut document));
    if mode.json {
        document["report"] = report.into();
        if layout_only && revival.is_empty() {
            document.as_object_mut().expect("object").remove("revival");
        }
        writeln!(output, "{document}")?;
    } else {
        let count = |action: &str, beside: bool| {
            result
                .sessions
                .iter()
                .filter(|session| {
                    session.action == action && session.restored_as.is_some() == beside
                })
                .count()
        };
        let mut line = format!(
            "Workspace on {socket}: {} sessions created",
            count("created", false)
        );
        if count("created", true) > 0 {
            line.push_str(&format!(
                ", {} restored beside a name in use",
                count("created", true)
            ));
        }
        if count("merged", true) > 0 {
            line.push_str(&format!(
                ", {} added to the fresh default session",
                count("merged", true)
            ));
        }
        line.push_str(&format!(
            ", {} already restored",
            count("skip_existing", false)
        ));
        if partial {
            line.push_str(", partial restore");
        }
        writeln!(output, "{line}.")?;
        // A name that was not simply recreated is always said, never left to be inferred.
        for session in &result.sessions {
            match (session.action.as_str(), session.restored_as.as_deref()) {
                ("merged", Some(live)) => writeln!(
                    output,
                    "  {:?}: its windows were added to your session {live:?}, after your window",
                    session.name
                )?,
                ("created", Some(alias)) => writeln!(
                    output,
                    "  {:?}: the name is in use, so it was restored as {alias:?}",
                    session.name
                )?,
                _ => (),
            }
        }
        for fallback in &result.fallbacks {
            writeln!(
                output,
                "  Pane {} opened in {}: its recorded directory {} is missing",
                fallback.pane, fallback.used, fallback.wanted
            )?;
        }
        for session in result.sessions.iter().filter(|_| layout_only) {
            if session.restored_as.is_none() {
                writeln!(output, "  {:?}: {}", session.name, session.action)?;
            }
            if let Some(pane) = &session.retained_bootstrap {
                writeln!(output, "    Bootstrap retained: {pane}")?;
            }
        }
        for pane in &revival {
            writeln!(output, "  {:?}: {}", pane.name, pane.message)?;
        }
        for failure in &result.failures {
            writeln!(output, "  {failure}")?;
        }
        if let Some(path) = report.as_deref().filter(|_| partial) {
            writeln!(output, "  Full report: {path}")?;
        }
    }
    Ok(u8::from(partial))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_adapters::host::CallerEnvironment;
    use tmt_core::{
        binding::{
            BindingEntry,
            session::{
                HarnessId, ProviderSessionId, RememberedSession, RuntimeMode, SessionPreferences,
            },
        },
        endpoint::ProcessIncarnation,
        identity::{Identity, Lifetime},
        workspace::*,
    };

    fn input() -> (WorkspaceSnapshot, Vec<StoredIdentity>) {
        let snapshot = WorkspaceSnapshot {
            captured_at_ms: 1,
            server: WorkspaceServer {
                socket: "/selected".into(),
                process: ProcessIncarnation::new(1, "old").unwrap(),
                id: None,
            },
            sessions: Vec::new(),
            windows: Vec::new(),
            panes: vec![WorkspacePane {
                id: "%1".into(),
                window: "@1".into(),
                index: 0,
                left: 0,
                top: 0,
                width: 80,
                height: 24,
                cwd: "/".into(),
                command: None,
                identity: Some(WorkspaceIdentity {
                    id: "original".into(),
                    name: "Original".into(),
                    lifetime: "saved".into(),
                    binding: "old".into(),
                    harness: None,
                    session: Some("obsolete".into()),
                    mode: None,
                    channel: None,
                }),
            }],
        };
        let records = vec![StoredIdentity {
            entry: BindingEntry {
                identity: Identity {
                    id: "original".into(),
                    name: "Original".into(),
                    canonical_name: "original".into(),
                    lifetime: Lifetime::Saved,
                    created_at: "date".into(),
                    updated_at: "date".into(),
                },
                binding: None,
            },
            preferences: SessionPreferences {
                remembered: Some(RememberedSession {
                    harness: HarnessId::new("claude").unwrap(),
                    provider_session: ProviderSessionId::new("current").unwrap(),
                    mode: RuntimeMode::new("default").unwrap(),
                    state: None,
                    stale_at_ms: None,
                    resume_pending_at_ms: Some(2),
                }),
                ..Default::default()
            },
        }];
        (snapshot, records)
    }

    #[test]
    fn revival_uses_ordinary_resume_without_replaying_snapshot_coordinates_or_retry() {
        let (saved, records) = input();
        let pane = prepare(&saved, &records, "/selected/tmt").pop().unwrap();
        assert_eq!(
            pane.argv.unwrap(),
            ["/selected/tmt", "resume", "--", "Original"]
        );
        assert_eq!(pane.message, "resume started");
        let mut layout = workspace::restore::LayoutRestore::default();
        layout.failures.push("layout stopped".into());
        // An incomplete layout cannot reach storage or host startup, even with
        // eligible commands. Empty mappings likewise mean existing sessions.
        let paths = ConfigPaths::resolve(
            std::path::Path::new("/unused/cwd"),
            std::path::Path::new("/unused/home"),
            None,
            None,
        );
        let host = Host::for_caller(&CallerEnvironment {
            tmux: None,
            pane: None,
            process_id: 10,
            driver_env: Default::default(),
        });
        let skipped = revive(
            &paths,
            &saved,
            &host,
            &layout,
            prepare(&saved, &records, "/selected/tmt"),
        );
        assert_eq!(skipped[0].message, "skipped (layout incomplete)");
        layout.failures.clear();
        let skipped = revive(
            &paths,
            &saved,
            &host,
            &layout,
            prepare(&saved, &records, "/selected/tmt"),
        );
        assert_eq!(skipped[0].message, "skipped (existing session)");
    }

    // A tiled shell %2 and %1 floating above it, in the producer's shape (#4405).
    const FLOATING_ABOVE_SHELL: &str = r#"{"V":2,"L":{"t":"v","w":80,"h":24,"x":0,"y":0,"c":[{"t":"p","w":80,"h":24,"x":0,"y":0,"a":true,"i":0,"I":"%2"},{"t":"p","w":20,"h":5,"x":2,"y":2,"i":1,"z":0,"I":"%1"}]}}"#;

    fn floating_input() -> (WorkspaceSnapshot, Vec<StoredIdentity>) {
        let (mut saved, records) = input();
        saved.windows.push(WorkspaceWindow {
            id: "@1".into(),
            name: "work".into(),
            layout: FLOATING_ABOVE_SHELL.into(),
            visible_layout: FLOATING_ABOVE_SHELL.into(),
            width: 80,
            height: 24,
            active_pane: "%2".into(),
        });
        saved.panes.push(WorkspacePane {
            id: "%2".into(),
            index: 0,
            identity: None,
            ..saved.panes[0].clone()
        });
        saved.panes[0].index = 1;
        (saved, records)
    }

    #[test]
    fn a_floating_pane_is_reported_and_never_started_or_counted_as_a_failure() {
        let (saved, records) = floating_input();
        let panes = prepare(&saved, &records, "/tmt");
        let float = &panes[0];
        // An agent on it is the user's to resume; nothing is launched in a pane never made.
        assert!(float.floating && !float.failed && float.argv.is_none());
        assert_eq!(float.status, "needs_you");
        assert_eq!(
            float.message,
            "needs you (floating pane is not restored; tmt resume -- Original)"
        );
        assert_eq!(panes[1].message, "skipped (ordinary shell)");
        assert!(!panes[1].floating);

        let mut annotated = saved.clone();
        annotated.panes[0].identity = None;
        let plain = prepare(&annotated, &[], "/tmt");
        assert_eq!(
            (plain[0].status, plain[0].message.as_str()),
            ("skipped", "skipped (floating pane)")
        );
        annotated.panes[0].command = Some(ExternalCommand {
            argv: vec!["tmt".into(), "example".into(), "ui".into()],
            owner: ProcessIncarnation::new(1, "old").unwrap(),
        });
        let command = prepare(&annotated, &[], "/tmt");
        assert_eq!(command[0].status, "needs_you");
        assert!(command[0].message.ends_with("start it yourself)"));
        assert!(command[0].argv.is_none());

        // revive leaves it as reported, whatever the layout did: it is not an existing session.
        let paths = ConfigPaths::resolve(
            std::path::Path::new("/unused/cwd"),
            std::path::Path::new("/unused/home"),
            None,
            None,
        );
        let host = Host::for_caller(&CallerEnvironment {
            tmux: None,
            pane: None,
            process_id: 10,
            driver_env: Default::default(),
        });
        let layout = workspace::restore::LayoutRestore::default();
        let revived = revive(
            &paths,
            &saved,
            &host,
            &layout,
            prepare(&saved, &records, "/tmt"),
        );
        assert_eq!(revived[0].status, "needs_you");
        assert_eq!(revived[1].message, "skipped (existing session)");
    }

    #[test]
    fn stale_missing_renamed_and_replaced_identities_never_get_a_launch_command() {
        let (saved, mut records) = input();
        records[0]
            .preferences
            .remembered
            .as_mut()
            .unwrap()
            .stale_at_ms = Some(1);
        let pane = prepare(&saved, &records, "/tmt").pop().unwrap();
        assert!(pane.argv.is_none());
        assert!(pane.message.contains("tmt resume --retry -- Original"));
        records[0].entry.identity.canonical_name = "renamed".into();
        let renamed = prepare(&saved, &records, "/tmt").pop().unwrap();
        assert!(renamed.argv.is_none());
        assert!(renamed.message.contains("identity name changed"));
        assert!(!renamed.message.contains("--retry"));
        records[0]
            .preferences
            .remembered
            .as_mut()
            .unwrap()
            .stale_at_ms = None;
        records[0].entry.identity.canonical_name = "renamed".into();
        assert!(prepare(&saved, &records, "/tmt")[0].argv.is_none());
        records[0].entry.identity.canonical_name = "original".into();
        records[0].entry.identity.id = "replacement".into();
        assert!(prepare(&saved, &records, "/tmt")[0].argv.is_none());
        assert!(prepare(&saved, &[], "/tmt")[0].argv.is_none());
        records[0].entry.identity.id = "original".into();
        records[0].preferences.remembered = None;
        assert!(prepare(&saved, &records, "/tmt")[0].argv.is_none());
    }

    #[test]
    fn recorded_board_arguments_remain_literal_and_use_this_executable() {
        let (mut saved, records) = input();
        saved.panes[0].command = Some(ExternalCommand {
            argv: vec![
                "tmt".into(),
                "example".into(),
                "ui".into(),
                "--tabs=agents,requests".into(),
                "literal '$()#{pid};\n".into(),
            ],
            owner: ProcessIncarnation::new(1, "old").unwrap(),
        });
        assert_eq!(
            prepare(&saved, &records, "/selected/tmt")[0]
                .argv
                .as_ref()
                .unwrap(),
            &[
                "/selected/tmt",
                "example",
                "ui",
                "--tabs=agents,requests",
                "literal '$()#{pid};\n"
            ]
        );
    }

    #[test]
    fn an_agent_pane_with_a_missing_directory_waits_alone_with_the_exact_retry() {
        let (mut saved, records) = input();
        saved.panes[0].cwd = "/no/such/directory".into();
        let resumed = &prepare(&saved, &records, "/selected/tmt")[0];
        assert!(
            resumed.argv.is_none(),
            "no agent starts in a missing directory"
        );
        assert_eq!(resumed.status, "needs_you");
        assert_eq!(
            resumed.message,
            "needs you (directory /no/such/directory is missing; recreate it, then tmt resume -- Original)"
                .replace("/no/such/directory", "'/no/such/directory'")
                .replace("''", "'")
        );
        saved.panes[0].command = Some(ExternalCommand {
            argv: vec!["tmt".into(), "example".into(), "two words".into()],
            owner: ProcessIncarnation::new(1, "old").unwrap(),
        });
        let command = &prepare(&saved, &records, "/selected/tmt")[0];
        assert!(command.argv.is_none());
        assert!(
            command
                .message
                .ends_with("recreate it, then run tmt example 'two words')"),
            "{}",
            command.message
        );
        // An ordinary shell pane in a missing directory is not an agent: it restores in a fallback.
        saved.panes[0].command = None;
        saved.panes[0].identity = None;
        let shell = &prepare(&saved, &records, "/selected/tmt")[0];
        assert_eq!(shell.status, "skipped");
    }

    #[test]
    fn a_large_workspace_gets_more_time_but_never_more_than_two_minutes() {
        let seconds = |panes| {
            layout_deadline(panes)
                .duration_since(Instant::now())
                .as_secs()
        };
        assert!((19..=20).contains(&seconds(0)));
        assert!(seconds(65) >= 52, "20 s plus half a second a pane");
        assert!(seconds(10_000) <= 120);
    }

    #[test]
    fn the_report_is_private_replaced_each_run_and_named_by_every_failure() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("tmt-restore-report-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let paths = ConfigPaths {
            global_dir: root.join("tmt"),
            global_config: root.join("config"),
            local_config: root.join("local"),
            database: root.join("db"),
        };
        let first = write_report(&paths, &mut serde_json::json!({"status": "partial"})).unwrap();
        assert!(first.ends_with("/restore-last.json"));
        assert_eq!(
            std::fs::metadata(&first).unwrap().permissions().mode() & 0o777,
            0o600
        );
        write_report(&paths, &mut serde_json::json!({"status": "completed"})).unwrap();
        assert_eq!(
            std::fs::read_to_string(&first).unwrap(),
            r#"{"status":"completed"}"#
        );
        // Too large: the per-pane lists go, the verdict stays.
        let mut large =
            serde_json::json!({"status": "partial", "panes": ["x".repeat(MAX_REPORT_BYTES)]});
        write_report(&paths, &mut large).unwrap();
        let kept: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&first).unwrap()).unwrap();
        assert_eq!(
            kept,
            serde_json::json!({"status": "partial", "truncated": true})
        );
        assert_eq!(
            with_report(
                "Could not restore workspace layout: no.",
                Some("/x/restore-last.json")
            ),
            "Could not restore workspace layout: no. Full report: /x/restore-last.json"
        );
        assert_eq!(with_report("no", None), "no");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
