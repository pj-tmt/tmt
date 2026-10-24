//! Nested callers (#3583): tmux and one approved external host, ordered on
//! one pinned ancestry, for hooks and for ordinary commands (#3920). The
//! processes and tmux are scripted; no tmux server runs and no real pane is
//! read. Every refusal or fallback sits beside the same world with only that
//! fault removed, which admits the other host.

use super::*;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

const CALLER: u64 = 10;
const SEPARATOR: &str = "__TMT_CALLER_PANE_4f1c__";

/// Scripted processes, read through the UTC `ps` protocol, and a fake tmux
/// for pane `%1` of `/tmp/nest.sock`, server pid 1. Driver calls, and `ps`
/// for a pid the world does not script (the fake host's server), run as
/// production does.
#[derive(Clone, Default)]
struct World(Rc<RefCell<State>>);

#[derive(Default)]
struct State {
    /// pid: (parent, start in whole seconds), as the pinned read sees it.
    processes: BTreeMap<u64, (u64, u64)>,
    /// A raw `ps` row that replaces the pinned read of a pid.
    rows: BTreeMap<u64, String>,
    /// What the re-check sees instead: a new start, or `None` (ended).
    later: BTreeMap<u64, Option<u64>>,
    /// A pid and its new parent, from the end of the first chain read on.
    reparent: Option<(u64, u64)>,
    tmux: Option<Answer>,
    /// Every `ps` and `tmux` call, and the native link reads asked for.
    calls: Vec<String>,
    native_links: usize,
    /// A read whose child could not be cleaned up: a pinned `ps` read
    /// (`"link"` or `"observe"`), the PID-only `ps` read (`"pid"`), or the
    /// driver's `caller` call (`"caller"`), which runs first.
    unclean: Option<&'static str>,
}

#[derive(Clone)]
enum Answer {
    Pane(u64),
    Text(String),
    Fails(CommandFailure),
    /// The pane, after the 1 s proof budget has run out.
    Late(u64),
}

fn fields(id: &str, socket: &str, server: &str, pane_pid: &str, dead: &str) -> String {
    [id, socket, server, pane_pid, dead].join(SEPARATOR) + "\n"
}

fn lstart(seconds: u64) -> String {
    let (hours, minutes) = (seconds / 3600, seconds % 3600 / 60);
    format!("Thu Jan 1 {hours:02}:{minutes:02}:{:02} 1970", seconds % 60)
}

fn exit(code: u32) -> CommandError {
    CommandError::new(CommandFailure::Exit {
        code: Some(code),
        signal: None,
    })
}

fn output(text: String) -> Result<CommandOutput, CommandError> {
    Ok(CommandOutput {
        stdout: text.into_bytes(),
        stderr: Vec::new(),
    })
}

impl World {
    /// `pids` nearest first, each the parent of the one before it, the last
    /// one's parent 0; every parent started before its child.
    fn chain(pids: &[u64]) -> Self {
        let world = Self::default();
        for (index, pid) in pids.iter().enumerate() {
            let parent = pids.get(index + 1).copied().unwrap_or(0);
            let start = 2000 - 10 * index as u64;
            world.edit(|state| {
                state.processes.insert(*pid, (parent, start));
            });
        }
        world
    }

