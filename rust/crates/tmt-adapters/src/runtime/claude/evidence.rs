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
use tmt_core::binding::session::RuntimeIncarnation;

pub fn observe_in_pane(
    runner: &impl CommandRunner,
    caller_pid: u64,
    pane_pid: u64,
    deadline: Instant,
) -> Option<RuntimeIncarnation> {
    let snapshot = query_ps(
        runner,
        &["-A".into(), "-o".into(), "pid=,ppid=,comm=".into()],
        deadline,
        4 * 1024 * 1024,
    )
    .ok()?;
    let process = ancestor(
        std::str::from_utf8(&snapshot.stdout).ok()?,
        caller_pid,
        pane_pid,
    )?;
    match observe_runtime_process(runner, process, deadline).ok()? {
        ProcessObservation::Live(incarnation) => Some(incarnation),
        _ => None,
    }
}

fn ancestor(text: &str, caller: u64, pane: u64) -> Option<u64> {
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
    let mut claude = None;
    for _ in 0..64 {
        if !seen.insert(pid) {
            return None;
        }
        let (parent, command) = rows.get(&pid)?;
        if pid != caller
            && Path::new(command)
                .file_name()
                .is_some_and(|name| name == "claude")
            && claude.is_none()
        {
            claude = Some(pid);
        }
        if pid == pane {
            return claude;
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
    fn claude_must_be_on_the_chain_in_the_verified_pane() {
        let tree = "10 1 /bin/zsh\n20 10 /opt/bin/claude\n30 20 /bin/sh\n40 30 /bin/tmt\n50 40 /bin/tmt\n60 1 /bin/zsh\n";
        assert_eq!(ancestor(tree, 50, 10), Some(20));
        assert_eq!(ancestor(tree, 50, 20), Some(20));
        assert_eq!(ancestor(tree, 50, 60), None);
        assert_eq!(
            ancestor(&tree.replace("/opt/bin/claude", "/opt/bin/codex"), 50, 10),
            None
        );
        assert_eq!(ancestor(&format!("{tree}20 10 claude\n"), 50, 10), None);
        assert_eq!(ancestor(&tree.replace("30 20", "30 40"), 50, 10), None);
    }
}
