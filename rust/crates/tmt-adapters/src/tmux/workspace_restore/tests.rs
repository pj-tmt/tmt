use super::*;

#[test]
fn nested_command_quotes_literal_bytes_without_expanding_saved_data() {
    // Multibyte fixture bytes verify literal UTF-8 preservation by the tmux lexer.
    let value = "'\"$HOME;#(touch /tmp/no)\n\\日本";
    let encoded = command(&[value.into()]);
    assert!(!encoded.contains("$HOME"));
    assert!(!encoded.contains("#("));
    assert!(!encoded.contains('\n'));
    let decoded: Vec<_> = encoded.as_bytes()[1..encoded.len() - 1]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| u8::from_str_radix(std::str::from_utf8(&chunk[1..]).unwrap(), 8).unwrap())
        .collect();
    assert_eq!(decoded, value.as_bytes());
    assert_eq!(format_literal("#(test)#{pid}"), "##(test)##{pid}");
    assert_eq!(argv_literal("literal\\;"), "literal\\\\;");
}

use crate::{
    process::{CommandError, CommandOutput, CommandRequest},
    scripted_runner::{ScriptedRunner, failure},
};
use std::{cell::Cell, time::Duration};

#[derive(Default)]
struct Runner {
    scripted: ScriptedRunner,
    replaced: Cell<bool>,
}
impl CommandRunner for Runner {
    fn process_observation(&self, pid: u64, _: Instant) -> Option<ProcessObservation> {
        Some(ProcessObservation::Live(
            ProcessIncarnation::new(
                pid,
                if self.replaced.get() {
                    "replacement"
                } else {
                    "owned-start"
                },
            )
            .unwrap(),
        ))
    }
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        self.scripted.execute(request)
    }
}
fn restore(tmux: &Tmux<Runner>) -> Restore<'_, Runner> {
    Restore {
        tmux,
        socket: "/tmp/selected",
        deadline: Instant::now() + Duration::from_secs(5),
        fence: Some(Fence {
            server: ProcessIncarnation::new(10, "owned-start").unwrap(),
            native_start: 22,
        }),
        windows: HashMap::new(),
        panes: HashMap::new(),
        outcome: LayoutRestore::default(),
    }
}
fn created_output(session: u64, window: u64, pane: u64, index: u64) -> String {
    format!(
        "/tmp/selected{SEP}10{SEP}22{SEP}${session}{SEP}@{window}{SEP}%{pane}{SEP}20{SEP}{index}\n"
    )
}
fn snapshot() -> WorkspaceSnapshot {
    use tmt_core::workspace::*;
    WorkspaceSnapshot {
        captured_at_ms: 1,
        server: WorkspaceServer {
            socket: "/tmp/selected".into(),
            process: ProcessIncarnation::new(99, "historical").unwrap(),
            id: None,
        },
        sessions: vec![WorkspaceSession {
            id: "$1".into(),
            name: "workspace".into(),
            windows: vec![WindowLink {
                index: 0,
                window: "@1".into(),
                active: true,
            }],
        }],
        windows: vec![WorkspaceWindow {
            id: "@1".into(),
            name: "shell".into(),
            layout: "b25e,80x24,0,0,1".into(),
            visible_layout: "b25e,80x24,0,0,1".into(),
            width: 80,
            height: 24,
            active_pane: "%1".into(),
        }],
        panes: vec![WorkspacePane {
            id: "%1".into(),
            window: "@1".into(),
            index: 0,
            left: 0,
            top: 0,
            width: 80,
            height: 24,
            cwd: "/tmp".into(),
            identity: None,
            command: None,
        }],
    }
}

#[test]
fn invalid_saved_layout_refuses_before_any_host_operation() {
    let tmux = Tmux::new(Runner::default());
    let mut saved = snapshot();
    saved.windows[0].layout = "bad".into();
    assert!(
        tmux.workspace_restore_layout(&saved, Instant::now() + Duration::from_secs(1))
            .is_err()
    );
    assert!(tmux.runner.scripted.calls.borrow().is_empty());
}