    fn edit(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.0.borrow_mut());
    }

    fn tmux(self, answer: Answer) -> Self {
        self.edit(|state| state.tmux = Some(answer));
        self
    }

    fn row(self, pid: u64, text: &str) -> Self {
        self.edit(|state| {
            state.rows.insert(pid, text.into());
        });
        self
    }

    fn gone(self, pid: u64) -> Self {
        self.edit(|state| {
            state.processes.remove(&pid);
        });
        self
    }

    fn started(self, pid: u64, seconds: u64) -> Self {
        self.edit(|state| {
            state.processes.get_mut(&pid).unwrap().1 = seconds;
        });
        self
    }

    fn later(self, pid: u64, start: Option<u64>) -> Self {
        self.edit(|state| {
            state.later.insert(pid, start);
        });
        self
    }

    fn reparent(self, pid: u64, parent: u64) -> Self {
        self.edit(|state| state.reparent = Some((pid, parent)));
        self
    }

    fn unclean(self, read: &'static str) -> Self {
        self.edit(|state| state.unclean = Some(read));
        self
    }

    fn calls(&self) -> Vec<String> {
        self.0.borrow().calls.clone()
    }

    fn ps(&self, args: &[String]) -> Option<Result<CommandOutput, CommandError>> {
        let list = &args[args.iter().position(|arg| arg == "-p")? + 1];
        let pids: Vec<u64> = list.split(',').map(|pid| pid.parse().unwrap()).collect();
        let state = self.0.borrow();
        let link = args.iter().any(|arg| arg == "ppid=");
        if state.unclean == Some(if link { "link" } else { "observe" }) {
            let mut error = exit(1);
            error.cleanup_error = Some(std::io::Error::other("ps survived"));
            return Some(Err(error));
        }
        if !pids.iter().all(|pid| state.processes.contains_key(pid)) {
            // Real ps exits 1 for a pid that is gone.
            return link.then(|| Err(exit(1)));
        }
        if link {
            let pid = pids[0];
            let (parent, start) = state.processes[&pid];
            let row = state
                .rows
                .get(&pid)
                .cloned()
                .unwrap_or_else(|| format!("{pid} {parent} {} Ss\n", lstart(start)));
            drop(state);
            self.read_up_to(parent);
            return Some(output(row));
        }
        let rows = pids
            .iter()
            .filter_map(|pid| {
                let start = match state.later.get(pid) {
                    Some(start) => (*start)?,
                    None => state.processes[pid].1,
                };
                Some(format!("{pid} {} Ss\n", lstart(start)))
            })
            .collect();
        Some(output(rows))
    }

    /// A PID-only `ps -o pid=,ppid= -p <pid>` row, as an ordinary command
    /// caller's single-host read asks for.
    fn pid_only(&self, args: &[String]) -> Result<CommandOutput, CommandError> {
        if self.0.borrow().unclean == Some("pid") {
            let mut error = exit(1);
            error.cleanup_error = Some(std::io::Error::other("ps survived"));
            return Err(error);
        }
        let pid: u64 = args.last().and_then(|pid| pid.parse().ok()).unwrap();
        let parent = self.0.borrow().processes.get(&pid).map(|link| link.0);
        let parent = parent.ok_or_else(|| exit(1))?;
        self.read_up_to(parent);
        output(format!("{pid} {parent}\n"))
    }

    /// A chain read has reached `parent`: past the root, a scripted
    /// reparent applies to every later read.
    fn read_up_to(&self, parent: u64) {
        if parent == 0 {
            self.edit(|state| {
                if let Some((pid, parent)) = state.reparent.take() {
                    state.processes.get_mut(&pid).unwrap().0 = parent;
                }
            });
        }
    }

    fn answer_tmux(&self) -> Result<CommandOutput, CommandError> {
        let pane = |pid: u64| fields("%1", "/tmp/nest.sock", "1", &pid.to_string(), "0");
        let answer = self.0.borrow().tmux.clone();
        match answer {
            Some(Answer::Pane(pid)) => output(pane(pid)),
            Some(Answer::Text(text)) => output(text),
            Some(Answer::Late(pid)) => {
                std::thread::sleep(Duration::from_millis(1100));
                output(pane(pid))
            }
            Some(Answer::Fails(kind)) => Err(CommandError::new(kind)),
            None => Err(exit(1)),
        }
    }
}

