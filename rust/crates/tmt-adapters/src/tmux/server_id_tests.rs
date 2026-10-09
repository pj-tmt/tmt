//! Opt-in real-tmux regression; every command selects a task-owned socket.
use super::*;
use crate::{process::CommandOutput, test_support::TestDirectory};
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct FirstReadBarrier {
    barrier: Arc<Barrier>,
    first: AtomicBool,
    refused_sets: Arc<AtomicUsize>,
}

impl CommandRunner for FirstReadBarrier {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        let first_read = request.args.iter().any(|arg| arg == "show-options")
            && self.first.swap(false, Ordering::SeqCst);
        let setting = request.args.iter().any(|arg| arg == "set-option");
        let result = UnixCommandRunner.execute(request);
        if first_read {
            // Every real read finishes before any caller can attempt its set.
            self.barrier.wait();
            assert!(result.is_err(), "fresh server unexpectedly had an ID");
        }
        if setting && result.is_err() {
            self.refused_sets.fetch_add(1, Ordering::SeqCst);
        }
        result
    }
}

struct PrivateServer {
    directory: Option<TestDirectory>,
    socket: String,
}

impl PrivateServer {
    fn command(&self, args: &[&str]) -> Result<String, TmuxError> {
        let mut selected = socket_args(Some(&self.socket));
        selected.extend(args.iter().map(|arg| (*arg).into()));
        Tmux::default().execute(selected, OperationOptions::default(), TmuxFailure::Command)
    }
}

impl Drop for PrivateServer {
    fn drop(&mut self) {
        let _ = self.command(&["kill-server"]);
        let stopped = self.command(&["list-sessions"]);
        let absent = stopped.as_ref().err().is_some_and(|error| {
            !error.cleanup_failed()
                && error.cause.as_ref().is_some_and(|cause| {
                    matches!(
                        cause.kind,
                        CommandFailure::Exit {
                            code: Some(1),
                            signal: None
                        }
                    ) && cause.output.as_ref().is_some_and(|output| {
                        std::str::from_utf8(&output.stderr).is_ok_and(|text| {
                            text.starts_with("no server running on ")
                                || (text.starts_with("error connecting to ")
                                    && text.trim_end().ends_with("(No such file or directory)"))
                        })
                    })
                })
        });
        if !absent {
            // Retain the directory if cleanup cannot be confirmed.
            let directory = self.directory.take().unwrap();
            let path = directory.path.clone();
            std::mem::forget(directory);
            eprintln!("Private tmux cleanup unconfirmed: {}", path.display());
            if !std::thread::panicking() {
                panic!("private tmux server cleanup failed");
            }
        }
    }
}

#[test]
#[ignore = "requires tmux; creates and stops only a private explicit-socket server"]
fn server_id_concurrent_callers_adopt_one_id_on_a_fresh_server() {
    const CALLERS: usize = 8;
    let directory = TestDirectory::new();
    let socket = directory
        .path
        .join("race.sock")
        .to_str()
        .unwrap()
        .to_owned();
    let server = PrivateServer {
        directory: Some(directory),
        socket,
    };
    server
        .command(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "race",
            "sleep 60",
        ])
        .unwrap();
    let barrier = Arc::new(Barrier::new(CALLERS));
    let refused_sets = Arc::new(AtomicUsize::new(0));
    let ids = std::thread::scope(|scope| {
        let callers: Vec<_> = (0..CALLERS)
            .map(|_| {
                let runner = FirstReadBarrier {
                    barrier: barrier.clone(),
                    first: AtomicBool::new(true),
                    refused_sets: refused_sets.clone(),
                };
                let socket = &server.socket;
                scope.spawn(move || {
                    Tmux::new(runner).ensure_server_id_on(Some(socket), OperationOptions::default())
                })
            })
            .collect();
        callers
            .into_iter()
            .map(|caller| caller.join().unwrap())
            .collect::<Vec<_>>()
    });
    let ids: Vec<_> = ids.into_iter().map(Result::unwrap).collect();
    assert_eq!(refused_sets.load(Ordering::SeqCst), CALLERS - 1);
    assert!(valid_server_id(&ids[0]));
    assert!(ids.iter().all(|id| id == &ids[0]));
    assert_eq!(
        server
            .command(&["show-options", "-s", "-v", SERVER_ID_OPTION])
            .unwrap()
            .trim(),
        ids[0]
    );
}

