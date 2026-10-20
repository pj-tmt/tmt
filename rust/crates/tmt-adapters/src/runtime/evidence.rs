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

/// A direct conversation or an independent hook must belong to the pane's
/// main provider, never a nested invocation. Reuse the same native process
/// snapshot and incarnation boundary.
pub(crate) fn observe_main_in_pane(
    runner: &impl CommandRunner,
    caller: u64,
    pane: u64,
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
    let process = main_ancestor(text, caller, pane, executable)?;
    match observe_runtime_process(runner, process, deadline).ok()? {
        ProcessObservation::Live(value) => Some(value),
        _ => None,
    }
}

fn main_ancestor(text: &str, caller: u64, pane: u64, executable: &str) -> Option<u64> {
    let process = ancestor(text, caller, pane, executable)?;
    // Starting above this provider must find no second provider on the same
    // pane ancestry. The pane itself may be the provider after shell exec.
    if process != pane
        && crate::drivers::Registry::builtin()
            .iter()
            .filter(|driver| driver.runtime.is_some())
            .flat_map(|driver| driver.descriptor.executables.iter())
            .any(|name| ancestor(text, process, pane, name).is_some())
    {
        return None;
    }
    Some(process)
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
        if named_ancestor(text, pid, pane_pid, executable, true) == Some(pid) {
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

fn ancestor(text: &str, caller: u64, pane: u64, executable: &str) -> Option<u64> {
    named_ancestor(text, caller, pane, executable, false)
}

/// One `pid=,ppid=,comm=` row per PID; a malformed line or a repeated PID
/// makes the whole snapshot unusable.
fn rows(text: &str) -> Option<HashMap<u64, (u64, &str)>> {
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
    Some(rows)
}

fn named_ancestor(
    text: &str,
    caller: u64,
    pane: u64,
    executable: &str,
    include_caller: bool,
) -> Option<u64> {
    let rows = rows(text)?;
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
                .is_some_and(|name| name == executable)
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
    fn main_provider_refuses_nested_and_unrelated_native_callers() {
        let tree = "10 1 sh\n20 10 claude\n30 20 sh\n40 30 claude\n50 40 tmt\n";
        assert_eq!(main_ancestor(tree, 30, 10, "claude"), Some(20));
        assert_eq!(main_ancestor(tree, 30, 20, "claude"), Some(20));
        assert_eq!(main_ancestor(tree, 50, 10, "claude"), None);
        assert_eq!(
            main_ancestor(
                &tree.replace("40 30 claude", "40 30 codex"),
                50,
                10,
                "codex"
            ),
            None
        );
        assert_eq!(main_ancestor(tree, 50, 99, "claude"), None);
        assert_eq!(main_ancestor(tree, 30, 10, "codex"), None);
        assert_eq!(
            main_ancestor("10 20 sh\n20 10 claude\n30 20 tmt\n", 30, 99, "claude"),
            None
        );
    }
    #[test]
    fn only_the_terminal_provider_of_a_daemon_tree_is_the_main_provider() {
        // A terminal Claude that starts a daemon, a pty host and a worker: four
        // Claude processes on one pane ancestry, each with its own hook caller.
        let tree = "10 1 zsh\n20 10 /opt/bin/claude\n21 20 tmt\n30 20 claude\n31 30 tmt\n\
                    40 30 claude\n41 40 tmt\n50 40 claude\n60 50 sh\n61 60 tmt\n";
        assert_eq!(main_ancestor(tree, 21, 10, "claude"), Some(20));
        for caller in [31, 41, 61] {
            assert_eq!(main_ancestor(tree, caller, 10, "claude"), None, "{caller}");
        }
        // A provider that is the pane process, after shell exec, is the main one.
        assert_eq!(main_ancestor(tree, 21, 20, "claude"), Some(20));
        assert_eq!(
            main_ancestor("20 1 claude\n21 20 tmt\n", 21, 20, "claude"),
            Some(20)
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