impl CommandRunner for World {
    fn process_link(&self, _: u64, _: Instant) -> Option<crate::process::ancestry::ProcessLink> {
        self.edit(|state| state.native_links += 1);
        None
    }

    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        let program = request.program.to_string_lossy().into_owned();
        let args: Vec<String> = request
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let ps = program == "/usr/bin/env" && args.get(2).is_some_and(|arg| arg.ends_with("/ps"));
        if ps || program == "ps" || program == "tmux" {
            self.edit(|state| state.calls.push(format!("{program} {}", args.join(" "))));
        }
        if program == "tmux" {
            return self.answer_tmux();
        }
        if program == "ps" {
            return self.pid_only(&args);
        }
        if ps && let Some(result) = self.ps(&args) {
            return result;
        }
        if let Some(op) = args
            .iter()
            .find(|arg| ["caller", "server", "snapshot"].contains(&arg.as_str()))
        {
            self.edit(|state| state.calls.push(format!("driver {op}")));
        }
        if self.0.borrow().unclean == Some("caller") && args.iter().any(|arg| arg == "caller") {
            let _ = HostRunner.execute(request);
            let mut error = exit(1);
            error.cleanup_error = Some(std::io::Error::other("driver survived"));
            return Err(error);
        }
        HostRunner.execute(request)
    }
}

/// tmux in a Herdr pane, the real shape: caller > provider > tmux pane shell
/// 30 > tmux server 40 (daemonized) > launchd. The Herdr pane is not here.
fn tmux_in_herdr() -> World {
    World::chain(&[CALLER, 20, 30, 40, 1]).tmux(Answer::Pane(30))
}

/// Herdr in a tmux pane, the chain #3583 measured (Herdr 0.9.3, tmux 3.7c):
/// caller > provider > Herdr pane shell 30 > Herdr server 40 > `herdr`
/// client 50 > tmux pane shell 60 > tmux server 70 > launchd.
fn herdr_in_tmux() -> World {
    World::chain(&[CALLER, 20, 30, 40, 50, 60, 70, 1]).tmux(Answer::Pane(60))
}

/// Herdr in tmux after the client detached, inferred (not measured): the
/// Herdr server's parent is launchd, so the tmux pane 60 is off the chain.
fn detached() -> World {
    World::chain(&[CALLER, 20, 30, 40, 1]).tmux(Answer::Pane(60))
}

/// The nested caller in `world`: tmux variables for `%1` and the fake
/// host's variable, whose driver answers `pane`. The chosen host and the
/// pinned pane shell.
fn nested(world: &World, pane: Pane) -> Result<(HostKind, Option<u64>), HostError> {
    nested_in(world, pane, TmuxEnv::Full, &mut Ids::default())
}

fn nested_in(
    world: &World,
    pane: Pane,
    env: TmuxEnv,
    ids: &mut impl HostServerIds,
) -> Result<(HostKind, Option<u64>), HostError> {
    admitted(pane, env, CALLER, world.clone(), false, ids)
        .map(|host| (host.primary, host.pinned_caller_pane()))
}

fn tmux_owns(result: Result<(HostKind, Option<u64>), HostError>, pane: u64) {
    assert_eq!(result.unwrap(), (HostKind::Tmux, Some(pane)));
}

fn herdr_owns(result: Result<(HostKind, Option<u64>), HostError>, pane: u64) {
    assert_eq!(result.unwrap(), (HostKind::External(fake()), Some(pane)));
}

fn doubt(result: Result<(HostKind, Option<u64>), HostError>) {
    assert!(
        matches!(&result, Err(HostError::Unavailable(reason)) if reason.starts_with("caller host unverified")),
        "{result:?}"
    );
}

#[test]
fn a_nested_caller_is_owned_by_its_nearest_located_pane() {
    // A1: the tmux pane is on the chain, the Herdr pane (500) is not.
    tmux_owns(nested(&tmux_in_herdr(), Pane::Shell(500)), 30);
    // A2, measured: both on the chain, the Herdr pane nearer.
    herdr_owns(nested(&herdr_in_tmux(), Pane::Shell(30)), 30);
    // A3, inferred: the tmux pane is off the chain after a detach.
    herdr_owns(nested(&detached(), Pane::Shell(30)), 30);
    // A4: the driver conclusively names no pane.
    tmux_owns(nested(&tmux_in_herdr(), Pane::Missing), 30);
    // A5: both located, in both orders; the nearer one owns the caller.
    let world = World::chain(&[CALLER, 20, 30, 40, 50, 60, 70, 1]);
    herdr_owns(
        nested(&world.clone().tmux(Answer::Pane(60)), Pane::Shell(30)),
        30,
    );
    tmux_owns(nested(&world.tmux(Answer::Pane(30)), Pane::Shell(60)), 30);
}

