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
use tmt_core::{binding::session::RuntimeLiveness, endpoint::ProcessIncarnation};

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

/// Unlike replacement discovery, this selects only the captured incarnation.
/// It must still be this driver's main process under the verified pane, and
/// must not be the pane itself: stopping that process would close the pane.
pub(crate) fn movable_process(
    runner: &impl CommandRunner,
    recorded: &ProcessIncarnation,
    pane: u64,
    deadline: Instant,
    executable: &str,
) -> Option<ProcessIncarnation> {
    if recorded.pid() == pane {
        return None;
    }
    let snapshot = query_ps(
        runner,
        &["-A".into(), "-o".into(), "pid=,ppid=,comm=".into()],
        deadline,
        4 * 1024 * 1024,
    )
    .ok()?;
    let text = std::str::from_utf8(&snapshot.stdout).ok()?;
    let parsed = rows(text)?;
    if Path::new(parsed.get(&recorded.pid())?.1).file_name()
        != Some(std::ffi::OsStr::new(executable))
    {
        return None;
    }
    (classify_recorded(runner, text, recorded, pane, deadline) == RecordedRuntime::Main)
        .then(|| recorded.clone())
}

/// Where a recorded runtime sits relative to its verified pane (#3919).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecordedRuntime {
    /// No other built-in provider sits between it and the pane.
    Main,
    /// Below another provider in the pane, or on a complete chain outside it.
    ProvenNonMain,
    /// The snapshot or the process observation proves neither.
    Unknown,
}

/// Classifies `recorded` against the verified `pane` on `snapshot`, a
/// `pid=,ppid=,comm=` listing. `recorded` must have been read from the binding
/// before `snapshot` was captured: an incarnation that still matches after it
/// was then alive throughout, so the snapshot row for its PID is its own, not
/// a reuser's. A reparenting during the read is not covered.
pub(crate) fn classify_recorded(
    runner: &impl CommandRunner,
    snapshot: &str,
    recorded: &ProcessIncarnation,
    pane: u64,
    deadline: Instant,
) -> RecordedRuntime {
    let placement = placement(snapshot, recorded.pid(), pane);
    if placement == RecordedRuntime::Unknown {
        return placement;
    }
    match observe_runtime_process(runner, recorded.pid(), deadline) {
        Ok(observation) if observation.matches(recorded) == RuntimeLiveness::Alive => placement,
        _ => RecordedRuntime::Unknown,
    }
}

/// Only a complete chain decides: a provider seen below a missing link, a
/// loop or the step bound is still `Unknown`. The walk ends at the pane or at
/// the PID 1 row, and the pane is checked first.
fn placement(text: &str, recorded: u64, pane: u64) -> RecordedRuntime {
    let Some(rows) = rows(text) else {
        return RecordedRuntime::Unknown;
    };
    if !rows.contains_key(&recorded) {
        return RecordedRuntime::Unknown;
    }
    // The pane process itself may be the provider after a shell exec.
    if recorded == pane {
        return RecordedRuntime::Main;
    }
    let mut nested = false;
    let mut pid = recorded;
    let mut seen = HashSet::new();
    for _ in 0..64 {
        if !seen.insert(pid) {
            return RecordedRuntime::Unknown;
        }
        let Some((parent, command)) = rows.get(&pid) else {
            return RecordedRuntime::Unknown;
        };
        if pid == pane {
            return if nested || builtin_provider(command) {
                RecordedRuntime::ProvenNonMain
            } else {
                RecordedRuntime::Main
            };
        }
        nested |= pid != recorded && builtin_provider(command);
        if pid == 1 {
            return RecordedRuntime::ProvenNonMain;
        }
        if *parent == 0 {
            return RecordedRuntime::Unknown;
        }
        pid = *parent;
    }
    RecordedRuntime::Unknown
}

