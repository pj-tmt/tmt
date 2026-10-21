//! One topology read. Optional markers annotate structure, never authorize IO.

use super::{
    Tmux, TmuxError, TmuxFailure,
    evidence::{SEPARATOR, wire_integer},
    metadata,
};
use crate::{
    host::{CallerEnvironment, WorkspaceCapture},
    process::{
        CommandRunner, ancestry,
        runtime::{ProcessObservation, observe_starts},
    },
};
use std::{
    collections::{BTreeMap, HashMap},
    time::Instant,
};
use tmt_core::{endpoint::ServerEvidence, host::HostKind, workspace::*};

pub(super) const COMMAND_OPTION: &str = "@tmt.workspace-command";

impl<R: CommandRunner> Tmux<R> {
    /// Eligibility only. Capture independently verifies native owner ancestry.
    pub fn workspace_pane_tty(
        &self,
        socket: &str,
        pane: &str,
        deadline: Instant,
    ) -> Result<String, TmuxError> {
        if !super::valid_pane_id(pane) {
            return Err(invalid());
        }
        let output = self.run(
            "tmux",
            vec![
                "-S".into(),
                socket.into(),
                "display-message".into(),
                "-p".into(),
                "-t".into(),
                pane.into(),
                "#{pane_tty}".into(),
            ],
            deadline,
            4096,
            TmuxFailure::Evidence,
        )?;
        let tty = output.strip_suffix('\n').unwrap_or(&output);
        if !tty.starts_with("/dev/") || tty.contains(['\n', '\r', '\0']) {
            return Err(invalid());
        }
        Ok(tty.to_owned())
    }

    /// Read names only, on an explicitly selected socket, without starting a server.
    pub fn workspace_session_names(
        &self,
        socket: &str,
        deadline: Instant,
    ) -> Result<Vec<String>, TmuxError> {
        let output = self.run(
            "tmux",
            vec![
                "-S".into(),
                socket.into(),
                "list-sessions".into(),
                "-F".into(),
                session_name_format(),
            ],
            deadline,
            MAX_BYTES,
            TmuxFailure::Evidence,
        )?;
        session_names(&output)
    }

    pub fn workspace_capture(
        &self,
        socket: &str,
        expected: Option<&ServerEvidence>,
        caller: Option<&CallerEnvironment>,
        deadline: Instant,
    ) -> Result<WorkspaceCapture, TmuxError> {
        let format = [
            "#{@tmt.server-id}",
            "#{socket_path}",
            "#{pid}",
            "#{start_time}",
            "#{session_id}",
            "#{session_name}",
            "#{window_id}",
            "#{window_index}",
            "#{window_active}",
            "#{window_name}",
            "#{window_layout}",
            "#{window_visible_layout}",
            "#{window_width}",
            "#{window_height}",
            "#{pane_id}",
            "#{pane_index}",
            "#{pane_active}",
            "#{pane_left}",
            "#{pane_top}",
            "#{pane_width}",
            "#{pane_height}",
            "#{pane_current_path}",
            "#{pane_pid}",
            "#{@tmt.agent}",
            "#{@tmt.workspace-command}",
            "#{pane_tty}",
        ]
        .join(SEPARATOR);
        let output = self.run(
            "tmux",
            vec![
                "-S".into(),
                socket.into(),
                "list-panes".into(),
                "-a".into(),
                "-F".into(),
                format,
            ],
            deadline,
            MAX_BYTES,
            TmuxFailure::Evidence,
        )?;
        let (mut capture, commands) = parse(&output, socket)?;
        if expected.is_some_and(|expected| capture.binding_server.as_ref() != Some(expected)) {
            return Err(invalid());
        }
        let mut pids = vec![capture.snapshot.server.process.pid()];
        pids.extend(capture.evidence.iter().map(|(_, pid, _)| *pid));
        pids.extend(commands.values().map(|(command, _, _)| command.owner.pid()));
        let starts = observe_starts(&self.runner, &pids, deadline).map_err(|_| invalid())?;
        capture.snapshot.server.process = starts.get(&pids[0]).cloned().ok_or_else(invalid)?;
        // One capture owns this native evidence cache. Missing native evidence
        // never starts a per-pane ps process; optional recovery simply omits it.
        let mut processes = HashMap::new();
        if let Some(caller) = caller {
            let pane = caller
                .pane
                .as_ref()
                .and_then(|value| value.to_str())
                .ok_or_else(invalid)?;
            let (_, pane_pid, _) = capture
                .evidence
                .iter()
                .find(|(id, _, _)| id == pane)
                .ok_or_else(invalid)?;
            if !native_contains(
                &self.runner,
                caller.process_id,
                *pane_pid,
                deadline,
                &mut processes,
            ) {
                return Err(invalid());
            }
        }
        for pane in &mut capture.snapshot.panes {
            if let Some((command, pane_pid, tty)) = commands.get(&pane.id)
                && starts.get(&command.owner.pid()) == Some(&command.owner)
                && crate::process::terminal::foreground(tty, command.owner.pid())
                && native_contains(
                    &self.runner,
                    command.owner.pid(),
                    *pane_pid,
                    deadline,
                    &mut processes,
                )
            {
                pane.command = Some(command.clone());
            }
        }
        capture.starts = starts;
        Ok(capture)
    }