#[test]
fn a_driver_in_doubt_refuses_and_never_falls_back_to_tmux() {
    // The tmux pane is on the chain, so a fallback would admit tmux.
    tmux_owns(nested(&tmux_in_herdr(), Pane::Missing), 30);
    doubt(nested(&tmux_in_herdr(), Pane::Fails));
    doubt(nested(&tmux_in_herdr(), Pane::Changed(500)));
    doubt(nested(&tmux_in_herdr(), Pane::NotText(500)));
    // Refused before tmux is asked.
    let world = tmux_in_herdr();
    doubt(nested(&world, Pane::Fails));
    assert!(
        !world.calls().iter().any(|call| call.starts_with("tmux")),
        "{:?}",
        world.calls()
    );
    // A driver that names the tmux pane's own shell: one pane process has
    // one host, so neither owns it.
    doubt(nested(&tmux_in_herdr(), Pane::Shell(30)));
}

#[test]
fn tmux_in_doubt_refuses_and_never_counts_as_absent() {
    // The Herdr pane owns the caller when tmux is proven absent.
    herdr_owns(nested(&detached(), Pane::Shell(30)), 30);
    let pane = |id: &str, socket: &str, server: &str, pid: &str, dead: &str| {
        Answer::Text(fields(id, socket, server, pid, dead))
    };
    for (case, answer) in [
        (
            "no server",
            Answer::Fails(CommandFailure::Exit {
                code: Some(1),
                signal: None,
            }),
        ),
        ("timeout", Answer::Fails(CommandFailure::Timeout)),
        (
            "not a number",
            pane("%1", "/tmp/nest.sock", "1", "sixty", "0"),
        ),
        (
            "two lines",
            Answer::Text(fields("%1", "/tmp/nest.sock", "1", "60", "0").repeat(2)),
        ),
        ("another pane", pane("%2", "/tmp/nest.sock", "1", "60", "0")),
        (
            "another socket",
            pane("%1", "/tmp/other.sock", "1", "60", "0"),
        ),
        (
            "a restarted server",
            pane("%1", "/tmp/nest.sock", "2", "60", "0"),
        ),
        ("a dead pane", pane("%1", "/tmp/nest.sock", "1", "60", "1")),
    ] {
        let result = nested(&detached().tmux(answer), Pane::Shell(30));
        assert!(
            matches!(&result, Err(HostError::Unavailable(_))),
            "{case}: {result:?}"
        );
    }
}

#[test]
fn a_partial_tmux_environment_refuses_before_any_read() {
    let ids = &mut Ids::default();
    herdr_owns(
        nested_in(&detached(), Pane::Shell(30), TmuxEnv::Full, ids),
        30,
    );
    for env in [TmuxEnv::PaneOnly, TmuxEnv::TmuxOnly] {
        let world = detached();
        doubt(nested_in(&world, Pane::Shell(30), env, ids));
        // Refused before the pin, the driver and tmux: no read at all.
        assert_eq!(world.calls(), Vec::<String>::new());
    }
}

#[test]
fn an_ancestry_that_is_not_one_consistent_read_refuses() {
    herdr_owns(nested(&herdr_in_tmux(), Pane::Shell(30)), 30);
    let rewritten = |row: &str| herdr_in_tmux().row(40, row);
    for (case, world) in [
        ("an ancestor gone during the read", herdr_in_tmux().gone(40)),
        (
            "a row for another pid",
            rewritten("41 50 Thu Jan 1 00:16:40 1970 Ss"),
        ),
        ("a malformed row", rewritten("40 50 yesterday Ss")),
        // The Herdr server's pid now names a process younger than its child:
        // the real parent ended and its pid was reused.
        ("a reused pid", herdr_in_tmux().started(40, 5000)),
        (
            "a cycle",
            herdr_in_tmux().row(70, "70 30 Thu Jan 1 00:00:01 1970 Ss"),
        ),
        ("the 1 s budget", herdr_in_tmux().tmux(Answer::Late(60))),
    ] {
        let result = nested(&world, Pane::Shell(30));
        assert!(
            matches!(&result, Err(HostError::Unavailable(_))),
            "{case}: {result:?}"
        );
    }
}

