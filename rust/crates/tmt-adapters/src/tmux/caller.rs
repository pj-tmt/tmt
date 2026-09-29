use super::evidence::wire_integer as number;
use super::valid_pane_id;
use super::{CommandRunner, OPERATION_TIMEOUT, Tmux, TmuxError, TmuxFailure};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    time::Instant,
};
use tmt_core::{
    host::{HostKind, ServerSelector},
    limits::MAX_JS_SAFE_INTEGER,
};

const SEPARATOR: &str = "__TMT_CALLER_PANE_4f1c__";
const MAX_DEPTH: usize = 64;
const DISCOVERY_MAX_OUTPUT: usize = 64 * 1024;
const VERIFY_MAX_OUTPUT: usize = 4096;

/// Invocation-owned observations; tests never mutate process-global variables.
pub struct CallerEnvironment {
    pub tmux: Option<OsString>,
    pub pane: Option<OsString>,
    pub process_id: u64,
}

impl CallerEnvironment {
    /// A scheduling preference only, not verified caller or routing authority.
    pub fn selected_server(&self) -> Option<ServerSelector<'_>> {
        context(self.tmux.as_ref()?.to_str()?).map(|context| ServerSelector {
            host: HostKind::Tmux,
            socket: context.socket,
        })
    }

    /// The selected tmux server, with malformed supplied evidence rejected
    /// instead of treated as permission to use an ambient default server.
    pub fn selected_server_socket(&self) -> Result<Option<&str>, TmuxError> {
        let Some(value) = &self.tmux else {
            return Ok(None);
        };
        let text = value.to_str().ok_or_else(unavailable)?;
        if text.is_empty() {
            return Ok(None);
        }
        context(text)
            .map(|context| Some(context.socket))
            .ok_or_else(unavailable)
    }

    /// The session ID (`$N`) that tmux wrote into `TMUX` for this process.
    /// In a display-popup this is the popup client's session, while
    /// `TMUX_PANE` names the popup's own pane, which belongs to no session.
    pub fn selected_session(&self) -> Option<String> {
        context(self.tmux.as_ref()?.to_str()?).map(|context| format!("${}", context.session))
    }

    pub fn current() -> Self {
        Self {
            tmux: std::env::var_os("TMUX"),
            pane: std::env::var_os("TMUX_PANE"),
            process_id: u64::from(std::process::id()),
        }
    }
}

struct Context<'a> {
    socket: &'a str,
    server_pid: u64,
    session: &'a str,
}

fn context(text: &str) -> Option<Context<'_>> {
    let mut parts = text.rsplitn(3, ',');
    let session = parts.next()?;
    let pid = parts.next()?;
    let socket = parts.next()?;
    // Session is syntactic evidence only, not a numeric OS process target.
    if socket.is_empty() || session.is_empty() || !session.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    Some(Context {
        socket,
        server_pid: number(pid).filter(|pid| *pid > 0)?,
        session,
    })
}

pub(super) fn resolve<R: CommandRunner>(
    tmux: &Tmux<R>,
    environment: &CallerEnvironment,
) -> Result<String, TmuxError> {
    // Invalid supplied bytes are explicit invalid evidence, not absence that
    // permits discovery to bind an unrelated ambient pane.
    let tmux_text = match &environment.tmux {
        Some(value) => value.to_str().ok_or_else(unavailable)?,
        None => "",
    };
    let pane = match &environment.pane {
        Some(value) => value.to_str().ok_or_else(unavailable)?,
        None => "",
    };
    if !pane.is_empty() && !valid_pane_id(pane) {
        return Err(unavailable());
    }
    let context = if tmux_text.is_empty() {
        None
    } else {
        Some(context(tmux_text).ok_or_else(unavailable)?)
    };
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    if !pane.is_empty()
        && let Some(context) = &context
    {
        let format = ["#{pane_id}", "#{socket_path}", "#{pid}"].join(SEPARATOR);
        let output = tmux.run(
            "tmux",
            vec![
                "display-message".into(),
                "-p".into(),
                "-t".into(),
                pane.into(),
                format,
            ],
            deadline,
            VERIFY_MAX_OUTPUT,
            TmuxFailure::Command,
        )?;
        let text = output.trim();
        let fields: Vec<_> = text.split(SEPARATOR).collect();
        return (!text.contains('\n')
            && fields.len() == 3
            && fields[0] == pane
            && fields[1] == context.socket
            && fields[2] == context.server_pid.to_string())
        .then(|| pane.into())
        .ok_or_else(unavailable);
    }
    let ancestry = ancestry(tmux, environment.process_id, deadline)?;
    let mut args = context
        .as_ref()
        .map_or_else(Vec::new, |context| vec!["-S".into(), context.socket.into()]);
    let format = ["#{pane_id}", "#{pane_pid}", "#{socket_path}", "#{pid}"].join(SEPARATOR);
    args.extend(["list-panes".into(), "-a".into(), "-F".into(), format]);
    let output = tmux.run(
        "tmux",
        args,
        deadline,
        DISCOVERY_MAX_OUTPUT,
        TmuxFailure::Command,
    )?;
    if Instant::now() >= deadline {
        return Err(unavailable());
    }
    let mut unique: HashMap<&str, Vec<&str>> = HashMap::new();
    for line in output.trim().lines().filter(|line| !line.is_empty()) {
        let fields: Vec<_> = line.split(SEPARATOR).collect();
        if fields.len() != 4
            || !valid_pane_id(fields[0])
            || number(fields[1]).filter(|pid| *pid > 0).is_none()
            || fields[2].is_empty()
            || number(fields[3]).filter(|pid| *pid > 0).is_none()
        {
            return Err(unavailable());
        }
        if let Some(previous) = unique.insert(fields[0], fields.clone())
            && previous != fields
        {
            return Err(unavailable());
        }
    }
    let mut candidates = unique.values().filter(|fields| {
        number(fields[1]).is_some_and(|pid| ancestry.contains(&pid))
            && (pane.is_empty() || pane == fields[0])
            && context.as_ref().is_none_or(|context| {
                fields[2] == context.socket && fields[3] == context.server_pid.to_string()
            })
    });
    let candidate = candidates.next().ok_or_else(unavailable)?;
    if candidates.next().is_some() {
        return Err(unavailable());
    }
    Ok(candidate[0].into())
}

fn unavailable() -> TmuxError {
    TmuxError::evidence("Caller pane evidence is unavailable")
}

fn ancestry<R: CommandRunner>(
    tmux: &Tmux<R>,
    first: u64,
    deadline: Instant,
) -> Result<HashSet<u64>, TmuxError> {
    let mut ancestry = HashSet::new();
    let mut pid = first;
    for _ in 0..MAX_DEPTH {
        if pid == 0 || pid > MAX_JS_SAFE_INTEGER || !ancestry.insert(pid) {
            return Err(unavailable());
        }
        let output = tmux.run(
            "ps",
            vec![
                "-o".into(),
                "pid=,ppid=".into(),
                "-p".into(),
                pid.to_string(),
            ],
            deadline,
            DISCOVERY_MAX_OUTPUT,
            TmuxFailure::Command,
        )?;
        let lines: Vec<_> = output.trim().lines().collect();
        if lines.len() != 1 {
            return Err(unavailable());
        }
        let fields: Vec<_> = lines[0].split_whitespace().collect();
        if fields.len() != 2 || number(fields[0]) != Some(pid) {
            return Err(unavailable());
        }
        let parent = number(fields[1]).ok_or_else(unavailable)?;
        if parent == 0 {
            return Ok(ancestry);
        }
        pid = parent;
    }
    Err(unavailable())
}
