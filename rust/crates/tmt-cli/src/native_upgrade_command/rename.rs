//! One-release post-upgrade option cutover; no ordinary dispatch fallback.

use std::{
    path::Path,
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    host::{CallerEnvironment, Host},
    process::{CommandRunner, UnixCommandRunner},
    storage::{Storage, StorageError},
};
use tmt_core::{binding::BindingRepository, host::HostKind};

const STORAGE_BUDGET: Duration = Duration::from_secs(3);
const PANE_BUDGET: Duration = Duration::from_secs(3);
const OVERALL_BUDGET: Duration = Duration::from_secs(180);

pub(super) fn hints() -> Vec<String> {
    match ConfigPaths::discover() {
        Ok(paths) => migrate(
            &paths.database,
            &CallerEnvironment::current(),
            &UnixCommandRunner,
        ),
        Err(error) => vec![format!(
            "Pane option cutover unavailable: {error}; rebind each affected pane with tmt this <name>"
        )],
    }
}

fn migrate<R: CommandRunner>(
    database: &Path,
    caller: &CallerEnvironment,
    runner: &R,
) -> Vec<String> {
    migrate_with_now(database, caller, runner, Instant::now)
}

fn migrate_with_now<R: CommandRunner>(
    database: &Path,
    caller: &CallerEnvironment,
    runner: &R,
    now: impl Fn() -> Instant,
) -> Vec<String> {
    if !database.exists() {
        return Vec::new();
    }
    let result = (|| -> Result<Vec<String>, StorageError> {
        // Existing schema only; never create or migrate a database for cosmetics.
        let mut storage = Storage::open_hook(database, now() + STORAGE_BUDGET)?;
        let entries = storage.with_binding_transaction(|records| records.binding_entries())?;
        let entries: Vec<_> = entries
            .into_iter()
            .filter(|entry| {
                entry
                    .binding
                    .as_ref()
                    .is_some_and(|binding| binding.server.host == HostKind::Tmux)
            })
            .collect();
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        let socket = match caller.selected_server_socket() {
            Ok(Some(socket)) => socket,
            _ => return Ok(vec!["No verified selected tmux server for option cutover; rebind affected panes with tmt this <name>".into()]),
        };
        let mut hints = Vec::new();
        // Forty fully exhausted panes still fit without consuming a later
        // pane's budget. The overall bound limits larger/pathological stores.
        let overall_deadline = now() + OVERALL_BUDGET;
        let mut converted = 0;
        let mut unchanged = 0;
        for entry in entries.into_iter().filter(|entry| {
            entry
                .binding
                .as_ref()
                .is_some_and(|binding| binding.server.socket_path == socket)
        }) {
            let started = now();
            let deadline = (started + PANE_BUDGET).min(overall_deadline);
            let (migrated, needs_hint) = if started >= overall_deadline {
                (false, true)
            } else {
                let host = Host::for_server_with(
                    &entry.binding.as_ref().expect("bound entry").server,
                    runner,
                );
                match host.prepare_option_rename(&entry, deadline) {
                    // No retired marker needs no change and no rebind hint.
                    Ok(None) => (false, false),
                    Ok(Some(proof)) => {
                        let migrated = storage
                            .with_binding_transaction(|records| {
                                // Changed rows cannot authorize effects from the old proof.
                                if records.entry_by_id(&entry.identity.id)?.as_ref() != Some(&entry)
                                {
                                    return Ok::<_, StorageError>(false);
                                }
                                Ok(host.commit_option_rename(&proof, deadline).unwrap_or(false))
                            })
                            .unwrap_or(false);
                        (migrated, !migrated)
                    }
                    Err(_) => (false, true),
                }
            };
            if migrated {
                converted += 1;
            } else {
                unchanged += 1;
            }
            if needs_hint {
                hints.push(format!(
                    "Pane option cutover left {} unchanged; run tmt this {} in that pane",
                    entry.binding.as_ref().expect("bound entry").pane_id,
                    crate::output::shell_word(&entry.identity.name)
                ));
            }
        }
        if converted + unchanged > 0 {
            hints.push(format!(
                "Pane option cutover: {converted} converted, {unchanged} left unchanged"
            ));
        }
        Ok(hints)
    })();
    result.unwrap_or_else(|error| {
        vec![format!(
            "Pane option cutover unavailable: {error}; rebind affected panes with tmt this <name>"
        )]
    })
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "Pane option cutover unavailable: {error}; rebind each affected pane with tmt this <name>",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "Pane option cutover unavailable: {error}; rebind affected panes with tmt this <name>",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "No verified selected tmux server for option cutover; rebind affected panes with tmt this <name>",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "Pane option cutover left {} unchanged; run tmt this {} in that pane",
        &[" in that pane"],
        &[],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_adapters::process::{CommandError, CommandOutput, CommandRequest};
    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "tmt-option-cutover-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    struct NoHost;
    impl CommandRunner for NoHost {
        fn execute(&self, _: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            panic!("missing or unbound storage must not probe tmux");
        }
    }
    #[test]
    fn missing_database_stays_missing_without_host_work() {
        let root = Directory::new();
        let path = root.0.join("missing.db");
        assert!(migrate(&path, &CallerEnvironment::current(), &NoHost).is_empty());
        assert!(!path.exists());
    }
    #[test]
    fn empty_database_has_no_host_work_or_hint() {
        let root = Directory::new();
        let path = root.0.join("state.db");
        Storage::open(&path).unwrap().close().unwrap();
        assert!(migrate(&path, &CallerEnvironment::current(), &NoHost).is_empty());
    }
    fn forty_bindings(database: &Path) -> Vec<tmt_core::binding::BindingEntry> {
        use tmt_core::{
            binding::BindingEntry,
            endpoint::{PaneObservation, ServerEvidence},
            identity::{Lifetime, create_or_resolve},
        };
        let mut storage = Storage::open(database).unwrap();
        let mut entries = Vec::new();
        for index in 0..40 {
            let identity =
                create_or_resolve(&mut storage, &format!("Pane{index:02}"), Lifetime::Saved)
                    .unwrap()
                    .identity;
            let server = ServerEvidence {
                host: HostKind::Tmux,
                server_id: "00000000-0000-4000-8000-000000000001".into(),
                socket_path: "/tmp/selected.sock".into(),
                server_pid: 321,
                server_start_time: "1700000000".into(),
            };
            let pane = PaneObservation {
                id: format!("%{index}"),
                target: None,
                cwd: None,
                command: "sh".into(),
                pane_pid: 900 + index,
                pane_incarnation: Some("captured".into()),
                suggested_name: None,
                marker: None,
            };
            let binding = storage
                .with_binding_transaction(|records| {
                    records.insert_binding(&identity, &server, &pane)
                })
                .unwrap();
            entries.push(BindingEntry {
                identity,
                binding: Some(binding),
            });
        }
        storage.close().unwrap();
        entries
    }

    struct ManyPanes {
        entries: std::collections::BTreeMap<String, tmt_core::binding::BindingEntry>,
        timeouts: std::collections::BTreeSet<String>,
        exhausted: std::cell::Cell<Option<Instant>>,
        deadlines: std::cell::RefCell<std::collections::BTreeMap<String, Instant>>,
        converted: std::cell::RefCell<std::collections::BTreeSet<String>>,
        overall_clock: Option<std::rc::Rc<std::cell::Cell<Instant>>>,
        retired: bool,
        current_pane: std::cell::RefCell<Option<String>>,
        seen: std::cell::RefCell<std::collections::BTreeSet<String>>,
    }
    impl ManyPanes {
        fn new(entries: &[tmt_core::binding::BindingEntry]) -> Self {
            Self {
                entries: entries
                    .iter()
                    .map(|entry| {
                        (
                            entry.binding.as_ref().unwrap().pane_id.clone(),
                            entry.clone(),
                        )
                    })
                    .collect(),
                timeouts: Default::default(),
                exhausted: Default::default(),
                deadlines: Default::default(),
                converted: Default::default(),
                overall_clock: None,
                retired: true,
                current_pane: Default::default(),
                seen: Default::default(),
            }
        }
    }
    impl ManyPanes {
        fn observe_allocation(&self, deadline: Instant) {
            let current = self.current_pane.borrow();
            let pane = current.as_ref().unwrap();
            let previous = self.deadlines.borrow_mut().insert(pane.clone(), deadline);
            if let Some(previous) = previous {
                assert_eq!(deadline, previous, "one pane shares one bounded allocation");
            } else if let Some(exhausted) = self.exhausted.get() {
                assert!(
                    deadline > exhausted,
                    "later pane reused an exhausted allocation"
                );
            }
        }
    }
    impl CommandRunner for ManyPanes {
        fn process_observation(
            &self,
            pid: u64,
            deadline: Instant,
        ) -> Option<tmt_adapters::process::runtime::ProcessObservation> {
            if self
                .timeouts
                .contains(self.current_pane.borrow().as_ref().unwrap())
            {
                return None;
            }
            self.observe_allocation(deadline);
            Some(tmt_adapters::process::runtime::ProcessObservation::Live(
                tmt_core::endpoint::ProcessIncarnation::new(pid, "captured").unwrap(),
            ))
        }
        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            if request.program == std::ffi::OsStr::new("/usr/bin/env")
                && request
                    .args
                    .get(2)
                    .is_some_and(|path| path == "/bin/ps" || path == "/usr/bin/ps")
            {
                // The native proof receives the whole pane allocation; tmux
                // subprocesses additionally have their own shorter bound.
                // Inject a timed-out proof without sleeping or spawning a child.
                self.observe_allocation(request.deadline);
                self.exhausted.set(Some(request.deadline));
                return UnixCommandRunner.execute(CommandRequest {
                    deadline: Instant::now(),
                    ..request
                });
            }
            let args: Vec<_> = request
                .args
                .iter()
                .map(|arg| arg.to_str().unwrap())
                .collect();
            let pane = args[args.iter().position(|arg| *arg == "-t").unwrap() + 1];
            self.current_pane.replace(Some(pane.into()));
            self.seen.borrow_mut().insert(pane.into());
            let entry = &self.entries[pane];
            let binding = entry.binding.as_ref().unwrap();
            if let Some(allocation) = self.deadlines.borrow().get(pane) {
                assert!(request.deadline <= *allocation);
            }
            if let Some(clock) = &self.overall_clock {
                clock.set(clock.get() + OVERALL_BUDGET);
            }
            let raw = serde_json::json!({"version":1,"globalIdentity":{
                "name":entry.identity.name,"canonicalName":entry.identity.canonical_name,
                "identityId":entry.identity.id,"bindingId":binding.id,
                "serverId":binding.server.server_id,"panePid":binding.pane_pid
            }})
            .to_string();
            let text = if args.contains(&"display-message") {
                [
                    binding.server.server_id.as_str(),
                    "/tmp/selected.sock",
                    "321",
                    "1700000000",
                    pane,
                    "e2e:0.0",
                    "/workspace",
                    "sh",
                    &binding.pane_pid.to_string(),
                    "1",
                    &raw,
                ]
                .join("__TMT_FIELD_4f1c__")
                    + "\n"
            } else if args.contains(&"if-shell") {
                self.converted.borrow_mut().insert(pane.into());
                String::new()
            } else {
                assert!(args.contains(&"show-options"));
                let converted = self.converted.borrow().contains(pane);
                match *args.last().unwrap() {
                    "@tmux-team.agent" if !converted && self.retired => raw + "\n",
                    "@tmt.agent" if converted => raw + "\n",
                    "@tmux-team.agent" | "@tmt.agent" | "@tmux-team.badge"
                    | "@tmux-team.border" => String::new(),
                    other => panic!("unexpected option {other}"),
                }
            };
            Ok(CommandOutput {
                stdout: text.into_bytes(),
                stderr: Vec::new(),
            })
        }
    }
    fn selected_caller() -> CallerEnvironment {
        CallerEnvironment {
            tmux: Some("/tmp/selected.sock,321,0".into()),
            pane: Some("%0".into()),
            process_id: 999,
            driver_env: Default::default(),
        }
    }
    #[test]
    fn exhausted_panes_do_not_starve_later_bindings_in_a_forty_pane_upgrade() {
        let directory = Directory::new();
        let database = directory.0.join("state.db");
        let entries = forty_bindings(&database);
        let mut runner = ManyPanes::new(&entries);
        runner.timeouts.extend(["%0".into(), "%20".into()]);
        let hints = migrate(&database, &selected_caller(), &runner);
        assert_eq!(runner.deadlines.borrow().len(), 40);
        assert_eq!(runner.converted.borrow().len(), 38);
        assert!(runner.converted.borrow().contains("%39"));
        assert_eq!(hints.len(), 3);
        assert!(hints[0].contains("tmt this Pane00"));
        assert!(hints[1].contains("tmt this Pane20"));
        assert_eq!(
            hints[2],
            "Pane option cutover: 38 converted, 2 left unchanged"
        );
        let mut storage = Storage::open(&database).unwrap();
        assert_eq!(
            storage
                .with_binding_transaction(|records| records.binding_entries())
                .unwrap(),
            entries
        );
        storage.close().unwrap();
    }
    #[test]
    fn panes_without_retired_markers_are_counted_without_a_rebind_hint_or_native_write() {
        let directory = Directory::new();
        let database = directory.0.join("state.db");
        let entries = forty_bindings(&database);
        let mut runner = ManyPanes::new(&entries);
        runner.retired = false;
        let hints = migrate(&database, &selected_caller(), &runner);
        assert_eq!(runner.seen.borrow().len(), 40);
        assert!(runner.deadlines.borrow().is_empty());
        assert!(runner.converted.borrow().is_empty());
        assert_eq!(
            hints,
            ["Pane option cutover: 0 converted, 40 left unchanged"]
        );
    }

    #[test]
    fn overall_cap_leaves_later_panes_untouched_and_reports_the_complete_count() {
        let directory = Directory::new();
        let database = directory.0.join("state.db");
        let entries = forty_bindings(&database);
        let clock = std::rc::Rc::new(std::cell::Cell::new(Instant::now()));
        let mut runner = ManyPanes::new(&entries);
        runner.overall_clock = Some(clock.clone());
        let hints = migrate_with_now(&database, &selected_caller(), &runner, || clock.get());
        assert_eq!(runner.deadlines.borrow().len(), 1);
        assert_eq!(
            *runner.converted.borrow(),
            std::collections::BTreeSet::from(["%0".into()])
        );
        assert_eq!(hints.len(), 40);
        assert_eq!(
            hints.last().unwrap(),
            "Pane option cutover: 1 converted, 39 left unchanged"
        );
    }

    #[test]
    fn a_concurrent_binding_change_refuses_the_captured_proof_without_native_writes() {
        use std::{cell::Cell, path::PathBuf};
        use tmt_adapters::process::runtime::ProcessObservation;
        use tmt_core::{
            binding::BindingEntry,
            endpoint::{PaneObservation, ProcessIncarnation, ServerEvidence},
            identity::{Lifetime, create_or_resolve},
        };
        struct Changed {
            database: PathBuf,
            entry: BindingEntry,
            raw: String,
            changed: Cell<bool>,
            calls: Cell<usize>,
        }
        impl CommandRunner for Changed {
            fn process_observation(&self, pid: u64, _: Instant) -> Option<ProcessObservation> {
                Some(ProcessObservation::Live(
                    ProcessIncarnation::new(pid, "captured").unwrap(),
                ))
            }
            fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
                self.calls.set(self.calls.get() + 1);
                let args: Vec<_> = request
                    .args
                    .iter()
                    .map(|arg| arg.to_str().unwrap())
                    .collect();
                assert_eq!(&args[..2], ["-S", "/tmp/selected.sock"]);
                let binding = self.entry.binding.as_ref().unwrap();
                let text = if args.contains(&"display-message") {
                    [
                        binding.server.server_id.as_str(),
                        "/tmp/selected.sock",
                        "321",
                        "1700000000",
                        "%9",
                        "e2e:0.0",
                        "/workspace",
                        "sh",
                        "900",
                        "1",
                        &self.raw,
                    ]
                    .join("__TMT_FIELD_4f1c__")
                        + "\n"
                } else {
                    assert!(
                        args.contains(&"show-options"),
                        "stale row must never reach a native write: {args:?}"
                    );
                    match args.last().unwrap() {
                        &"@tmux-team.agent" => self.raw.clone() + "\n",
                        &"@tmux-team.badge" => String::new(),
                        &"@tmux-team.border" => {
                            // A separate writer commits after the reader's capture
                            // and before its IMMEDIATE full-record fence.
                            let mut writer = Storage::open(&self.database).unwrap();
                            writer
                                .with_binding_transaction(|records| {
                                    records.detach_binding(&binding.id)
                                })
                                .unwrap();
                            writer.close().unwrap();
                            self.changed.set(true);
                            String::new()
                        }
                        other => panic!("unexpected option {other}"),
                    }
                };
                Ok(CommandOutput {
                    stdout: text.into_bytes(),
                    stderr: Vec::new(),
                })
            }
        }
        let directory = Directory::new();
        let database = directory.0.join("state.db");
        let mut storage = Storage::open(&database).unwrap();
        let identity = create_or_resolve(&mut storage, "Alice", Lifetime::Saved)
            .unwrap()
            .identity;
        let server = ServerEvidence {
            host: HostKind::Tmux,
            server_id: "00000000-0000-4000-8000-000000000001".into(),
            socket_path: "/tmp/selected.sock".into(),
            server_pid: 321,
            server_start_time: "1700000000".into(),
        };
        let pane = PaneObservation {
            id: "%9".into(),
            target: None,
            cwd: None,
            command: "sh".into(),
            pane_pid: 900,
            pane_incarnation: Some("captured".into()),
            suggested_name: None,
            marker: None,
        };
        let binding = storage
            .with_binding_transaction(|records| records.insert_binding(&identity, &server, &pane))
            .unwrap();
        storage.close().unwrap();
        let raw=serde_json::json!({"version":1,"globalIdentity":{"name":identity.name,"canonicalName":identity.canonical_name,"identityId":identity.id,"bindingId":binding.id,"serverId":server.server_id,"panePid":900}}).to_string();
        let runner = Changed {
            database: database.clone(),
            entry: BindingEntry {
                identity: identity.clone(),
                binding: Some(binding),
            },
            raw,
            changed: Cell::new(false),
            calls: Cell::new(0),
        };
        let caller = CallerEnvironment {
            tmux: Some("/tmp/selected.sock,321,0".into()),
            pane: Some("%9".into()),
            process_id: 999,
            driver_env: Default::default(),
        };
        let hints = migrate(&database, &caller, &runner);
        assert_eq!(hints.len(), 2);
        assert!(hints[0].contains("tmt this Alice"));
        assert_eq!(
            hints[1],
            "Pane option cutover: 0 converted, 1 left unchanged"
        );
        let mut storage = Storage::open(&database).unwrap();
        assert!(
            storage
                .with_binding_transaction(|records| records.entry_by_id(&identity.id))
                .unwrap()
                .unwrap()
                .binding
                .is_none()
        );
        storage.close().unwrap();
    }
}
