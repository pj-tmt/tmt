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
