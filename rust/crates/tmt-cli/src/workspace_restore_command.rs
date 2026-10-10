//! Two-phase recovery reuses ordinary resume and generic external dispatch.

use crate::{invocation::OutputMode, output::Failure, workspace_show_command::selected_socket};
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    host::{CallerEnvironment, Host},
    storage::Storage,
    workspace,
};
use tmt_core::workspace::{
    StoredIdentity, WorkspaceSnapshot,
    plan::{WorkspacePaneAction, restore_plan},
};

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
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] =
    &[crate::cli_style_tests::HintSpec::core(
        "needs you (stale conversation; tmt resume --retry -- {name})",
        &[")"],
        &[],
    )];

struct Revival {
    recorded: String,
    native: Option<String>,
    name: String,
    status: &'static str,
    message: String,
    argv: Option<Vec<String>>,
    failed: bool,
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
            };
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

pub fn execute(socket: Option<&str>, layout_only: bool, mode: OutputMode) -> io::Result<u8> {
    let outcome: Result<_, Failure> = (|| {
        let caller = CallerEnvironment::current();
        let socket = selected_socket(socket, &caller)?;
        let paths = ConfigPaths::discover().map_err(Failure::from)?;
        let snapshot = workspace::read_snapshot(&paths, &socket).map_err(|error| {
            Failure::new(
                "WORKSPACE_RESTORE_UNAVAILABLE",
                format!("Could not read workspace recovery input: {error}"),
                1,
            )
        })?;
        let identities = if layout_only {
            Vec::new()
        } else {
            identities(&paths, &snapshot)?
        };
        let executable = std::env::current_exe()
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
        let host = Host::for_caller(&caller);
        let result = host
            .workspace_restore_layout(
                &snapshot,
                Instant::now() + Duration::from_secs(30),
                &targets,
            )
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
        let revival = if layout_only {
            Vec::new()
        } else {
            revive(&paths, &snapshot, &host, &result, prepared)
        };
        Ok((socket, result, revival))
    })();
    let (socket, result, revival) = match outcome {
        Ok(result) => result,
        Err(error) => return error.publish(mode),
    };
    let mut output = tmt_cli_style::stream::stdout(mode.json);
    let partial = result.partial() || revival.iter().any(|pane| pane.failed);
    if mode.json {
        let mut document = serde_json::json!({"version": 1, "socket": socket, "layoutOnly": layout_only,
            "status": if partial { "partial" } else { "completed" }, "sessions": result.sessions,
            "windows": result.windows, "panes": result.panes, "failures": result.failures});
        if !layout_only {
            document["revival"] = serde_json::json!(
                revival
                    .iter()
                    .map(|pane| serde_json::json!({
                "recorded": pane.recorded, "native": pane.native, "name": pane.name,
                "status": pane.status, "message": pane.message}))
                    .collect::<Vec<_>>()
            );
        }
        writeln!(output, "{document}")?;
    } else {
        let created = result
            .sessions
            .iter()
            .filter(|session| session.action == "created")
            .count();
        let skipped = result
            .sessions
            .iter()
            .filter(|session| session.action == "skip_existing")
            .count();
        writeln!(
            output,
            "Workspace on {socket}: {created} sessions created, {skipped} skipped{}.",
            if partial { ", partial restore" } else { "" }
        )?;
        for session in result.sessions.iter().filter(|_| layout_only) {
            writeln!(output, "  {:?}: {}", session.name, session.action)?;
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
    }
    Ok(u8::from(partial))
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
                cwd: "/cwd".into(),
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
}