    pub fn workspace_command_marker(
        &self,
        socket: &str,
        pane: &str,
        document: Option<&str>,
        deadline: Instant,
    ) -> Result<(), TmuxError> {
        if !super::valid_pane_id(pane) {
            return Err(invalid());
        }
        let mut args = vec!["-S".into(), socket.into(), "set-option".into(), "-p".into()];
        if document.is_none() {
            args.push("-u".into());
        }
        args.extend(["-t".into(), pane.into(), COMMAND_OPTION.into()]);
        if let Some(document) = document {
            args.push(document.into());
        }
        self.run("tmux", args, deadline, 4096, TmuxFailure::MetadataWrite)
            .map(|_| ())
    }
    pub fn clear_workspace_command(
        &self,
        socket: &str,
        pane: &str,
        expected: &str,
        deadline: Instant,
    ) -> Result<(), TmuxError> {
        if !super::valid_pane_id(pane)
            || expected.is_empty()
            || !expected
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(invalid());
        }
        // -F performs a server-side format test, not a shell invocation. The
        // marker's base64url alphabet cannot introduce format operators.
        self.run(
            "tmux",
            vec![
                "-S".into(),
                socket.into(),
                "if-shell".into(),
                "-F".into(),
                "-t".into(),
                pane.into(),
                format!("#{{==:#{{{COMMAND_OPTION}}},{expected}}}"),
                format!("set-option -pu -t {pane} {COMMAND_OPTION}"),
            ],
            deadline,
            4096,
            TmuxFailure::MetadataWrite,
        )
        .map(|_| ())
    }
}

/// A shared, bounded native process tree; no command fallback per pane. Compare
/// start order as well as parent ids so a reused ancestor is never accepted.
fn native_contains<R: CommandRunner>(
    runner: &R,
    first: u64,
    ancestor: u64,
    deadline: Instant,
    cache: &mut HashMap<u64, Option<ancestry::ProcessLink>>,
) -> bool {
    native_path(runner, first, ancestor, deadline, cache).is_some()
}
fn native_path<R: CommandRunner>(
    runner: &R,
    first: u64,
    ancestor: u64,
    deadline: Instant,
    cache: &mut HashMap<u64, Option<ancestry::ProcessLink>>,
) -> Option<()> {
    let mut chain = Vec::new();
    let mut pid = first;
    let mut child_start = None;
    for _ in 0..64 {
        if pid == 0 || chain.contains(&pid) || Instant::now() >= deadline {
            return None;
        }
        let link = cache
            .entry(pid)
            .or_insert_with(|| runner.process_link(pid, deadline))
            .as_ref()?;
        let (ProcessObservation::Live(process) | ProcessObservation::Stopped(process)) =
            &link.observation
        else {
            return None;
        };
        if process.pid() != pid || child_start.is_some_and(|start| link.start.later_than(start)) {
            return None;
        }
        chain.push(pid);
        if pid == ancestor {
            return (Instant::now() < deadline).then_some(());
        }
        if link.parent == 0 {
            return None;
        }
        child_start = Some(link.start);
        pid = link.parent;
    }
    None
}

type Commands = BTreeMap<String, (ExternalCommand, u64, String)>;

fn invalid() -> TmuxError {
    TmuxError::evidence("Workspace topology evidence is incomplete or inconsistent")
}
fn id(value: &str, prefix: char) -> bool {
    value.strip_prefix(prefix).and_then(wire_integer).is_some()
}
fn number(value: &str) -> Result<u64, TmuxError> {
    wire_integer(value).ok_or_else(invalid)
}
fn flag(value: &str) -> Result<bool, TmuxError> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(invalid()),
    }
}