#[test]
fn a_process_up_to_the_owner_pane_that_changes_before_the_answer_refuses() {
    // Entries above the owner pane may change: they don't own the caller.
    herdr_owns(
        nested(
            &herdr_in_tmux().later(60, Some(9000)).later(70, None),
            Pane::Shell(30),
        ),
        30,
    );
    doubt(nested(
        &herdr_in_tmux().later(30, Some(9000)),
        Pane::Shell(30),
    ));
    doubt(nested(&herdr_in_tmux().later(20, None), Pane::Shell(30)));
    doubt(nested(
        &herdr_in_tmux().later(CALLER, Some(9000)),
        Pane::Shell(30),
    ));
    tmux_owns(
        nested(&tmux_in_herdr().later(40, None), Pane::Shell(500)),
        30,
    );
    doubt(nested(
        &tmux_in_herdr().later(30, Some(9000)),
        Pane::Shell(500),
    ));
    doubt(nested(&tmux_in_herdr().later(20, None), Pane::Shell(500)));
}

#[test]
fn a_nested_caller_without_a_located_pane_refuses() {
    tmux_owns(nested(&tmux_in_herdr(), Pane::Shell(500)), 30);
    doubt(nested(
        &tmux_in_herdr().tmux(Answer::Pane(600)),
        Pane::Shell(500),
    ));
}

/// A recorded-server lookup that finds no bind's server.
struct Unrecorded;

impl HostServerIds for Unrecorded {
    type Error = std::io::Error;

    fn server_id(&mut self, _: &HostServerIncarnation<'_>) -> Result<String, Self::Error> {
        Err(std::io::Error::other("no recorded server"))
    }
}

#[test]
fn an_external_owner_needs_its_recorded_server() {
    herdr_owns(nested(&herdr_in_tmux(), Pane::Shell(30)), 30);
    let result = nested_in(
        &herdr_in_tmux(),
        Pane::Shell(30),
        TmuxEnv::Full,
        &mut Unrecorded,
    );
    assert!(matches!(result, Err(HostError::ServerIds(_))), "{result:?}");
}

#[test]
fn a_nested_caller_reads_one_pinned_chain_and_one_tmux_pane() {
    let world = herdr_in_tmux();
    herdr_owns(nested(&world, Pane::Shell(30)), 30);
    let calls = world.calls();
    let links: Vec<_> = calls
        .iter()
        .filter(|call| call.contains("ppid=") && call.contains("lstart="))
        .collect();
    let chain = [CALLER, 20, 30, 40, 50, 60, 70, 1];
    assert_eq!(links.len(), chain.len(), "{calls:?}");
    for (call, pid) in links.iter().zip(chain) {
        assert!(call.contains(&format!("-p {pid} ")), "{call}");
    }
    assert_eq!(world.0.borrow().native_links, chain.len());
    // No PID-only read, and one tmux call on the socket `TMUX` names.
    assert!(
        !calls.iter().any(|call| call.starts_with("ps ")),
        "{calls:?}"
    );
    let tmux: Vec<_> = calls
        .iter()
        .filter(|call| call.starts_with("tmux"))
        .collect();
    let format = [
        "#{pane_id}",
        "#{socket_path}",
        "#{pid}",
        "#{pane_pid}",
        "#{pane_dead}",
    ]
    .join(SEPARATOR);
    assert_eq!(
        tmux,
        [&format!(
            "tmux -u -S /tmp/nest.sock display-message -p -t %1 {format}"
        )]
    );
}

/// Counts the pinned reads a runner is asked for, and runs everything.
#[derive(Clone)]
struct Counting<R> {
    inner: R,
    pinned: Rc<RefCell<usize>>,
}