#[test]
fn native_server_replacement_refuses_before_effect_and_tmux_false_guard_is_observable() {
    let tmux = Tmux::new(Runner::default());
    let owner = restore(&tmux);
    tmux.runner.replaced.set(true);
    assert!(
        owner
            .guard(
                Some("%7"),
                &[],
                vec!["select-pane".into(), "-t".into(), "%7".into()]
            )
            .is_err()
    );
    assert!(tmux.runner.scripted.calls.borrow().is_empty());
    tmux.runner.replaced.set(false);
    tmux.runner
        .scripted
        .push_output(format!("{REFUSED}\n").into_bytes(), vec![]);
    assert!(
        owner
            .guard(
                Some("%7"),
                &[],
                vec!["select-pane".into(), "-t".into(), "%7".into()]
            )
            .is_err()
    );
    let calls = tmux.runner.scripted.calls.borrow();
    assert_eq!(
        &calls[0].args[..7],
        ["-u", "-S", "/tmp/selected", "if-shell", "-F", "-t", "%7"]
    );
    assert!(calls[0].args[7].contains("#{pid},10"));
    assert!(calls[0].args[7].contains("#{start_time},22"));
}

#[test]
fn returned_coordinates_are_validated_and_never_adopt_snapshot_incarnation() {
    let tmux = Tmux::new(Runner::default());
    let mut owner = restore(&tmux);
    let new = owner.created(&created_output(2, 3, 4, 5)).unwrap();
    assert_eq!(
        (&new.session, &new.window, &new.pane, new.index),
        (&"$2".into(), &"@3".into(), &"%4".into(), 5)
    );
    for wrong in [
        created_output(2, 3, 4, 5).replace("/tmp/selected", "/tmp/foreign"),
        created_output(2, 3, 4, 5).replace("@3", "old:target"),
        created_output(2, 3, 4, 5).replace(&format!("{SEP}22{SEP}"), &format!("{SEP}23{SEP}")),
    ] {
        assert!(owner.created(&wrong).is_err());
    }
}

#[test]
fn successful_link_is_required_before_bootstrap_removal_and_failed_link_retains_it() {
    let tmux = Tmux::new(Runner::default());
    let mut owner = restore(&tmux);
    let saved = snapshot();
    let plan = layout_creation(&saved, &[]).unwrap();
    owner.windows.insert(0, "@3".into());
    let links = vec![tmt_core::workspace::layout::WorkspaceLayoutLink {
        index: 0,
        window: 0,
        create: false,
        active: true,
    }];
    tmux.runner
        .scripted
        .push_output(created_output(2, 4, 5, 1).into_bytes(), vec![]);
    tmux.runner
        .scripted
        .results
        .borrow_mut()
        .push_back(Err(failure(false)));
    let mut record = RestoredSession {
        recorded: "$1".into(),
        name: "workspace".into(),
        action: "partial".into(),
        native: None,
        retained_bootstrap: None,
    };
    assert!(
        owner
            .session(
                &plan,
                &saved.sessions[0],
                None,
                Some(1),
                &links,
                &mut record
            )
            .is_err()
    );
    assert_eq!(record.native.as_deref(), Some("$2"));
    assert_eq!(record.retained_bootstrap.as_deref(), Some("%5"));
    let calls = tmux.runner.scripted.calls.borrow();
    assert_eq!(calls.len(), 2);
    let branch = &calls[1].args[calls[1].args.len() - 2];
    assert_eq!(
        branch,
        &command(&[
            "link-window".into(),
            "-d".into(),
            "-s".into(),
            "@3".into(),
            "-t".into(),
            "$2:0".into()
        ])
    );
}

#[test]
fn bootstrap_pid_or_process_mismatch_removes_nothing() {
    let tmux = Tmux::new(Runner::default());
    let mut owner = restore(&tmux);
    let created = owner.created(&created_output(2, 3, 4, 0)).unwrap();
    tmux.runner.replaced.set(true);
    assert!(owner.remove_bootstrap(&created).is_err());
    assert!(tmux.runner.scripted.calls.borrow().is_empty());
    tmux.runner.replaced.set(false);
    tmux.runner.scripted.push_output(
        format!("%4{SEP}99{SEP}sh{SEP}/dev/tty{SEP}/bin/sh -i\n").into_bytes(),
        vec![],
    );
    assert!(owner.remove_bootstrap(&created).is_err());
    assert_eq!(tmux.runner.scripted.calls.borrow().len(), 1);
    assert_eq!(
        tmux.runner.scripted.calls.borrow()[0].args[8],
        command(&[
            "display-message".into(),
            "-p".into(),
            "-t".into(),
            "%4".into(),
            [
                "#{pane_id}",
                "#{pane_pid}",
                "#{pane_current_command}",
                "#{pane_tty}",
                "#{pane_start_command}"
            ]
            .join(SEP)
        ])
    );
}