fn builtin_provider(command: &str) -> bool {
    let name = Path::new(command).file_name();
    crate::drivers::Registry::builtin()
        .iter()
        .filter(|driver| driver.runtime.is_some())
        .flat_map(|driver| driver.descriptor.executables.iter())
        .any(|executable| name == Some(std::ffi::OsStr::new(executable)))
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

    use RecordedRuntime::{Main, ProvenNonMain, Unknown};

    const DAEMON: &str = "1 0 launchd\n10 1 zsh\n20 10 claude\n30 20 claude\n40 30 claude\n";

    #[test]
    fn a_recorded_runtime_is_main_only_with_no_provider_up_to_its_pane() {
        assert_eq!(placement(DAEMON, 20, 10), Main);
        assert_eq!(
            placement("10 1 zsh\n15 10 tmt\n20 15 claude\n", 20, 10),
            Main
        );
        // After a shell exec the pane process is the provider, whatever runs
        // above the pane.
        assert_eq!(placement(DAEMON, 20, 20), Main);
        assert_eq!(placement("5 1 claude\n20 5 claude\n", 20, 20), Main);
    }

    #[test]
    fn a_provider_up_to_the_pane_proves_the_recorded_runtime_nested() {
        assert_eq!(placement(DAEMON, 40, 10), ProvenNonMain);
        assert_eq!(placement(DAEMON, 30, 10), ProvenNonMain);
        // The pane process itself is a provider above the recorded one.
        assert_eq!(
            placement("20 1 claude\n30 20 sh\n40 30 claude\n", 40, 20),
            ProvenNonMain
        );
        assert_eq!(
            placement(
                &DAEMON.replace("20 10 claude", "20 10 /opt/bin/codex"),
                40,
                10
            ),
            ProvenNonMain
        );
        assert_eq!(
            placement("10 1 zsh\n20 10 claude\n30 20 sh\n40 30 codex\n", 40, 10),
            ProvenNonMain
        );
    }

    #[test]
    fn only_the_pid_1_row_ends_a_chain_outside_the_pane() {
        let outside = "1 0 launchd\n10 1 zsh\n20 10 claude\n30 1 claude\n40 30 claude\n";
        assert_eq!(placement(outside, 40, 10), ProvenNonMain);
        assert_eq!(
            placement(&outside.replace("1 0 launchd\n", ""), 40, 10),
            Unknown
        );
        assert_eq!(
            placement("2 0 kthreadd\n10 2 zsh\n40 2 claude\n", 40, 10),
            Unknown
        );
        // The pane is checked before the root.
        assert_eq!(placement("1 0 launchd\n40 1 claude\n", 40, 1), Main);
        assert_eq!(
            placement("1 0 launchd\n30 1 claude\n40 30 claude\n", 40, 1),
            ProvenNonMain
        );
    }

    #[test]
    fn an_incomplete_chain_is_unknown_even_after_a_provider() {
        assert_eq!(placement(DAEMON, 99, 10), Unknown);
        assert_eq!(
            placement(&DAEMON.replace("30 20 claude\n", ""), 40, 10),
            Unknown
        );
        // Provider 30 is seen before the missing link 20.
        assert_eq!(
            placement(&DAEMON.replace("20 10 claude\n", ""), 40, 10),
            Unknown
        );
        assert_eq!(placement("30 40 claude\n40 30 claude\n", 40, 10), Unknown);
        assert_eq!(placement("30 40 sh\n40 30 claude\n", 40, 10), Unknown);
        for malformed in ["x 1 sh\n", "50 y sh\n", "50\n", "40 1 claude\n"] {
            assert_eq!(
                placement(&format!("{DAEMON}{malformed}"), 40, 10),
                Unknown,
                "{malformed}"
            );
        }
    }

    #[test]
    fn the_walk_visits_at_most_64_processes_counting_the_recorded_one() {
        // Pane 1000 under PID 1; the recorded runtime is `length` processes
        // down from it, counting both.
        let chain = |length: u64| {
            let mut text = String::from("1 0 launchd\n1000 1 zsh\n");
            for pid in 1001..1000 + length {
                text.push_str(&format!("{pid} {} sh\n", pid - 1));
            }
            text
        };
        assert_eq!(placement(&chain(64), 1063, 1000), Main);
        assert_eq!(placement(&chain(65), 1064, 1000), Unknown);
        assert_eq!(placement(&chain(200), 1199, 5), Unknown);
    }

    const LIVE: &str = "Sun Sep 27 10:00:00 2026 S+\n";

    fn recorded(pid: u64) -> ProcessIncarnation {
        ProcessIncarnation::new(pid, "ps-v1:Sun Sep 27 10:00:00 2026").unwrap()
    }

    fn soon() -> Instant {
        Instant::now() + std::time::Duration::from_secs(5)
    }

    #[test]
    fn a_classified_runtime_is_still_the_recorded_incarnation_after_the_snapshot() {
        use crate::scripted_runner::ScriptedRunner;
        for (pid, placed) in [(20, Main), (40, ProvenNonMain)] {
            let runner = ScriptedRunner::new([Ok(LIVE)]);
            assert_eq!(
                classify_recorded(&runner, DAEMON, &recorded(pid), 10, soon()),
                placed
            );
            let calls = runner.calls.borrow();
            assert_eq!(calls.len(), 1);
            assert!(calls[0].args.contains(&pid.to_string()));
        }
        // A reused PID (another start), a stopped or zombie process, and an
        // unreadable answer prove nothing about the recorded incarnation.
        for answer in [
            "Sun Sep 27 10:00:01 2026 S+\n",
            "Sun Sep 27 10:00:00 2026 T+\n",
            "Sun Sep 27 10:00:00 2026 Z\n",
            "",
            "busy\n",
        ] {
            let runner = ScriptedRunner::new([Ok(answer)]);
            assert_eq!(
                classify_recorded(&runner, DAEMON, &recorded(40), 10, soon()),
                Unknown,
                "{answer:?}"
            );
        }
    }

    #[test]
    fn a_failed_observation_or_placement_classifies_nothing() {
        use crate::scripted_runner::{ScriptedRunner, failure};
        let runner = ScriptedRunner::new([Err(failure(true))]);
        assert_eq!(
            classify_recorded(&runner, DAEMON, &recorded(40), 10, soon()),
            Unknown
        );
        // A plain probe failure falls back to the observer's existence check;
        // this PID can't exist, so no live process is checked.
        let tree = "1 0 launchd\n10 1 zsh\n20 10 claude\n2147483647 20 claude\n";
        let runner = ScriptedRunner::new([Err(failure(false))]);
        assert_eq!(
            classify_recorded(&runner, tree, &recorded(2147483647), 10, soon()),
            Unknown
        );
        // An unknown placement asks nothing.
        let runner = ScriptedRunner::new([]);
        assert_eq!(
            classify_recorded(&runner, DAEMON, &recorded(99), 10, soon()),
            Unknown
        );
        assert!(runner.calls.borrow().is_empty());
    }

    #[test]
    fn a_move_proves_the_exact_main_driver_and_never_the_pane_or_a_nested_agent() {
        use crate::scripted_runner::ScriptedRunner;
        let runner = ScriptedRunner::new([Ok(DAEMON), Ok(LIVE)]);
        assert_eq!(
            movable_process(&runner, &recorded(20), 10, soon(), "claude"),
            Some(recorded(20))
        );
        let runner = ScriptedRunner::new([Ok(DAEMON), Ok(LIVE)]);
        assert_eq!(
            movable_process(&runner, &recorded(40), 10, soon(), "claude"),
            None
        );
        let runner = ScriptedRunner::new([]);
        assert_eq!(
            movable_process(&runner, &recorded(20), 20, soon(), "claude"),
            None
        );
        assert!(runner.calls.borrow().is_empty());
        let runner = ScriptedRunner::new([Ok(DAEMON)]);
        assert_eq!(
            movable_process(&runner, &recorded(20), 10, soon(), "codex"),
            None
        );
        assert_eq!(runner.calls.borrow().len(), 1);
        for answer in [
            "Sun Sep 27 10:00:01 2026 S+\n",
            "Sun Sep 27 10:00:00 2026 T+\n",
            "busy\n",
        ] {
            let runner = ScriptedRunner::new([Ok(DAEMON), Ok(answer)]);
            assert_eq!(
                movable_process(&runner, &recorded(20), 10, soon(), "claude"),
                None
            );
        }
    }
}