impl<R: CommandRunner> CommandRunner for Counting<R> {
    fn process_link(
        &self,
        pid: u64,
        deadline: Instant,
    ) -> Option<crate::process::ancestry::ProcessLink> {
        *self.pinned.borrow_mut() += 1;
        self.inner.process_link(pid, deadline)
    }

    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        let args: Vec<_> = request
            .args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect();
        if args.iter().any(|arg| arg == "ppid=") && args.iter().any(|arg| arg == "lstart=") {
            *self.pinned.borrow_mut() += 1;
        }
        self.inner.execute(request)
    }
}

#[test]
fn a_caller_named_by_one_host_or_none_reads_no_pinned_chain() {
    use crate::host::{CallerEnvironment, Host};
    let chain = ancestors();
    let counting = |pane| Counting {
        inner: Nested { pane },
        pinned: Rc::default(),
    };
    let external = counting(None);
    let (pid, _) = live();
    let host = choose(
        Pane::Shell(chain[0]),
        TmuxEnv::Absent,
        pid,
        external.clone(),
        false,
    );
    assert_eq!(host.unwrap(), HostKind::External(fake()));
    assert_eq!(*external.pinned.borrow(), 0);
    let builtin = counting(Some(chain[0]));
    let caller = CallerEnvironment {
        tmux: Some("/tmp/nest.sock,1,0".into()),
        pane: Some("%1".into()),
        process_id: pid,
        driver_env: Default::default(),
    };
    let host = Host::for_verified_caller(&caller, builtin.clone(), Vec::new(), &mut Ids::default())
        .unwrap();
    assert_eq!(
        (host.primary, host.pinned_caller_pane()),
        (HostKind::Tmux, None)
    );
    assert_eq!(*builtin.pinned.borrow(), 0);
}

#[test]
fn a_failed_cleanup_is_an_error_of_its_own() {
    herdr_owns(nested(&herdr_in_tmux(), Pane::Shell(30)), 30);
    for read in ["link", "observe"] {
        let result = nested(&herdr_in_tmux().unclean(read), Pane::Shell(30));
        assert!(
            matches!(&result, Err(error @ HostError::Process(_)) if error.cleanup_failed()),
            "{read}: {result:?}"
        );
    }
}

/// The host an ordinary command picks for the caller in `world`: the fake
/// host's variable, whose driver answers `pane`, and the tmux variables `env`.
fn ordinary(world: &World, pane: Pane, env: TmuxEnv) -> HostKind {
    with_caller(pane, env, CALLER, false, |environment, records| {
        crate::host::Host::for_caller_with_drivers(environment, world.clone(), records).primary
    })
}

fn herdr() -> HostKind {
    HostKind::External(fake())
}

/// tmux pane shell 30 nearer than the located Herdr pane shell 60 (A5).
fn herdr_around_tmux() -> World {
    World::chain(&[CALLER, 20, 30, 40, 50, 60, 70, 1]).tmux(Answer::Pane(30))
}

#[test]
fn an_ordinary_command_orders_both_panes_on_one_pinned_ancestry() {
    let full = TmuxEnv::Full;
    assert_eq!(
        ordinary(&tmux_in_herdr(), Pane::Shell(500), full),
        HostKind::Tmux
    );
    assert_eq!(ordinary(&herdr_in_tmux(), Pane::Shell(30), full), herdr());
    assert_eq!(ordinary(&detached(), Pane::Shell(30), full), herdr());
    assert_eq!(
        ordinary(&herdr_around_tmux(), Pane::Shell(60), full),
        HostKind::Tmux
    );
    let world = herdr_in_tmux();
    assert_eq!(ordinary(&world, Pane::Shell(30), full), herdr());
    // One pinned read of the whole chain, one tmux call, then one re-check
    // of the caller up to the Herdr pane; no PID-only read.
    let calls = world.calls();
    let links = calls
        .iter()
        .filter(|call| call.contains("ppid=") && call.contains("lstart="))
        .count();
    assert_eq!(links, 8, "{calls:?}");
    assert!(
        !calls.iter().any(|call| call.starts_with("ps ")),
        "{calls:?}"
    );
    let tmux = calls.iter().filter(|call| call.starts_with("tmux")).count();
    assert_eq!(tmux, 1, "{calls:?}");
    let observe: Vec<_> = calls
        .iter()
        .filter(|call| call.contains("lstart=") && !call.contains("ppid="))
        .collect();
    assert_eq!(observe.len(), 1, "{calls:?}");
    assert!(
        observe[0].contains(&format!("-p {CALLER},20,30 ")),
        "{calls:?}"
    );
}

