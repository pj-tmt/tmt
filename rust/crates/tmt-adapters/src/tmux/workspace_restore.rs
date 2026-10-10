//! Workspace recovery effects. Saved IDs never become live targets.

use super::{Tmux, TmuxError, TmuxFailure};
use crate::{
    process::{
        CommandRunner,
        runtime::{ProcessObservation, observe_runtime_process},
    },
    workspace::restore::{LayoutRestore, RestoredPane, RestoredSession, RestoredWindow},
};
use std::{collections::HashMap, fmt::Write, io, os::unix::fs::FileTypeExt, time::Instant};
use tmt_core::{
    endpoint::ProcessIncarnation,
    workspace::{
        WorkspaceSnapshot,
        layout::{
            WorkspaceLayoutPlan, WorkspaceLayoutSessionAction, WorkspaceLayoutWindow,
            WorkspaceSplitAxis, layout_creation,
        },
    },
};

const REFUSED: &str = "tmt-workspace-effect-refused";
const SEP: &str = super::evidence::SEPARATOR;
const BOOTSTRAP_COMMAND: [&str; 2] = ["/bin/sh", "-i"];
fn created_format() -> String {
    [
        "#{socket_path}",
        "#{pid}",
        "#{start_time}",
        "#{session_id}",
        "#{window_id}",
        "#{pane_id}",
        "#{pane_pid}",
        "#{window_index}",
    ]
    .join(SEP)
}

struct Fence {
    server: ProcessIncarnation,
    native_start: u64,
}

struct Created {
    session: String,
    window: String,
    pane: String,
    process: ProcessIncarnation,
    index: u64,
}

struct Restore<'a, R> {
    tmux: &'a Tmux<R>,
    socket: &'a str,
    deadline: Instant,
    fence: Option<Fence>,
    windows: HashMap<usize, String>,
    panes: HashMap<String, String>,
    outcome: LayoutRestore,
    revival_panes: &'a [String],
}

impl<R: CommandRunner> Tmux<R> {
    /// Replace only the idle shell created for this invocation's revival phase.
    /// Explicit user-shell startup quotes every argument and never types input.
    pub fn workspace_start_pane(
        &self,
        snapshot: &tmt_core::workspace::WorkspaceSnapshot,
        layout: &LayoutRestore,
        pane: &RestoredPane,
        argv: &[String],
        deadline: Instant,
    ) -> Result<(), TmuxError> {
        let socket = snapshot.server.socket.as_str();
        let cwd = &snapshot
            .panes
            .iter()
            .find(|saved| saved.id == pane.recorded)
            .ok_or_else(invalid)?
            .cwd;
        let shell = pane.shell.as_ref().ok_or_else(invalid)?;
        let (server, native_start) = layout.server.as_ref().ok_or_else(invalid)?;
        if argv.len() < 2 || argv.iter().any(|arg| arg.contains('\0')) {
            return Err(invalid());
        }
        let restore = Restore {
            tmux: self,
            socket,
            deadline,
            fence: Some(Fence {
                server: server.clone(),
                native_start: *native_start,
            }),
            windows: HashMap::new(),
            panes: HashMap::new(),
            outcome: LayoutRestore::default(),
            revival_panes: &[],
        };
        if restore.live(shell.pid())? != *shell {
            return Err(invalid());
        }
        let output = restore.run(vec![
            "display-message".into(),
            "-p".into(),
            "-t".into(),
            pane.native.clone(),
            "#{pane_tty}".into(),
        ])?;
        if !crate::process::terminal::foreground(output.trim_end_matches('\n'), shell.pid()) {
            return Err(invalid());
        }
        let user_shell = restore.run(vec![
            "display-message".into(),
            "-p".into(),
            "-t".into(),
            pane.native.clone(),
            "#{default-shell}".into(),
        ])?;
        let user_shell = user_shell.trim_end_matches('\n');
        if !user_shell.starts_with('/') || user_shell.contains('\0') {
            return Err(invalid());
        }
        let checks = [
            format!("#{{==:#{{pane_id}},{}}}", pane.native),
            format!("#{{==:#{{pane_pid}},{}}}", shell.pid()),
            "#{==:#{pane_current_command},sh}".into(),
            format!(
                "#{{==:#{{pane_start_command}},{}}}",
                BOOTSTRAP_COMMAND.join(" ")
            ),
        ];
        restore.guard(
            Some(&pane.native),
            &checks,
            respawn_args(&pane.native, cwd, user_shell, argv),
        )?;
        Ok(())
    }