#[test]
#[ignore = "requires tmux; creates and stops only a private explicit-socket server"]
fn option_rename_compares_native_binding_and_exact_values_without_editing_user_formats() {
    use tmt_core::{
        binding::{Binding, BindingEntry},
        identity::{Identity, Lifetime},
    };
    let directory = TestDirectory::new();
    let server = PrivateServer {
        socket: directory
            .path
            .join("rename.sock")
            .to_str()
            .unwrap()
            .to_owned(),
        directory: Some(directory),
    };
    server
        .command(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "rename",
            "/bin/sleep 60",
        ])
        .unwrap();
    let id = Tmux::default()
        .ensure_server_id_on(Some(&server.socket), OperationOptions::default())
        .unwrap();
    let raw_endpoint = server
        .command(&["display-message", "-p", &evidence::endpoint_format()])
        .unwrap();
    let snapshot = evidence::parse_snapshot(&raw_endpoint, Some(&id)).unwrap();
    let pane = &snapshot.panes[0];
    let deadline = Instant::now() + Duration::from_secs(10);
    let starts =
        crate::process::runtime::observe_starts(&UnixCommandRunner, &[pane.pane_pid], deadline)
            .unwrap();
    let identity = Identity {
        id: "00000000-0000-4000-8000-000000000001".into(),
        name: "A, #} 雪".into(),
        canonical_name: "a, #} 雪".into(),
        lifetime: Lifetime::Saved,
        created_at: "2026-10-09T00:00:00Z".into(),
        updated_at: "2026-10-09T00:00:00Z".into(),
    };
    let binding = Binding {
        id: "00000000-0000-4000-8000-000000000002".into(),
        identity_id: identity.id.clone(),
        server: snapshot.server,
        pane_id: pane.id.clone(),
        pane_pid: pane.pane_pid,
        pane_incarnation: Some(starts[&pane.pane_pid].start_identity().into()),
        session: Default::default(),
    };
    let mut document = serde_json::json!({"version": 1, "foreign": "preserve #}, \" ; $HOME"});
    metadata::replace(&mut document, &binding.marker(&identity));
    let raw = document.to_string();
    let entry = BindingEntry {
        identity,
        binding: Some(binding.clone()),
    };
    let pane = binding.pane_id.as_str();
    let prepare = || {
        server
            .command(&["set-option", "-s", "@tmux-team.server-id", &id])
            .unwrap();
        server
            .command(&["set-option", "-su", "@tmt.server-id"])
            .unwrap();
        server
            .command(&["set-option", "-pu", "-t", pane, "@tmt.agent"])
            .unwrap();
        server
            .command(&["set-option", "-p", "-t", pane, "@tmux-team.agent", &raw])
            .unwrap();
        Tmux::default()
            .prepare_option_rename(&entry, Instant::now() + Duration::from_secs(10))
            .unwrap()
            .unwrap()
    };
    let proof = prepare();
    let changed = format!("{raw} ");
    server
        .command(&["set-option", "-p", "-t", pane, "@tmux-team.agent", &changed])
        .unwrap();
    assert!(
        !Tmux::default()
            .commit_option_rename(&proof, Instant::now() + Duration::from_secs(10))
            .unwrap()
    );
    assert_eq!(
        server
            .command(&["show-options", "-pv", "-t", pane, "@tmux-team.agent"])
            .unwrap()
            .trim_end_matches('\n'),
        changed
    );
    let proof = prepare();
    server
        .command(&["set-option", "-p", "-t", pane, "@tmt.agent", "user owned"])
        .unwrap();
    assert!(
        !Tmux::default()
            .commit_option_rename(&proof, Instant::now() + Duration::from_secs(10))
            .unwrap()
    );
    assert_eq!(
        server
            .command(&["show-options", "-pv", "-t", pane, "@tmt.agent"])
            .unwrap(),
        "user owned\n"
    );
    let mut unknown = entry.clone();
    unknown.binding.as_mut().unwrap().pane_incarnation = None;
    assert!(
        Tmux::default()
            .prepare_option_rename(&unknown, Instant::now() + Duration::from_secs(10))
            .is_err()
    );
    let prefix = "#{?@tmux-team.badge, [#{@tmux-team.badge}],}";
    let owned = format!("{prefix}theme:#{{pane_index}}, 雪");
    server
        .command(&[
            "set-option",
            "-p",
            "-t",
            pane,
            "@tmux-team.badge",
            "alice, #} 雪",
        ])
        .unwrap();
    server
        .command(&["set-option", "-p", "-t", pane, "@tmux-team.border", &owned])
        .unwrap();
    server
        .command(&["set-option", "-p", "-t", pane, "pane-border-format", &owned])
        .unwrap();
    let global_before = server
        .command(&["show-options", "-gwv", "pane-border-format"])
        .unwrap();
    let proof = prepare();
    assert!(
        Tmux::default()
            .commit_option_rename(&proof, Instant::now() + Duration::from_secs(10))
            .unwrap()
    );
    assert_eq!(
        server
            .command(&["show-options", "-pv", "-t", pane, "@tmt.agent"])
            .unwrap(),
        format!("{raw}\n")
    );
    assert_eq!(
        server
            .command(&["show-options", "-pqv", "-t", pane, "@tmux-team.agent"])
            .unwrap(),
        ""
    );
    let next = owned.replacen(prefix, "#{?@tmt.badge, [#{@tmt.badge}],}", 1);
    assert_eq!(
        server
            .command(&["show-options", "-pv", "-t", pane, "pane-border-format"])
            .unwrap(),
        format!("{next}\n")
    );
    assert_eq!(
        server
            .command(&["show-options", "-pv", "-t", pane, "@tmt.border"])
            .unwrap(),
        format!("{next}\n")
    );
    assert_eq!(
        server
            .command(&["show-options", "-gwv", "pane-border-format"])
            .unwrap(),
        global_before
    );
    assert!(
        Tmux::default()
            .prepare_option_rename(&entry, Instant::now() + Duration::from_secs(10))
            .unwrap()
            .is_none()
    );
}
