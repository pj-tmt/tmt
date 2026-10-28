//! Opt-in real-tmux regression; every command selects a task-owned socket.
use super::*;
use crate::{test_support::TestDirectory, tmux::server_id_tests::PrivateServer};
use std::{
    io::Write as _,
    process::{Child, ChildStdin, Command, Stdio},
    time::Duration,
};
use tmt_core::workspace::{
    WindowLink, WorkspacePane, WorkspaceServer, WorkspaceSession, WorkspaceWindow,
};

fn with_checksum(body: &str) -> String {
    let mut checksum = 0u16;
    for byte in body.bytes() {
        checksum = (checksum >> 1) | ((checksum & 1) << 15);
        checksum = checksum.wrapping_add(u16::from(byte));
    }
    format!("{checksum:04x},{body}")
}

/// Two windows of two side-by-side panes at 100x30; the second one is zoomed on its right pane.
fn saved(socket: &str, cwd: &str) -> WorkspaceSnapshot {
    let window = |id: &str, name: &str, first: u64, visible: Option<String>| {
        let body = format!("100x30,0,0{{49x30,0,0,{first},50x30,50,0,{}}}", first + 1);
        WorkspaceWindow {
            id: id.into(),
            name: name.into(),
            layout: with_checksum(&body),
            visible_layout: visible.unwrap_or_else(|| with_checksum(&body)),
            width: 100,
            height: 30,
            active_pane: format!("%{}", first + 1),
        }
    };
    // A zoomed window records its active pane as it is shown: the whole window.
    let panes = |window: &str, first: u64, zoomed: bool| {
        let window = window.to_owned();
        (first..first + 2).map(move |id| WorkspacePane {
            id: format!("%{id}"),
            window: window.clone(),
            index: id - first,
            left: if id == first || zoomed { 0 } else { 50 },
            top: 0,
            width: match (id == first, zoomed) {
                (_, true) if id != first => 100,
                (true, _) => 49,
                _ => 50,
            },
            height: 30,
            cwd: cwd.into(),
            identity: None,
            command: None,
        })
    };
    WorkspaceSnapshot {
        captured_at_ms: 1,
        server: WorkspaceServer {
            socket: socket.into(),
            process: ProcessIncarnation::new(99, "historical").unwrap(),
            id: None,
        },
        sessions: vec![WorkspaceSession {
            id: "$1".into(),
            name: "workspace".into(),
            windows: vec![
                WindowLink {
                    index: 0,
                    window: "@1".into(),
                    active: true,
                },
                WindowLink {
                    index: 1,
                    window: "@2".into(),
                    active: false,
                },
            ],
        }],
        windows: vec![
            window("@1", "plain", 1, None),
            window("@2", "zoomed", 3, Some(with_checksum("100x30,0,0,4"))),
        ],
        panes: panes("@1", 1, false).chain(panes("@2", 3, true)).collect(),
    }
}

/// A tmux control-mode client on the private socket; it sets the size a terminal would.
struct ControlClient {
    child: Child,
    input: Option<ChildStdin>,
}

impl ControlClient {
    fn attach(server: &PrivateServer, session: &str) -> Self {
        // The same proof as every other command: the socket is the task's own, never the host's.
        server.command(&["has-session", "-t", session]).unwrap();
        let mut child = Command::new("tmux")
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .args(["-S", &server.socket, "-C", "attach-session", "-t", session])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the control client");
        let input = child.stdin.take();
        Self { child, input }
    }

    fn resize(&mut self, columns: u32, rows: u32) {
        let input = self.input.as_mut().expect("client input");
        writeln!(input, "refresh-client -C {columns}x{rows}").unwrap();
        input.flush().unwrap();
    }
}

impl Drop for ControlClient {
    fn drop(&mut self) {
        // Closing its input ends the client; killing the child is the fallback for a stuck one.
        drop(self.input.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn window_size(server: &PrivateServer, window: &str) -> String {
    server
        .command(&[
            "display-message",
            "-p",
            "-t",
            window,
            "#{window_width}x#{window_height}",
        ])
        .unwrap()
        .trim()
        .to_owned()
}

/// What a window sets for itself, as opposed to what it inherits.
fn window_level_size_option(server: &PrivateServer, window: &str) -> String {
    server
        .command(&["show-options", "-w", "-t", window, "window-size"])
        .unwrap()
        .trim()
        .to_owned()
}

fn wait_for_size(server: &PrivateServer, window: &str, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while window_size(server, window) != expected {
        assert!(
            Instant::now() < deadline,
            "{window} stayed {} instead of following the client to {expected}",
            window_size(server, window)
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
#[ignore = "requires tmux; creates and stops only a private explicit-socket server"]
fn restored_windows_keep_the_saved_geometry_then_follow_a_client_resize() {
    let cwd = TestDirectory::new();
    let cwd_path = std::fs::canonicalize(&cwd.path).unwrap();
    let server = PrivateServer::new(TestDirectory::new(), "size.sock");
    server
        .command(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "seed",
            "-x",
            "80",
            "-y",
            "24",
            "sleep 600",
        ])
        .unwrap();
    // Panes the restore opens without a command must not start the user's shell.
    server
        .command(&["set-option", "-g", "default-command", "sleep 600"])
        .unwrap();

    let restored = Tmux::default()
        .workspace_restore_layout(
            &saved(&server.socket, cwd_path.to_str().unwrap()),
            Instant::now() + Duration::from_secs(30),
            &[],
        )
        .unwrap();
    assert!(restored.failures.is_empty(), "{:?}", restored.failures);
    assert_eq!(restored.windows.len(), 2);

    // Restore is finished and nothing has attached yet: the saved geometry holds, and no window
    // keeps a size override of its own.
    for window in &restored.windows {
        assert_eq!(window_size(&server, &window.native), "100x30");
        assert_eq!(
            window_level_size_option(&server, &window.native),
            "",
            "{} still pins its size",
            window.native
        );
    }
    let panes = |window: &str| {
        server
            .command(&[
                "list-panes",
                "-t",
                window,
                "-F",
                "#{pane_width}x#{pane_height}#{?window_zoomed_flag,z,}",
            ])
            .unwrap()
    };
    assert_eq!(panes(&restored.windows[0].native), "49x30\n50x30\n");
    assert_eq!(panes(&restored.windows[1].native), "49x30z\n100x30z\n");

    // A terminal now attaches and resizes: both restored windows follow it.
    let mut client = ControlClient::attach(&server, "workspace");
    client.resize(90, 20);
    for window in &restored.windows {
        wait_for_size(&server, &window.native, "90x20");
    }
    client.resize(120, 40);
    for window in &restored.windows {
        wait_for_size(&server, &window.native, "120x40");
    }
}