    pub fn workspace_restore_layout(
        &self,
        snapshot: &WorkspaceSnapshot,
        deadline: Instant,
        revival_panes: &[String],
    ) -> Result<LayoutRestore, TmuxError> {
        // Validate all saved layouts, including skipped ones, before even host reads.
        layout_creation(snapshot, &[]).map_err(|_| invalid())?;
        let socket = snapshot.server.socket.as_str();
        let absent = match std::fs::symlink_metadata(socket) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => true,
            Err(_) => return Err(invalid()),
            Ok(_) => false,
        };
        let mut restore = Restore {
            tmux: self,
            socket,
            deadline,
            fence: None,
            windows: HashMap::new(),
            panes: HashMap::new(),
            outcome: LayoutRestore::default(),
            revival_panes,
        };
        let (names, starting) = if absent {
            (Vec::new(), true)
        } else {
            match restore
                .run(vec![
                    "list-sessions".into(),
                    "-F".into(),
                    super::workspace::session_name_format(),
                ])
                .and_then(|output| super::workspace::session_names(&output))
            {
                Ok(names) => (names, false),
                Err(error)
                    if error.exited()
                        && !error.socket_permission_denied()
                        && std::fs::symlink_metadata(socket)
                            .is_ok_and(|entry| entry.file_type().is_socket())
                        && stale_socket(socket, deadline) =>
                {
                    (Vec::new(), true)
                }
                Err(error) => return Err(error),
            }
        };
        let plan = layout_creation(snapshot, &names).map_err(|_| invalid())?;
        // A skipped session's old directories are irrelevant to new creation.
        for session in &plan.sessions {
            if let WorkspaceLayoutSessionAction::Create { links, .. } = &session.action {
                for link in links.iter().filter(|link| link.create) {
                    if plan.windows[link.window]
                        .panes
                        .iter()
                        .any(|pane| !std::fs::metadata(&pane.cwd).is_ok_and(|entry| entry.is_dir()))
                    {
                        return Err(TmuxError::evidence(
                            "A recorded workspace directory is unavailable",
                        ));
                    }
                }
            }
        }
        if !starting {
            restore.fence = Some(restore.observe_server()?);
        }
        for planned in &plan.sessions {
            let mut record = RestoredSession {
                recorded: planned.session.id.clone(),
                name: planned.session.name.clone(),
                action: "skip_existing".into(),
                native: None,
                retained_bootstrap: None,
            };
            if let WorkspaceLayoutSessionAction::Create {
                seed_window,
                bootstrap_index,
                links,
            } = &planned.action
            {
                record.action = "partial".into();
                let result = restore.session(
                    &plan,
                    planned.session,
                    *seed_window,
                    *bootstrap_index,
                    links,
                    &mut record,
                );
                match result {
                    Ok(()) => record.action = "created".into(),
                    Err(error) => {
                        restore
                            .outcome
                            .failures
                            .push(format!("Session {:?}: {error}", record.name));
                        restore.outcome.sessions.push(record);
                        // An uncertain failure cannot grant authority for later effects.
                        for remaining in plan.sessions.iter().skip(restore.outcome.sessions.len()) {
                            restore.outcome.sessions.push(RestoredSession {
                                recorded: remaining.session.id.clone(),
                                name: remaining.session.name.clone(),
                                action: if matches!(
                                    remaining.action,
                                    WorkspaceLayoutSessionAction::SkipExisting
                                ) {
                                    "skip_existing".into()
                                } else {
                                    "not_attempted".into()
                                },
                                native: None,
                                retained_bootstrap: None,
                            });
                        }
                        break;
                    }
                }
            }
            restore.outcome.sessions.push(record);
        }
        restore.outcome.server = restore
            .fence
            .map(|fence| (fence.server, fence.native_start));
        Ok(restore.outcome)
    }
}

