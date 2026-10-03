//! Runtime-owned ancestry proof; tmux supplies only the verified pane PID.

use crate::process::{
    CommandRunner,
    ps::query_ps,
    runtime::{ProcessObservation, observe_runtime_process},
};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::Instant,
};
use tmt_core::endpoint::ProcessIncarnation;

pub(crate) fn observe_named_in_pane(
    runner: &impl CommandRunner,
    caller_pid: u64,
    pane_pid: u64,
    deadline: Instant,
    executables: &[&str],
) -> Option<ProcessIncarnation> {
    let snapshot = query_ps(
        runner,
        &["-A".into(), "-o".into(), "pid=,ppid=,comm=".into()],
        deadline,
        4 * 1024 * 1024,
    )
    .ok()?;
    let process = named_ancestor(
        std::str::from_utf8(&snapshot.stdout).ok()?,
        caller_pid,
        pane_pid,
        executables,
        false,
    )?;
    match observe_runtime_process(runner, process, deadline).ok()? {
        ProcessObservation::Live(incarnation) => Some(incarnation),
        _ => None,
    }
}

/// Recovery is based on a unique new runtime below the verified container,
/// never the shell PID or a provider name remembered in application state.
pub(crate) fn observe_replacement(
    runner: &impl CommandRunner,
    pane_pid: u64,
    deadline: Instant,
    executable: &str,
) -> Option<ProcessIncarnation> {
    let snapshot = query_ps(
        runner,
        &["-A".into(), "-o".into(), "pid=,ppid=,comm=".into()],
        deadline,
        4 * 1024 * 1024,
    )
    .ok()?;
    let text = std::str::from_utf8(&snapshot.stdout).ok()?;
    let mut candidates = Vec::new();
    for line in text.lines() {
        let (pid, tail) = line.trim().split_once(char::is_whitespace)?;
        let (_, command) = tail.trim_start().split_once(char::is_whitespace)?;
        if Path::new(command.trim()).file_name() != Some(std::ffi::OsStr::new(executable)) {
            continue;
        }
        let pid = pid.parse::<u64>().ok()?;
        if named_ancestor(text, pid, pane_pid, &[executable], true) == Some(pid) {
            candidates.push(pid);
        }
    }
    let [pid] = candidates.as_slice() else {
        return None;
    };
    match observe_runtime_process(runner, *pid, deadline).ok()? {
        ProcessObservation::Live(value) => Some(value),
        _ => None,
    }
}

#[cfg(test)]
fn ancestor(text: &str, caller: u64, pane: u64, executable: &str) -> Option<u64> {
    named_ancestor(text, caller, pane, &[executable], false)
}

fn named_ancestor(
    text: &str,
    caller: u64,
    pane: u64,
    executables: &[&str],
    include_caller: bool,
) -> Option<u64> {
    let mut rows = HashMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let (pid, tail) = line.trim().split_once(char::is_whitespace)?;
        let (parent, command) = tail.trim_start().split_once(char::is_whitespace)?;
        if rows
            .insert(
                pid.parse::<u64>().ok()?,
                (parent.parse::<u64>().ok()?, command.trim()),
            )
            .is_some()
        {
            return None;
        }
    }
    let mut pid = caller;
    let mut seen = HashSet::new();
    let mut runtime = None;
    for _ in 0..64 {
        if !seen.insert(pid) {
            return None;
        }
        let (parent, command) = rows.get(&pid)?;
        if (include_caller || pid != caller)
            && Path::new(command)
                .file_name()
                .is_some_and(|name| executables.iter().any(|executable| name == *executable))
            && runtime.is_none()
        {
            runtime = Some(pid);
        }
        if pid == pane {
            return runtime;
        }
        if *parent <= 1 {
            return None;
        }
        pid = *parent;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn declared_aliases_match_exactly_on_the_verified_ancestry() {
        let tree = "10 1 /bin/sh\n20 10 /opt/agent-alt\n30 20 /bin/tmt\n40 1 /bin/sh\n";
        assert_eq!(
            named_ancestor(tree, 30, 10, &["agent", "agent-alt"], false),
            Some(20)
        );
        assert_eq!(named_ancestor(tree, 30, 10, &["agent"], false), None);
        assert_eq!(
            named_ancestor(tree, 30, 40, &["agent", "agent-alt"], false),
            None
        );
    }

    #[test]
    fn claude_must_be_on_the_chain_in_the_verified_pane() {
        let tree = "10 1 /bin/zsh\n20 10 /opt/bin/claude\n30 20 /bin/sh\n40 30 /bin/tmt\n50 40 /bin/tmt\n60 1 /bin/zsh\n";
        assert_eq!(ancestor(tree, 50, 10, "claude"), Some(20));
        assert_eq!(ancestor(tree, 50, 20, "claude"), Some(20));
        assert_eq!(ancestor(tree, 50, 60, "claude"), None);
        assert_eq!(
            ancestor(
                &tree.replace("/opt/bin/claude", "/opt/bin/codex"),
                50,
                10,
                "claude"
            ),
            None
        );
        assert_eq!(
            ancestor(&format!("{tree}20 10 claude\n"), 50, 10, "claude"),
            None
        );
        assert_eq!(
            ancestor(&tree.replace("30 20", "30 40"), 50, 10, "claude"),
            None
        );
    }
}