#[test]
fn directory_readback_refuses_silent_tmux_fallback_and_preserves_literal_newlines() {
    let tmux = Tmux::new(Runner::default());
    let owner = restore(&tmux);
    tmux.runner
        .scripted
        .push_output(b"/fallback\n".to_vec(), vec![]);
    assert!(owner.verify_cwd("%7", "/recorded").is_err());
    tmux.runner
        .scripted
        .push_output(b"/recorded\nnewline;\n".to_vec(), vec![]);
    assert!(owner.verify_cwd("%7", "/recorded\nnewline;").is_ok());
    assert_eq!(tmux.runner.scripted.calls.borrow().len(), 2);
}

#[test]
fn stale_socket_requires_kernel_refusal_and_leaves_live_and_closed_inodes_untouched() {
    use std::{
        os::unix::{fs::MetadataExt, net::UnixListener},
        path::PathBuf,
    };
    struct OwnedSocket(PathBuf);
    impl Drop for OwnedSocket {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let path = OwnedSocket(PathBuf::from(format!(
        "/tmp/tmt-restore-{}.sock",
        uuid::Uuid::new_v4()
    )));
    let listener = UnixListener::bind(&path.0).unwrap();
    let before = std::fs::metadata(&path.0).unwrap().ino();
    let deadline = Instant::now() + Duration::from_secs(1);
    assert!(!stale_socket(path.0.to_str().unwrap(), deadline));
    assert_eq!(std::fs::metadata(&path.0).unwrap().ino(), before);
    let tmux = Tmux::new(Runner::default());
    let mut saved = snapshot();
    saved.server.socket = path.0.to_str().unwrap().into();
    tmux.runner.scripted.results.borrow_mut().push_back(Err(
        crate::scripted_runner::failure_with_kind(
            crate::process::CommandFailure::Exit {
                code: Some(1),
                signal: None,
            },
            false,
        ),
    ));
    assert!(tmux.workspace_restore_layout(&saved, deadline).is_err());
    assert_eq!(tmux.runner.scripted.calls.borrow().len(), 1);
    assert_eq!(std::fs::metadata(&path.0).unwrap().ino(), before);
    drop(listener);
    assert!(stale_socket(path.0.to_str().unwrap(), deadline));
    assert_eq!(std::fs::metadata(&path.0).unwrap().ino(), before);
    assert!(!stale_socket(path.0.to_str().unwrap(), Instant::now()));
    assert!(!stale_socket(
        "/tmp/nonexistent-tmt-restore-socket",
        deadline
    ));
}

#[test]
fn bootstrap_with_a_different_start_command_is_retained_before_terminal_or_kill_checks() {
    let tmux = Tmux::new(Runner::default());
    let mut owner = restore(&tmux);
    let created = owner.created(&created_output(2, 3, 4, 0)).unwrap();
    tmux.runner.scripted.push_output(
        format!("%4{SEP}20{SEP}sh{SEP}/dev/tty{SEP}/bin/sh user-script\n").into_bytes(),
        vec![],
    );
    assert!(owner.remove_bootstrap(&created).is_err());
    assert_eq!(tmux.runner.scripted.calls.borrow().len(), 1);
}

#[test]
fn commandless_user_window_guard_has_one_literal_final_cwd_word() {
    let tmux = Tmux::new(Runner::default());
    let owner = restore(&tmux);
    tmux.runner.scripted.push_output(Vec::new(), Vec::new());
    owner
        .guard(
            None,
            &[],
            vec![
                "new-window".into(),
                "-d".into(),
                "-t".into(),
                "$2:0".into(),
                "-n".into(),
                "literal;$name".into(),
                "-c".into(),
                "/tmp/'$cwd\n;".into(),
            ],
        )
        .unwrap();
    let calls = tmux.runner.scripted.calls.borrow();
    assert_eq!(
        &calls[0].args[..5],
        ["-u", "-S", "/tmp/selected", "if-shell", "-F"]
    );
    assert_eq!(
        calls[0].args[6],
        r#""\156\145\167\055\167\151\156\144\157\167" "\055\144" "\055\164" "\044\062\072\060" "\055\156" "\154\151\164\145\162\141\154\073\044\156\141\155\145" "\055\143" "\057\164\155\160\057\047\044\143\167\144\012\073""#
    );
    assert_eq!(
        calls[0].args[7],
        "display-message -p tmt-workspace-effect-refused"
    );
    assert!(SEP.bytes().all(|byte| byte.is_ascii_graphic()));
}
