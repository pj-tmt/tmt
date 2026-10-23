//! Workspace recovery effects. Saved IDs never become live targets.

use super::{Tmux, TmuxError, TmuxFailure};
use crate::{
    process::{
        CommandRunner,
        runtime::{ProcessObservation, observe_runtime_process},
    },
    workspace::restore::{
        CwdFallback, LayoutRestore, RestoredPane, RestoredSession, RestoredWindow,
    },
};
use std::{
    collections::{BTreeSet, HashMap},
    fmt::Write,
    io,
    os::unix::fs::FileTypeExt,
    path::Path,
    time::Instant,
};
use tmt_core::{
    endpoint::ProcessIncarnation,
    workspace::{
        WorkspaceSnapshot,
        layout::{
            LiveSession, WorkspaceLayoutLink, WorkspaceLayoutPlan, WorkspaceLayoutSessionAction,
            WorkspaceLayoutWindow, WorkspaceSplitAxis, layout_creation, restored_marker,
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
    /// The directory each saved pane opens in when its recorded one is gone.
    cwd: HashMap<String, String>,
    outcome: LayoutRestore,
    revival_panes: &'a [String],
}

/// The server's sessions as a restore sees them.
#[derive(Default)]
struct Live {
    sessions: Vec<LiveSession>,
    /// A fresh default session by name: its native id and its one pane.
    fresh: HashMap<String, (String, String)>,
}

/// The session option a restore sets on a session it made, so a repeat recognises it.
const RESTORED_OPTION: &str = "@tmt.restored";

fn live_format() -> String {
    [
        "#{m/r:[[:cntrl:]],#{session_name}}",
        "#{session_name}",
        "#{session_id}",
        "#{window_index}",
        "#{pane_id}",
        "#{@tmt.agent}",
        "#{@tmt.workspace-command}",
        "#{@tmt.restored}",
    ]
    .join(SEP)
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
            .ok_or_else(pane_changed)?
            .cwd;
        let shell = pane.shell.as_ref().ok_or_else(pane_changed)?;
        let (server, native_start) = layout.server.as_ref().ok_or_else(server_changed)?;
        if argv.len() < 2 || argv.iter().any(|arg| arg.contains('\0')) {
            return Err(TmuxError::evidence(
                "The agent command to start is empty or holds a NUL byte, so no agent was started in the restored pane",
            ));
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
            cwd: HashMap::new(),
            outcome: LayoutRestore::default(),
            revival_panes: &[],
        };
        if restore.live(shell.pid())? != *shell {
            return Err(pane_changed());
        }
        let output = restore.run(vec![
            "display-message".into(),
            "-p".into(),
            "-t".into(),
            pane.native.clone(),
            "#{pane_tty}".into(),
        ])?;
        if !crate::process::terminal::foreground(output.trim_end_matches('\n'), shell.pid()) {
            return Err(pane_changed());
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
            return Err(unexpected("the default shell"));
        }
        let checks = [
            format!("#{{==:#{{pane_id}},{}}}", pane.native),
            format!("#{{==:#{{pane_pid}},{}}}", shell.pid()),
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
        layout_creation(snapshot, &[]).map_err(|_| layout_refused())?;
        let socket = snapshot.server.socket.as_str();
        let absent = match std::fs::symlink_metadata(socket) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => true,
            Err(error) => return Err(socket_unreadable(socket, &error)),
            Ok(_) => false,
        };
        let mut restore = Restore {
            tmux: self,
            socket,
            deadline,
            fence: None,
            windows: HashMap::new(),
            panes: HashMap::new(),
            cwd: HashMap::new(),
            outcome: LayoutRestore::default(),
            revival_panes,
        };
        let (live, starting) = if absent {
            (Live::default(), true)
        } else {
            match restore.inspect_live() {
                Ok(live) => (live, false),
                Err(error)
                    if error.exited()
                        && !error.socket_permission_denied()
                        && std::fs::symlink_metadata(socket)
                            .is_ok_and(|entry| entry.file_type().is_socket())
                        && stale_socket(socket, deadline) =>
                {
                    (Live::default(), true)
                }
                Err(error) => return Err(error),
            }
        };
        let plan = layout_creation(snapshot, &live.sessions).map_err(|_| layout_refused())?;
        // A missing directory never stops a restore: a plain pane opens in the nearest existing
        // parent and the report says so. Only windows this restore creates need one.
        for session in &plan.sessions {
            let links = match &session.action {
                WorkspaceLayoutSessionAction::Create { links, .. }
                | WorkspaceLayoutSessionAction::Merge { links, .. } => links,
                WorkspaceLayoutSessionAction::SkipExisting => continue,
            };
            for link in links.iter().filter(|link| link.create) {
                for pane in &plan.windows[link.window].panes {
                    restore.choose_cwd(&pane.id, &pane.cwd);
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
                restored_as: None,
                retained_bootstrap: None,
            };
            let marker = restored_marker(snapshot, planned.session);
            let merged = matches!(planned.action, WorkspaceLayoutSessionAction::Merge { .. });
            let result = match &planned.action {
                WorkspaceLayoutSessionAction::SkipExisting => Ok(()),
                WorkspaceLayoutSessionAction::Create {
                    name,
                    seed_window,
                    bootstrap_index,
                    links,
                } => {
                    record.action = "partial".into();
                    record.restored_as = (name != &planned.session.name).then(|| name.clone());
                    restore
                        .session(
                            &plan,
                            name,
                            *seed_window,
                            *bootstrap_index,
                            links,
                            &marker,
                            &mut record,
                        )
                        .map(|()| record.action = "created".into())
                }
                WorkspaceLayoutSessionAction::Merge { name, links } => {
                    record.action = "partial".into();
                    record.restored_as = Some(name.clone());
                    match live.fresh.get(name) {
                        Some(fresh) => restore
                            .merge(&plan, fresh, links, &marker, &mut record)
                            .map(|()| record.action = "merged".into()),
                        None => Err(server_changed()),
                    }
                }
            };
            if let Err(error) = result {
                restore.outcome.failures.push(format!(
                    "Session {:?}: {error}{}",
                    record.name,
                    partial_note(&record, merged)
                ));
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
                        restored_as: None,
                        retained_bootstrap: None,
                    });
                }
                break;
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
            return Err(server_changed());
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
                _ => Err(self.gone_or_late()),
            };
        }
        match observe_runtime_process(&self.tmux.runner, pid, self.deadline)
            .map_err(|_| self.gone_or_late())?
        {
            ProcessObservation::Live(process) => Ok(process),
            _ => Err(self.gone_or_late()),
        }
    }

    /// A process that cannot be observed live: out of time, or no longer there.
    fn gone_or_late(&self) -> TmuxError {
        if Instant::now() >= self.deadline {
            timed_out()
        } else {
            server_changed()
        }
    }

    fn guard(
        &self,
        target: Option<&str>,
        extra: &[String],
        args: Vec<String>,
    ) -> Result<String, TmuxError> {
        let fence = self.fence.as_ref().ok_or_else(server_changed)?;
        if self.live(fence.server.pid())? != fence.server {
            return Err(server_changed());
        }
        let mut conditions = vec![
            format!("#{{==:#{{pid}},{}}}", fence.server.pid()),
            format!("#{{==:#{{start_time}},{}}}", fence.native_start),
        ];
        conditions.extend_from_slice(extra);
        let test = conditions
            .into_iter()
            .reduce(|left, right| format!("#{{&&:{left},{right}}}"))
            .ok_or_else(server_changed)?;
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
            return Err(state_changed(target));
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
            return Err(unexpected("the window it just created"));
        }
        let server = self.live(number(fields[1])?)?;
        let native_start = number(fields[2])?;
        match &self.fence {
            Some(fence) if fence.server != server || fence.native_start != native_start => {
                return Err(server_changed());
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

    #[allow(clippy::too_many_arguments)]
    fn session(
        &mut self,
        plan: &WorkspaceLayoutPlan<'_>,
        name: &str,
        seed: Option<usize>,
        bootstrap_index: Option<u64>,
        links: &[WorkspaceLayoutLink],
        marker: &str,
        record: &mut RestoredSession,
    ) -> Result<(), TmuxError> {
        let first = &plan.windows[seed.unwrap_or(links[0].window)];
        let cwd = format_literal(self.cwd_of(&first.panes[0]));
        let mut args = vec![
            "new-session".into(),
            "-d".into(),
            "-P".into(),
            "-F".into(),
            created_format(),
            "-s".into(),
            name.to_owned(),
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
            .ok_or_else(layout_refused)?;
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
                    format_literal(self.cwd_of(&saved_window.panes[0])),
                ];
                if self.revival_panes.contains(&saved_window.panes[0].id) {
                    args.extend(BOOTSTRAP_COMMAND.map(str::to_owned));
                }
                let output = self.guard(Some(&created.pane), &[], args)?;
                let new = self.created(&output)?;
                if new.session != created.session {
                    return Err(server_changed());
                }
                self.window(link.window, saved_window, &new)?;
            } else {
                let window = self
                    .windows
                    .get(&link.window)
                    .ok_or_else(layout_refused)?
                    .clone();
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
        let active = links
            .iter()
            .find(|link| link.active)
            .ok_or_else(layout_refused)?;
        let target = format!("{}:{}", created.session, active.index);
        self.guard(
            Some(&target),
            &[format!("#{{==:#{{session_id}},{}}}", created.session)],
            vec!["select-window".into(), "-t".into(), target.clone()],
        )?;
        self.mark_restored(&created.pane, &created.session, marker)
    }

    /// Adds the saved windows to the server's fresh default session, after its own window. That
    /// window and its pane are never killed, moved or renumbered, and the client is not
    /// switched away from them.
    fn merge(
        &mut self,
        plan: &WorkspaceLayoutPlan<'_>,
        (session, lone): &(String, String),
        links: &[WorkspaceLayoutLink],
        marker: &str,
        record: &mut RestoredSession,
    ) -> Result<(), TmuxError> {
        record.native = Some(session.clone());
        let same = vec![format!("#{{==:#{{session_id}},{session}}}")];
        for link in links {
            if link.create {
                let saved_window = &plan.windows[link.window];
                let mut args = vec![
                    "new-window".into(),
                    "-d".into(),
                    "-P".into(),
                    "-F".into(),
                    created_format(),
                    "-t".into(),
                    format!("{session}:{}", link.index),
                    "-n".into(),
                    format_literal(&saved_window.window.name),
                    "-c".into(),
                    format_literal(self.cwd_of(&saved_window.panes[0])),
                ];
                if self.revival_panes.contains(&saved_window.panes[0].id) {
                    args.extend(BOOTSTRAP_COMMAND.map(str::to_owned));
                }
                let output = self.guard(Some(lone), &same, args)?;
                let new = self.created(&output)?;
                if &new.session != session {
                    return Err(server_changed());
                }
                self.window(link.window, saved_window, &new)?;
            } else {
                let window = self
                    .windows
                    .get(&link.window)
                    .ok_or_else(layout_refused)?
                    .clone();
                self.guard(
                    Some(lone),
                    &same,
                    vec![
                        "link-window".into(),
                        "-d".into(),
                        "-s".into(),
                        window,
                        "-t".into(),
                        format!("{session}:{}", link.index),
                    ],
                )?;
            }
        }
        self.mark_restored(lone, session, marker)
    }

    fn mark_restored(&self, pane: &str, session: &str, marker: &str) -> Result<(), TmuxError> {
        self.guard(
            Some(pane),
            &[format!("#{{==:#{{session_id}},{session}}}")],
            vec![
                "set-option".into(),
                "-t".into(),
                session.to_owned(),
                RESTORED_OPTION.into(),
                marker.to_owned(),
            ],
        )?;
        Ok(())
    }

    /// Every live session, with a fresh default one told apart: exactly one window of one pane
    /// with no agent or command bound to it and no restore marker.
    fn inspect_live(&self) -> Result<Live, TmuxError> {
        #[derive(Default)]
        struct Seen {
            id: String,
            windows: BTreeSet<u64>,
            panes: Vec<String>,
            bound: bool,
            marker: String,
        }
        let output = self.run(vec![
            "list-panes".into(),
            "-a".into(),
            "-F".into(),
            live_format(),
        ])?;
        let mut order: Vec<String> = Vec::new();
        let mut seen: HashMap<String, Seen> = HashMap::new();
        let mut rows = 0;
        let mut ignored = 0;
        for row in output.split_terminator('\n') {
            rows += 1;
            let fields: Vec<_> = row.split(SEP).collect();
            // A name holding a line break spreads over several rows. No saved session has such
            // a name, so its fragments only have to stay out of the way.
            if fields.len() != 8 {
                ignored += 1;
                continue;
            }
            // A name no saved session can share: it only has to stay out of the way.
            if fields[0] != "0"
                || fields[1].is_empty()
                || fields[1].contains('\u{fffd}')
                || fields[1].chars().any(char::is_control)
            {
                continue;
            }
            let entry = seen.entry(fields[1].to_owned()).or_insert_with(|| {
                order.push(fields[1].to_owned());
                Seen::default()
            });
            entry.id = fields[2].to_owned();
            entry.windows.insert(number(fields[3])?);
            entry.panes.push(fields[4].to_owned());
            entry.bound |= !fields[5].is_empty() || !fields[6].is_empty();
            entry.marker = fields[7].to_owned();
        }
        if order.len() > tmt_core::workspace::MAX_PANES || (rows > 0 && ignored == rows) {
            return Err(unexpected("the session list"));
        }
        let mut live = Live::default();
        for name in order {
            let found = &seen[&name];
            live.sessions.push(if !found.marker.is_empty() {
                LiveSession::Restored {
                    name,
                    marker: found.marker.clone(),
                }
            } else if found.windows.len() == 1
                && found.panes.len() == 1
                && !found.bound
                && found.marker.is_empty()
                && native_id(&found.id, b'$')
            {
                live.fresh
                    .insert(name.clone(), (found.id.clone(), found.panes[0].clone()));
                LiveSession::Fresh {
                    window_index: *found.windows.first().expect("one window"),
                    name,
                }
            } else {
                LiveSession::InUse(name)
            });
        }
        Ok(live)
    }

    /// Records where a pane opens: its recorded directory, or the nearest existing parent.
    fn choose_cwd(&mut self, pane: &str, recorded: &str) {
        if Path::new(recorded).is_dir() {
            return;
        }
        let used = nearest_directory(recorded);
        self.outcome.fallbacks.push(CwdFallback {
            pane: pane.to_owned(),
            wanted: recorded.to_owned(),
            used: used.clone(),
        });
        self.cwd.insert(pane.to_owned(), used);
    }

    fn cwd_of<'a>(&'a self, pane: &'a tmt_core::workspace::WorkspacePane) -> &'a str {
        self.cwd
            .get(&pane.id)
            .map_or(pane.cwd.as_str(), String::as_str)
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
        self.verify_cwd(&created.pane, self.cwd_of(&saved.panes[0]))?;
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
                .ok_or_else(layout_refused)?
                .clone();
            let old = format!("%{}", split.new_leaf);
            let pane = saved
                .panes
                .iter()
                .find(|pane| pane.id == old)
                .ok_or_else(layout_refused)?;
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
                format_literal(self.cwd_of(pane)),
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
                return Err(server_changed());
            }
            self.remember_pane(&old, &new);
            self.verify_cwd(&new.pane, self.cwd_of(pane))?;
        }
        // Observe the live server, not the client version. Only this invocation's
        // created window is queried, behind the same native-incarnation fence.
        let live_layout = self.guard(
            Some(&created.pane),
            &[format!("#{{==:#{{window_id}},{}}}", created.window)],
            vec![
                "display-message".into(),
                "-p".into(),
                "-t".into(),
                created.window.clone(),
                "#{window_layout}".into(),
            ],
        )?;
        let layout = saved
            .layout_for_server(live_layout.strip_suffix('\n').unwrap_or(&live_layout))
            .map_err(|_| unexpected("the restored window's layout"))?;
        self.guard(
            Some(&created.pane),
            &[],
            vec![
                "select-layout".into(),
                "-t".into(),
                created.window.clone(),
                layout,
            ],
        )?;
        let active = self
            .panes
            .get(&saved.window.active_pane)
            .ok_or_else(layout_refused)?
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
        let actual = output.strip_suffix('\n').unwrap_or(&output);
        if actual != expected && !same_directory(actual, expected) {
            return Err(TmuxError::refusal(format!(
                "A restored pane opened in {actual} instead of {expected}, so the restore stopped. Check that the directory is reachable and is not a link that moved, then restore again"
            )));
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
            return Err(bootstrap_changed());
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
            return Err(bootstrap_changed());
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
    super::socket_connect_error(std::path::Path::new(path), Some(deadline))
        == Some(nix::errno::Errno::ECONNREFUSED)
        && Instant::now() < deadline
}

/// Each refusal says what happened and what to do next; none is a bare "evidence" error.
fn layout_refused() -> TmuxError {
    TmuxError::evidence(
        "The saved workspace layout is not one tmt can recreate, so nothing was restored. Restore a different saved workspace, or save the current one again",
    )
}
fn socket_unreadable(socket: &str, error: &io::Error) -> TmuxError {
    TmuxError::refusal(format!(
        "The saved tmux socket {socket} cannot be read ({error}), so nothing was restored. Fix its permissions, or restore with --socket pointing at a tmux socket you can use"
    ))
}
fn server_changed() -> TmuxError {
    TmuxError::evidence(
        "The tmux server restarted or exited while the workspace was being restored, so the restore stopped. Run the restore again once the server is up",
    )
}
fn timed_out() -> TmuxError {
    TmuxError::evidence(
        "The restore ran past its time limit and stopped part way. Run the restore again; it finishes only what is missing",
    )
}
fn state_changed(target: Option<&str>) -> TmuxError {
    TmuxError::refusal(format!(
        "tmux no longer matched {} while restoring, so the restore stopped before touching it. Something else changed that session, window or pane; leave it alone and restore again",
        target.unwrap_or("the server"),
    ))
}
fn unexpected(what: &str) -> TmuxError {
    TmuxError::refusal(format!(
        "tmux answered with something unexpected about {what}, so the restore stopped. A different tmux version may be running; restore again, and report it if it repeats"
    ))
}
fn pane_changed() -> TmuxError {
    TmuxError::evidence(
        "The restored pane changed or its shell is no longer idle, so no agent was started in it. Start the agent there yourself, or restore again",
    )
}
fn bootstrap_changed() -> TmuxError {
    TmuxError::evidence(
        "The placeholder shell tmt opened to hold a restored session changed before it was removed, so it was left in place. Close that pane yourself",
    )
}
/// What a failed session leaves behind, and how to clear it before restoring again.
fn partial_note(record: &RestoredSession, merged: bool) -> String {
    let Some(native) = record.native.as_deref() else {
        return String::new();
    };
    let name = record.restored_as.as_deref().unwrap_or(&record.name);
    if merged {
        format!(
            " Windows already added to your session {name} ({native}) stay; close them yourself before restoring again"
        )
    } else {
        format!(
            " Session {name} ({native}) was left part restored; remove it with tmux kill-session -t {name} before restoring again"
        )
    }
}
/// The deepest existing directory at or above `recorded`; home, then `/`, at worst.
fn nearest_directory(recorded: &str) -> String {
    let mut path = Path::new(recorded);
    while let Some(parent) = path.parent() {
        if parent.is_dir() && parent != Path::new("") {
            return parent.to_string_lossy().into_owned();
        }
        path = parent;
    }
    std::env::var("HOME")
        .ok()
        .filter(|home| Path::new(home).is_dir())
        .unwrap_or_else(|| "/".into())
}
fn same_directory(left: &str, right: &str) -> bool {
    matches!(
        (std::fs::canonicalize(left), std::fs::canonicalize(right)),
        (Ok(left), Ok(right)) if left == right
    )
}
fn number(value: &str) -> Result<u64, TmuxError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(unexpected("a number"));
    }
    value.parse().map_err(|_| unexpected("a number"))
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