#[test]
fn a_reparent_between_reads_cannot_change_the_order() {
    // The tmux pane is nearer on the one read. A later read would see the
    // provider reparented to launchd and the tmux pane off the chain.
    let world = herdr_around_tmux().reparent(20, 1);
    assert_eq!(
        ordinary(&world, Pane::Shell(60), TmuxEnv::Full),
        HostKind::Tmux
    );
}

#[test]
fn a_changed_process_up_to_the_external_pane_keeps_tmux() {
    let full = TmuxEnv::Full;
    // Entries above the Herdr pane may change: they don't own the caller.
    assert_eq!(
        ordinary(
            &herdr_in_tmux().later(60, Some(9000)).later(70, None),
            Pane::Shell(30),
            full
        ),
        herdr()
    );
    for (case, world) in [
        // The Herdr server's pid names a process younger than its child: the
        // real parent ended and its pid was reused.
        ("a reused pid", herdr_in_tmux().started(40, 5000)),
        ("the pane shell", herdr_in_tmux().later(30, Some(9000))),
        ("the provider", herdr_in_tmux().later(20, None)),
        ("the caller", herdr_in_tmux().later(CALLER, Some(9000))),
    ] {
        assert_eq!(
            ordinary(&world, Pane::Shell(30), full),
            HostKind::Tmux,
            "{case}"
        );
    }
    // Out of time before the answer. A deadline already passed fails at the
    // pin; the re-check's own deadline arm is shared with hooks and tested
    // by their 1 s budget case.
    with_caller(
        Pane::Shell(30),
        full,
        CALLER,
        false,
        |environment, records| {
            let host = crate::host::Host::of(HostKind::Tmux, herdr_in_tmux()).with_drivers(records);
            assert!(
                host.nearest_nested_external(environment, Instant::now() + Duration::from_secs(1))
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                host.nearest_nested_external(environment, Instant::now())
                    .unwrap(),
                None
            );
        },
    );
}

#[test]
fn a_driver_in_doubt_keeps_tmux() {
    let full = TmuxEnv::Full;
    assert_eq!(ordinary(&detached(), Pane::Shell(30), full), herdr());
    for (case, world, pane) in [
        ("no pane", detached(), Pane::Missing),
        ("an error", detached(), Pane::Fails),
        ("a changed executable", detached(), Pane::Changed(30)),
        ("a variable that is not text", detached(), Pane::NotText(30)),
        ("a pane off the chain", detached(), Pane::Shell(500)),
        (
            "an ancestry that can't be pinned",
            detached().gone(40),
            Pane::Shell(30),
        ),
        (
            "a failed cleanup",
            detached().unclean("link"),
            Pane::Shell(30),
        ),
        // One pane process has one host, so the fallback keeps tmux.
        ("both hosts name one pane", tmux_in_herdr(), Pane::Shell(30)),
    ] {
        assert_eq!(ordinary(&world, pane, full), HostKind::Tmux, "{case}");
    }
}