// Flag controls on the server before a literal name can break newline framing.
pub(super) fn session_name_format() -> String {
    format!("#{{m/r:[[:cntrl:]],#{{session_name}}}}{SEPARATOR}#{{session_name}}")
}

pub(super) fn session_names(output: &str) -> Result<Vec<String>, TmuxError> {
    let names = output
        .split_terminator('\n')
        .map(|row| {
            let (controls, name) = row.split_once(SEPARATOR).ok_or_else(invalid)?;
            if controls != "0"
                || name.is_empty()
                || name.contains('\u{fffd}')
                || name.chars().any(char::is_control)
            {
                return Err(invalid());
            }
            Ok(name.to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if names.len() > MAX_PANES {
        return Err(invalid());
    }
    Ok(names)
}

fn parse(output: &str, socket: &str) -> Result<(WorkspaceCapture, Commands), TmuxError> {
    let rows: Vec<Vec<&str>> = output
        .lines()
        .map(|row| row.split(SEPARATOR).collect())
        .collect();
    let first = rows
        .first()
        .filter(|row| row.len() == 26)
        .ok_or_else(invalid)?;
    if rows.len() > MAX_PANES * MAX_PANES
        || first[1] != socket
        || first[3].is_empty()
        || rows
            .iter()
            .any(|row| row.len() != 26 || row[..4] != first[..4])
    {
        return Err(invalid());
    }
    let pid = number(first[2])?;
    let process =
        tmt_core::endpoint::ProcessIncarnation::new(pid, first[3]).map_err(|_| invalid())?;
    let server_id = if first[0].is_empty() {
        None
    } else if tmt_core::endpoint::valid_server_id(first[0]) {
        Some(first[0].to_owned())
    } else {
        return Err(invalid());
    };
    let binding_server = server_id.as_ref().map(|id| ServerEvidence {
        host: HostKind::Tmux,
        server_id: id.clone(),
        socket_path: socket.into(),
        server_pid: pid,
        server_start_time: first[3].into(),
    });
    let mut sessions: BTreeMap<String, WorkspaceSession> = BTreeMap::new();
    let mut windows: BTreeMap<String, WorkspaceWindow> = BTreeMap::new();
    let mut panes: BTreeMap<String, WorkspacePane> = BTreeMap::new();
    let mut evidence = BTreeMap::new();
    let mut commands = BTreeMap::new();
    let mut terminals = BTreeMap::new();
    for row in rows {
        if !id(row[4], '$') || !id(row[6], '@') || !id(row[14], '%') {
            return Err(invalid());
        }
        let session = sessions
            .entry(row[4].into())
            .or_insert_with(|| WorkspaceSession {
                id: row[4].into(),
                name: row[5].into(),
                windows: Vec::new(),
            });
        if session.name != row[5] {
            return Err(invalid());
        }
        let link = WindowLink {
            index: number(row[7])?,
            window: row[6].into(),
            active: flag(row[8])?,
        };
        if let Some(previous) = session
            .windows
            .iter()
            .find(|value| value.index == link.index)
        {
            if previous != &link {
                return Err(invalid());
            }
        } else {
            session.windows.push(link);
        }
        let window = WorkspaceWindow {
            id: row[6].into(),
            name: row[9].into(),
            layout: row[10].into(),
            visible_layout: row[11].into(),
            width: number(row[12])?,
            height: number(row[13])?,
            active_pane: String::new(),
        };
        let previous = windows
            .entry(window.id.clone())
            .or_insert_with(|| window.clone());
        if previous.name != window.name
            || previous.layout != window.layout
            || previous.visible_layout != window.visible_layout
            || previous.width != window.width
            || previous.height != window.height
        {
            return Err(invalid());
        }
        if flag(row[16])? {
            if !previous.active_pane.is_empty() && previous.active_pane != row[14] {
                return Err(invalid());
            }
            previous.active_pane = row[14].into();
        }
        let pane = WorkspacePane {
            id: row[14].into(),
            window: row[6].into(),
            index: number(row[15])?,
            left: number(row[17])?,
            top: number(row[18])?,
            width: number(row[19])?,
            height: number(row[20])?,
            cwd: row[21].into(),
            identity: None,
            command: None,
        };
        let pid = number(row[22])?;
        if !tmt_core::endpoint::valid_process_id(pid) {
            return Err(invalid());
        }
        let marker = metadata::marker(&metadata::decode(row[23]));
        if panes
            .get(&pane.id)
            .is_some_and(|previous| previous != &pane)
            || evidence.get(&pane.id).is_some_and(|previous| {
                previous != &(pid, marker.clone(), row[23].to_owned(), row[24].to_owned())
            })
        {
            return Err(invalid());
        }
        if let Some(command) = crate::workspace::decode_command(row[24]) {
            commands.insert(pane.id.clone(), (command, pid, row[25].into()));
        }
        terminals.insert(pane.id.clone(), row[25].to_owned());
        evidence.insert(
            pane.id.clone(),
            (pid, marker, row[23].to_owned(), row[24].to_owned()),
        );
        panes.insert(pane.id.clone(), pane);
    }
    if panes.len() > MAX_PANES || windows.values().any(|window| window.active_pane.is_empty()) {
        return Err(invalid());
    }
    for session in sessions.values_mut() {
        session.windows.sort_by_key(|link| link.index);
    }
    Ok((
        WorkspaceCapture {
            snapshot: WorkspaceSnapshot {
                captured_at_ms: 0,
                server: WorkspaceServer {
                    socket: socket.into(),
                    process,
                    id: server_id,
                },
                sessions: sessions.into_values().collect(),
                windows: windows.into_values().collect(),
                panes: panes.into_values().collect(),
            },
            evidence: evidence
                .into_iter()
                .map(|(id, (pid, marker, _, _))| (id, pid, marker))
                .collect(),
            binding_server,
            terminals,
            starts: std::collections::HashMap::new(),
        },
        commands,
    ))
}

#[cfg(test)]
mod capture_process_tests {
    use super::*;
    use crate::process::{CommandError, CommandOutput, CommandRequest};
    use std::cell::RefCell;
    struct NativeTree {
        calls: RefCell<Vec<u64>>,
        links: HashMap<u64, ancestry::ProcessLink>,
    }
    impl CommandRunner for NativeTree {
        fn execute(&self, _: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            panic!("capture ancestry must never spawn a per-pane process");
        }
        fn process_link(&self, pid: u64, _: Instant) -> Option<ancestry::ProcessLink> {
            self.calls.borrow_mut().push(pid);
            self.links.get(&pid).cloned()
        }
    }
    fn link(pid: u64, parent: u64, start: u64) -> ancestry::ProcessLink {
        ancestry::ProcessLink {
            observation: ProcessObservation::Live(
                tmt_core::endpoint::ProcessIncarnation::new(pid, &format!("start-{start}"))
                    .unwrap(),
            ),
            parent,
            start: ancestry::StartOrder::precise(start, 0),
        }
    }
    #[test]
    fn capture_native_ancestry_is_shared_and_never_falls_back_per_pane() {
        let tree = NativeTree {
            calls: RefCell::new(Vec::new()),
            links: HashMap::from([
                (30, link(30, 20, 3)),
                (31, link(31, 20, 3)),
                (20, link(20, 1, 2)),
                (1, link(1, 0, 1)),
            ]),
        };
        let mut cache = HashMap::new();
        let deadline = Instant::now() + std::time::Duration::from_secs(1);
        assert!(native_contains(&tree, 30, 20, deadline, &mut cache));
        assert!(native_contains(&tree, 31, 20, deadline, &mut cache));
        assert_eq!(*tree.calls.borrow(), vec![30, 20, 31]);
        assert!(!native_contains(&tree, 99, 20, deadline, &mut cache));
        assert!(!native_contains(&tree, 99, 20, deadline, &mut cache));
        assert_eq!(
            tree.calls.borrow().iter().filter(|&&pid| pid == 99).count(),
            1
        );
        let reused = NativeTree {
            calls: RefCell::new(Vec::new()),
            links: HashMap::from([(30, link(30, 20, 3)), (20, link(20, 1, 4))]),
        };
        assert!(!native_contains(
            &reused,
            30,
            20,
            deadline,
            &mut HashMap::new()
        ));
        let cycle = NativeTree {
            calls: RefCell::new(Vec::new()),
            links: HashMap::from([(20, link(20, 20, 1))]),
        };
        assert!(!native_contains(
            &cycle,
            20,
            1,
            deadline,
            &mut HashMap::new()
        ));
    }
}