impl<R: CommandRunner> Restore<'_, R> {
    fn run(&self, args: Vec<String>) -> Result<String, TmuxError> {
        let mut selected = vec!["-S".into(), self.socket.into()];
        selected.extend(args);
        self.tmux.run(
            "tmux",
            selected,
            self.deadline,
            1024 * 1024,
            TmuxFailure::Command,
        )
    }

    fn observe_server(&self) -> Result<Fence, TmuxError> {
        let output = self.run(vec![
            "display-message".into(),
            "-p".into(),
            ["#{socket_path}", "#{pid}", "#{start_time}"].join(SEP),
        ])?;
        let fields: Vec<_> = output.trim_end_matches('\n').split(SEP).collect();
        if fields.len() != 3 || fields[0] != self.socket {
            return Err(invalid());
        }
        let pid = number(fields[1])?;
        let server = self.live(pid)?;
        Ok(Fence {
            server,
            native_start: number(fields[2])?,
        })
    }

    fn live(&self, pid: u64) -> Result<ProcessIncarnation, TmuxError> {
        // Require a live process, rather than a stopped shell, before effects.
        if let Some(observation) = self.tmux.runner.process_observation(pid, self.deadline) {
            return match observation {
                ProcessObservation::Live(process) if Instant::now() < self.deadline => Ok(process),
                _ => Err(invalid()),
            };
        }
        match observe_runtime_process(&self.tmux.runner, pid, self.deadline)
            .map_err(|_| invalid())?
        {
            ProcessObservation::Live(process) => Ok(process),
            _ => Err(invalid()),
        }
    }

    fn guard(
        &self,
        target: Option<&str>,
        extra: &[String],
        args: Vec<String>,
    ) -> Result<String, TmuxError> {
        let fence = self.fence.as_ref().ok_or_else(invalid)?;
        if self.live(fence.server.pid())? != fence.server {
            return Err(invalid());
        }
        let mut conditions = vec![
            format!("#{{==:#{{pid}},{}}}", fence.server.pid()),
            format!("#{{==:#{{start_time}},{}}}", fence.native_start),
        ];
        conditions.extend_from_slice(extra);
        let test = conditions
            .into_iter()
            .reduce(|left, right| format!("#{{&&:{left},{right}}}"))
            .ok_or_else(invalid)?;
        let mut guarded = vec!["if-shell".into(), "-F".into()];
        if let Some(target) = target {
            guarded.extend(["-t".into(), target.into()]);
        }
        guarded.extend([
            test,
            command(&args),
            format!("display-message -p {REFUSED}"),
        ]);
        let output = self.run(guarded)?;
        if output.lines().any(|line| line == REFUSED) {
            return Err(invalid());
        }
        Ok(output)
    }

    fn created(&mut self, output: &str) -> Result<Created, TmuxError> {
        let fields: Vec<_> = output.trim_end_matches('\n').split(SEP).collect();
        if fields.len() != 8
            || fields[0] != self.socket
            || !native_id(fields[3], b'$')
            || !native_id(fields[4], b'@')
            || !native_id(fields[5], b'%')
        {
            return Err(invalid());
        }
        let server = self.live(number(fields[1])?)?;
        let native_start = number(fields[2])?;
        match &self.fence {
            Some(fence) if fence.server != server || fence.native_start != native_start => {
                return Err(invalid());
            }
            Some(_) => (),
            None => {
                self.fence = Some(Fence {
                    server,
                    native_start,
                })
            }
        }
        Ok(Created {
            session: fields[3].into(),
            window: fields[4].into(),
            pane: fields[5].into(),
            process: self.live(number(fields[6])?)?,
            index: number(fields[7])?,
        })
    }

    fn session(
        &mut self,
        plan: &WorkspaceLayoutPlan<'_>,
        saved: &tmt_core::workspace::WorkspaceSession,
        seed: Option<usize>,
        bootstrap_index: Option<u64>,
        links: &[tmt_core::workspace::layout::WorkspaceLayoutLink],
        record: &mut RestoredSession,
    ) -> Result<(), TmuxError> {
        let first = &plan.windows[seed.unwrap_or(links[0].window)];
        let cwd = format_literal(&first.panes[0].cwd);
        let mut args = vec![
            "new-session".into(),
            "-d".into(),
            "-P".into(),
            "-F".into(),
            created_format(),
            "-s".into(),
            saved.name.clone(),
            "-n".into(),
            seed.map(|_| format_literal(&first.window.name))
                .unwrap_or_else(|| "tmt-restore-bootstrap".into()),
            "-x".into(),
            first.window.width.to_string(),
            "-y".into(),
            first.window.height.to_string(),
            "-c".into(),
            cwd,
        ];
        if seed.is_none() || self.revival_panes.contains(&first.panes[0].id) {
            // Ordinary panes inherit tmux defaults. Revival and bootstrap shells
            // have fixed startup checked before replacement or removal.
            args.extend(BOOTSTRAP_COMMAND.map(str::to_owned));
        }
        let starting = self.fence.is_none();
        let output = if starting {
            // Only new-session may start the missing server. No attach, replace or grouping.
            for arg in &mut args {
                *arg = argv_literal(arg);
            }
            self.run(args)?
        } else {
            self.guard(None, &[], args)?
        };
        let created = self.created(&output)?;
        record.native = Some(created.session.clone());
        if seed.is_none() {
            record.retained_bootstrap = Some(created.pane.clone());
        }
        if starting {
            // A new server gets a fresh UUID; historical snapshot identity is never used.
            // Refuse initialization if another session appeared in the meantime.
            self.guard(
                Some(&created.pane),
                &[
                    format!("#{{==:#{{session_id}},{}}}", created.session),
                    format!("#{{==:#{{S:#{{session_id}}}},{}}}", created.session),
                ],
                vec![
                    "set-option".into(),
                    "-s".into(),
                    "@tmt.server-id".into(),
                    uuid::Uuid::new_v4().to_string(),
                ],
            )?;
        }
        let index = seed
            .map(|window| {
                links
                    .iter()
                    .find(|link| link.window == window)
                    .expect("seed is a link")
                    .index
            })
            .or(bootstrap_index)
            .ok_or_else(invalid)?;
        if created.index != index {
            self.guard(
                Some(&created.pane),
                &[],
                vec![
                    "move-window".into(),
                    "-s".into(),
                    created.window.clone(),
                    "-t".into(),
                    format!("{}:{index}", created.session),
                ],
            )?;
        }
        if let Some(seed) = seed {
            self.window(seed, first, &created)?;
        }
        for link in links {
            if Some(link.window) == seed {
                continue;
            }
            if link.create {
                let saved_window = &plan.windows[link.window];
                let mut args = vec![
                    "new-window".into(),
                    "-d".into(),
                    "-P".into(),
                    "-F".into(),
                    created_format(),
                    "-t".into(),
                    format!("{}:{}", created.session, link.index),
                    "-n".into(),
                    format_literal(&saved_window.window.name),
                    "-c".into(),
                    format_literal(&saved_window.panes[0].cwd),
                ];
                if self.revival_panes.contains(&saved_window.panes[0].id) {
                    args.extend(BOOTSTRAP_COMMAND.map(str::to_owned));
                }
                let output = self.guard(Some(&created.pane), &[], args)?;
                let new = self.created(&output)?;
                if new.session != created.session {
                    return Err(invalid());
                }
                self.window(link.window, saved_window, &new)?;
            } else {
                let window = self.windows.get(&link.window).ok_or_else(invalid)?.clone();
                self.guard(
                    Some(&created.pane),
                    &[],
                    vec![
                        "link-window".into(),
                        "-d".into(),
                        "-s".into(),
                        window,
                        "-t".into(),
                        format!("{}:{}", created.session, link.index),
                    ],
                )?;
            }
        }
        if seed.is_none() {
            // Every required link succeeded before this exceptional removal is considered.
            self.remove_bootstrap(&created)?;
            record.retained_bootstrap = None;
        }
        let active = links.iter().find(|link| link.active).ok_or_else(invalid)?;
        let target = format!("{}:{}", created.session, active.index);
        self.guard(
            Some(&target),
            &[format!("#{{==:#{{session_id}},{}}}", created.session)],
            vec!["select-window".into(), "-t".into(), target.clone()],
        )?;
        Ok(())
    }

    fn window(
        &mut self,
        key: usize,
        saved: &WorkspaceLayoutWindow<'_>,
        created: &Created,
    ) -> Result<(), TmuxError> {
        self.windows.insert(key, created.window.clone());
        self.outcome.windows.push(RestoredWindow {
            recorded: saved.window.id.clone(),
            native: created.window.clone(),
        });
        self.remember_pane(&saved.panes[0].id, created);
        self.verify_cwd(&created.pane, &saved.panes[0].cwd)?;
        self.guard(
            Some(&created.pane),
            &[],
            vec![
                "set-option".into(),
                "-w".into(),
                "-t".into(),
                created.window.clone(),
                "pane-border-status".into(),
                saved.border_status.into(),
            ],
        )?;
        self.guard(
            Some(&created.pane),
            &[],
            vec![
                "set-option".into(),
                "-w".into(),
                "-t".into(),
                created.window.clone(),
                "window-size".into(),
                "manual".into(),
            ],
        )?;
        self.guard(
            Some(&created.pane),
            &[],
            vec![
                "resize-window".into(),
                "-t".into(),
                created.window.clone(),
                "-x".into(),
                saved.window.width.to_string(),
                "-y".into(),
                saved.window.height.to_string(),
            ],
        )?;
        for split in &saved.splits {
            let target = self
                .panes
                .get(&format!("%{}", split.target_leaf))
                .ok_or_else(invalid)?
                .clone();
            let old = format!("%{}", split.new_leaf);
            let pane = saved
                .panes
                .iter()
                .find(|pane| pane.id == old)
                .ok_or_else(invalid)?;
            let mut args = vec![
                "split-window".into(),
                "-d".into(),
                "-P".into(),
                "-F".into(),
                created_format(),
                match split.axis {
                    WorkspaceSplitAxis::Horizontal => "-h",
                    WorkspaceSplitAxis::Vertical => "-v",
                }
                .into(),
                "-l".into(),
                split.remaining_extent.to_string(),
                "-t".into(),
                target.clone(),
                "-c".into(),
                format_literal(&pane.cwd),
            ];
            if self.revival_panes.contains(&pane.id) {
                args.extend(BOOTSTRAP_COMMAND.map(str::to_owned));
            }
            let output = self.guard(
                Some(&target),
                &[format!("#{{==:#{{window_id}},{}}}", created.window)],
                args,
            )?;
            let new = self.created(&output)?;
            if new.window != created.window {
                return Err(invalid());
            }
            self.remember_pane(&old, &new);
            self.verify_cwd(&new.pane, &pane.cwd)?;
        }
        self.guard(
            Some(&created.pane),
            &[],
            vec![
                "select-layout".into(),
                "-t".into(),
                created.window.clone(),
                saved.window.layout.clone(),
            ],
        )?;
        let active = self
            .panes
            .get(&saved.window.active_pane)
            .ok_or_else(invalid)?
            .clone();
        self.guard(
            Some(&active),
            &[],
            vec!["select-pane".into(), "-t".into(), active.clone()],
        )?;
        if saved.zoomed {
            self.guard(
                Some(&active),
                &[],
                vec![
                    "resize-pane".into(),
                    "-Z".into(),
                    "-t".into(),
                    active.clone(),
                ],
            )?;
        }
        Ok(())
    }

    fn verify_cwd(&self, pane: &str, expected: &str) -> Result<(), TmuxError> {
        let output = self.guard(
            Some(pane),
            &[],
            vec![
                "display-message".into(),
                "-p".into(),
                "-t".into(),
                pane.into(),
                "#{pane_current_path}".into(),
            ],
        )?;
        if output.strip_suffix('\n') != Some(expected) {
            return Err(TmuxError::evidence(
                "A restored pane did not enter its recorded directory",
            ));
        }
        Ok(())
    }

    fn remember_pane(&mut self, old: &str, new: &Created) {
        self.panes.insert(old.into(), new.pane.clone());
        self.outcome.panes.push(RestoredPane {
            recorded: old.into(),
            native: new.pane.clone(),
            shell: self
                .revival_panes
                .iter()
                .any(|pane| pane == old)
                .then(|| new.process.clone()),
        });
    }

    fn remove_bootstrap(&self, created: &Created) -> Result<(), TmuxError> {
        if self.live(created.process.pid())? != created.process {
            return Err(invalid());
        }
        let output = self.guard(
            Some(&created.pane),
            &[],
            vec![
                "display-message".into(),
                "-p".into(),
                "-t".into(),
                created.pane.clone(),
                [
                    "#{pane_id}",
                    "#{pane_pid}",
                    "#{pane_current_command}",
                    "#{pane_tty}",
                    "#{pane_start_command}",
                ]
                .join(SEP),
            ],
        )?;
        let fields: Vec<_> = output.trim_end_matches('\n').split(SEP).collect();
        if fields.len() != 5
            || fields[0] != created.pane
            || number(fields[1])? != created.process.pid()
            || fields[2] != "sh"
            || fields[4] != BOOTSTRAP_COMMAND.join(" ")
            || !crate::process::terminal::foreground(fields[3], created.process.pid())
            || self.live(created.process.pid())? != created.process
        {
            return Err(invalid());
        }
        let checks = [
            format!("#{{==:#{{pane_id}},{}}}", created.pane),
            format!("#{{==:#{{pane_pid}},{}}}", created.process.pid()),
            format!("#{{==:#{{session_id}},{}}}", created.session),
            format!("#{{==:#{{window_id}},{}}}", created.window),
            "#{==:#{pane_current_command},sh}".into(),
            format!(
                "#{{==:#{{pane_start_command}},{}}}",
                BOOTSTRAP_COMMAND.join(" ")
            ),
            "#{==:#{window_panes},1}".into(),
            "#{==:#{window_linked_sessions},1}".into(),
            "#{>=:#{session_windows},2}".into(),
            "#{==:#{session_attached},0}".into(),
            "#{==:#{@tmt.agent},}".into(),
            "#{==:#{@tmt.workspace-command},}".into(),
        ];
        self.guard(
            Some(&created.pane),
            &checks,
            vec!["kill-pane".into(), "-t".into(), created.pane.clone()],
        )?;
        Ok(())
    }
}