#[test]
fn tmux_in_doubt_counts_as_absent() {
    // Proven, the tmux pane is nearer than the Herdr pane and owns the caller.
    assert_eq!(
        ordinary(&herdr_around_tmux(), Pane::Shell(60), TmuxEnv::Full),
        HostKind::Tmux
    );
    let pane = |id: &str, socket: &str, server: &str, dead: &str| {
        Answer::Text(fields(id, socket, server, "30", dead))
    };
    for (case, answer, env) in [
        (
            "no server",
            Answer::Fails(CommandFailure::Exit {
                code: Some(1),
                signal: None,
            }),
            TmuxEnv::Full,
        ),
        (
            "timeout",
            Answer::Fails(CommandFailure::Timeout),
            TmuxEnv::Full,
        ),
        (
            "another pane",
            pane("%2", "/tmp/nest.sock", "1", "0"),
            TmuxEnv::Full,
        ),
        (
            "a stale socket",
            pane("%1", "/tmp/other.sock", "1", "0"),
            TmuxEnv::Full,
        ),
        (
            "a restarted server",
            pane("%1", "/tmp/nest.sock", "2", "0"),
            TmuxEnv::Full,
        ),
        (
            "a dead pane",
            pane("%1", "/tmp/nest.sock", "1", "1"),
            TmuxEnv::Full,
        ),
        ("only TMUX_PANE", Answer::Pane(30), TmuxEnv::PaneOnly),
        ("only TMUX", Answer::Pane(30), TmuxEnv::TmuxOnly),
    ] {
        let world = herdr_around_tmux().tmux(answer);
        assert_eq!(ordinary(&world, Pane::Shell(60), env), herdr(), "{case}");
    }
}

#[test]
fn an_ordinary_command_named_by_one_host_or_none_reads_no_pinned_chain() {
    let world = herdr_in_tmux();
    assert_eq!(ordinary(&world, Pane::Shell(30), TmuxEnv::Absent), herdr());
    let calls = world.calls();
    assert!(
        !calls
            .iter()
            .any(|call| call.contains("lstart=") || call.starts_with("tmux")),
        "{calls:?}"
    );
    assert!(
        calls.iter().any(|call| call.starts_with("ps ")),
        "{calls:?}"
    );
    let world = herdr_in_tmux();
    let caller = crate::host::CallerEnvironment {
        tmux: Some("/tmp/nest.sock,1,0".into()),
        pane: Some("%1".into()),
        process_id: CALLER,
        driver_env: Default::default(),
    };
    let host = crate::host::Host::for_caller_with_drivers(&caller, world.clone(), Vec::new());
    assert_eq!(host.primary, HostKind::Tmux);
    assert_eq!(world.calls(), Vec::<String>::new());
}

/// Discovery's selection (#4063): a read that fails to clean up, plain or
/// nested, is an error that chooses no host, and no inventory or tmux scan
/// follows it. The same world without the fault admits the fake host; an
/// ordinary command keeps its fallback to tmux (#3920).
#[test]
fn a_failed_cleanup_in_discovery_selection_chooses_no_host() {
    let select = |world: &World, env: TmuxEnv| {
        with_caller(
            Pane::Shell(30),
            env,
            CALLER,
            false,
            |environment, records| {
                let deadline = Instant::now() + Duration::from_secs(1);
                crate::host::Host::for_caller_within(environment, world.clone(), records, deadline)
                    .map(|host| host.primary)
            },
        )
    };
    for (env, read) in [
        (TmuxEnv::Absent, "caller"),
        (TmuxEnv::Absent, "pid"),
        (TmuxEnv::Full, "caller"),
        (TmuxEnv::Full, "link"),
        (TmuxEnv::Full, "observe"),
    ] {
        assert_eq!(select(&herdr_in_tmux(), env).unwrap(), herdr(), "{read}");
        let world = herdr_in_tmux().unclean(read);
        let result = select(&world, env);
        assert!(
            matches!(&result, Err(error) if error.cleanup_failed()),
            "{read}: {result:?}"
        );
        let calls = world.calls();
        assert!(
            !calls.iter().any(|call| call == "driver server"
                || call == "driver snapshot"
                || call.starts_with("tmux list-panes")),
            "{read}: {calls:?}"
        );
        assert_eq!(
            ordinary(&herdr_in_tmux().unclean(read), Pane::Shell(30), env),
            HostKind::Tmux,
            "{read}"
        );
    }
}