/// Only a kernel refusal proves that a remaining socket has no listener.
/// A nonblocking probe never removes the inode or waits for a healthy peer.
fn stale_socket(path: &str, deadline: Instant) -> bool {
    use nix::{
        errno::Errno,
        fcntl::{FcntlArg, FdFlag, OFlag, fcntl},
        sys::socket::{AddressFamily, SockFlag, SockType, UnixAddr, connect, socket},
    };
    use std::os::fd::AsRawFd;
    if Instant::now() >= deadline {
        return false;
    }
    let Ok(address) = UnixAddr::new(path) else {
        return false;
    };
    let Ok(fd) = socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::empty(),
        None,
    ) else {
        return false;
    };
    // Darwin lacks atomic socket flags; use the same bounded native operation
    // on both platforms and close the owned descriptor on every return.
    if fcntl(&fd, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC)).is_err()
        || fcntl(&fd, FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).is_err()
        || Instant::now() >= deadline
    {
        return false;
    }
    matches!(connect(fd.as_raw_fd(), &address), Err(Errno::ECONNREFUSED))
        && Instant::now() < deadline
}

fn invalid() -> TmuxError {
    TmuxError::evidence("Workspace layout or live creation evidence was refused")
}
fn number(value: &str) -> Result<u64, TmuxError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    value.parse().map_err(|_| invalid())
}
fn native_id(value: &str, prefix: u8) -> bool {
    value.as_bytes().first() == Some(&prefix) && number(&value[1..]).is_ok()
}
/// tmux expands cwd formats but passes multi-argument startup argv literally.
fn respawn_args(pane: &str, cwd: &str, shell: &str, argv: &[String]) -> Vec<String> {
    let command = argv
        .iter()
        .map(|arg| posix_quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    vec![
        "respawn-pane".into(),
        "-k".into(),
        "-t".into(),
        pane.into(),
        "-c".into(),
        format_literal(cwd),
        shell.into(),
        "-c".into(),
        format!("{command}; exec {}", posix_quote(shell)),
    ]
}

fn posix_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn format_literal(value: &str) -> String {
    value.replace('#', "##")
}
fn argv_literal(value: &str) -> String {
    value
        .strip_suffix(';')
        .map(|value| format!("{value}\\;"))
        .unwrap_or_else(|| value.into())
}
/// The nested tmux command lexer decodes octal bytes once, without recursively
/// expanding dollars, quotes, newlines or command separators from saved input.
fn command(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            let mut quoted = String::from("\"");
            for byte in arg.bytes() {
                write!(quoted, "\\{byte:03o}").expect("String write");
            }
            quoted.push('"');
            quoted
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;
