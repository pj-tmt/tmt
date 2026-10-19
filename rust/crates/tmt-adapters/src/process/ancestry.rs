//! A process's parent chain from bounded native reads or `ps` calls. A host
//! proves that a caller runs inside one of its panes by finding that pane's
//! process in this chain; the nearest such pane is the caller's own.

use super::{
    CommandError, CommandFailure, CommandRequest, CommandRunner,
    ps::query_ps,
    runtime::{ProcessObservation, parse_process_observation},
};
use std::{ffi::OsString, time::Instant};
use tmt_core::{endpoint::ProcessIncarnation, limits::MAX_JS_SAFE_INTEGER};

const MAX_DEPTH: usize = 64;
const MAX_OUTPUT: usize = 64 * 1024;

#[derive(Debug)]
pub enum AncestryError {
    Command(CommandError),
    /// The chain was malformed, cyclic, too deep or out of time, or (pinned
    /// chain only) an entry ended or had a reused pid.
    Unavailable,
}

/// `first` and its ancestors, nearest first, ending before PID 0.
pub fn chain<R: CommandRunner>(
    runner: &R,
    first: u64,
    deadline: Instant,
) -> Result<Vec<u64>, AncestryError> {
    let mut chain = Vec::new();
    let mut pid = first;
    for _ in 0..MAX_DEPTH {
        if pid == 0 || pid > MAX_JS_SAFE_INTEGER || chain.contains(&pid) {
            return Err(AncestryError::Unavailable);
        }
        chain.push(pid);
        if Instant::now() >= deadline {
            return Err(AncestryError::Command(CommandError::new(
                CommandFailure::Timeout,
            )));
        }
        let args: Vec<OsString> = ["-o", "pid=,ppid=", "-p", &pid.to_string()]
            .into_iter()
            .map(Into::into)
            .collect();
        let output = runner
            .execute(CommandRequest {
                program: "ps".as_ref(),
                args: &args,
                input: &[],
                deadline,
                max_output_bytes: MAX_OUTPUT,
            })
            .map_err(AncestryError::Command)?;
        let text = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<_> = text.trim().lines().collect();
        let [line] = lines.as_slice() else {
            return Err(AncestryError::Unavailable);
        };
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 2 || decimal(fields[0]) != Some(pid) {
            return Err(AncestryError::Unavailable);
        }
        let parent = decimal(fields[1]).ok_or(AncestryError::Unavailable)?;
        if parent == 0 {
            return Ok(chain);
        }
        pid = parent;
    }
    Err(AncestryError::Unavailable)
}

/// When a process started, for ordering a parent before its child. Native
/// reads give microseconds; the ps fallback gives whole seconds only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartOrder {
    seconds: u64,
    micros: Option<u32>,
}

impl StartOrder {
    pub(super) fn precise(seconds: u64, micros: u32) -> Self {
        Self {
            seconds,
            micros: Some(micros),
        }
    }

    fn whole_seconds(seconds: u64) -> Self {
        Self {
            seconds,
            micros: None,
        }
    }

    /// Whether this start is later than `other`. Two starts of different
    /// resolution compare at whole seconds: a child read by ps at 10 s (its
    /// real start 10.7 s) under a parent read natively at 10.3 s is in order.
    fn later_than(self, other: Self) -> bool {
        match (self.micros, other.micros) {
            (Some(own), Some(theirs)) => (self.seconds, own) > (other.seconds, theirs),
            _ => self.seconds > other.seconds,
        }
    }
}

/// One process as a single read saw it: its state and start, its parent, and
/// its start order. The read is the evidence; nothing in it comes from a
/// second call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessLink {
    pub observation: ProcessObservation,
    pub parent: u64,
    pub start: StartOrder,
}

/// One entry of a pinned chain: the process (pid and start token) and its
/// parent pid, from the same read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedLink {
    pub process: ProcessIncarnation,
    pub parent: u64,
    start: StartOrder,
}

/// A caller's ancestry, nearest first, where every entry is pinned by its
/// start. Each link names the next one as its parent, and no parent started
/// after its child, so a pid that was reused during the read is not in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedChain {
    links: Vec<PinnedLink>,
}

impl PinnedChain {
    pub fn links(&self) -> &[PinnedLink] {
        &self.links
    }

    /// The index of `pid` on the chain, nearest first.
    pub fn position(&self, pid: u64) -> Option<usize> {
        self.links.iter().position(|link| link.process.pid() == pid)
    }
}

/// `first` and its ancestors, nearest first, ending before PID 0, with each
/// entry's parent and start from one read: native where the runner has it,
/// else one fixed-locale, UTC `ps` call (so the start tokens equal the native
/// ones). An entry that is gone, a zombie or unreadable, a link whose parent
/// started after its child (a reused pid), a cycle, more than 64 entries or
/// the deadline makes the whole chain unavailable; it is never partial.
pub fn pinned_chain<R: CommandRunner>(
    runner: &R,
    first: u64,
    deadline: Instant,
) -> Result<PinnedChain, AncestryError> {
    let mut links: Vec<PinnedLink> = Vec::new();
    let mut pid = first;
    for _ in 0..MAX_DEPTH {
        if pid == 0
            || pid > MAX_JS_SAFE_INTEGER
            || links.iter().any(|link| link.process.pid() == pid)
        {
            return Err(AncestryError::Unavailable);
        }
        if Instant::now() >= deadline {
            return Err(timeout());
        }
        let link = match runner.process_link(pid, deadline) {
            Some(link) => link,
            None => ps_link(runner, pid, deadline)?,
        };
        // A late read is never current evidence.
        if Instant::now() >= deadline {
            return Err(timeout());
        }
        let (ProcessObservation::Live(process) | ProcessObservation::Stopped(process)) =
            link.observation
        else {
            return Err(AncestryError::Unavailable);
        };
        if process.pid() != pid
            || link.parent > MAX_JS_SAFE_INTEGER
            || links
                .last()
                .is_some_and(|child| link.start.later_than(child.start))
        {
            return Err(AncestryError::Unavailable);
        }
        let parent = link.parent;
        links.push(PinnedLink {
            process,
            parent,
            start: link.start,
        });
        if parent == 0 {
            return Ok(PinnedChain { links });
        }
        pid = parent;
    }
    Err(AncestryError::Unavailable)
}

fn timeout() -> AncestryError {
    AncestryError::Command(CommandError::new(CommandFailure::Timeout))
}

/// One `ps` row: pid, parent, start and state, in UTC.
fn ps_link<R: CommandRunner>(
    runner: &R,
    pid: u64,
    deadline: Instant,
) -> Result<ProcessLink, AncestryError> {
    let args: Vec<OsString> = [
        "-p",
        &pid.to_string(),
        "-o",
        "pid=",
        "-o",
        "ppid=",
        "-o",
        "lstart=",
        "-o",
        "stat=",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    // ps exits 1 for a gone pid: that is absence, not a failed command. A
    // timeout or a failed cleanup stays a command error.
    let output = query_ps(runner, &args, deadline, 512).map_err(|error| {
        if !error.cleanup_failed() && matches!(error.kind, CommandFailure::Exit { .. }) {
            AncestryError::Unavailable
        } else {
            AncestryError::Command(error)
        }
    })?;
    let text = std::str::from_utf8(&output.stdout).map_err(|_| AncestryError::Unavailable)?;
    let lines: Vec<_> = text.trim().lines().collect();
    let [line] = lines.as_slice() else {
        return Err(AncestryError::Unavailable);
    };
    let fields: Vec<_> = line.split_whitespace().collect();
    let [own, parent, start @ ..] = fields.as_slice() else {
        return Err(AncestryError::Unavailable);
    };
    if decimal(own) != Some(pid) {
        return Err(AncestryError::Unavailable);
    }
    let parent = decimal(parent).ok_or(AncestryError::Unavailable)?;
    let observation = parse_process_observation(pid, start.join(" ").as_bytes());
    let seconds = utc_seconds(start).ok_or(AncestryError::Unavailable)?;
    Ok(ProcessLink {
        observation,
        parent,
        start: StartOrder::whole_seconds(seconds),
    })
}

/// Seconds since the epoch of a UTC `lstart` ("Mon Oct 19 07:24:35 2026"),
/// whose shape the observation parser has already checked.
fn utc_seconds(fields: &[&str]) -> Option<u64> {
    let [_, month, day, clock, year, ..] = fields else {
        return None;
    };
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|name| name == month)?;
    let month = time::Month::try_from(u8::try_from(month + 1).ok()?).ok()?;
    let mut clock = clock.split(':').map(|part| part.parse::<u8>().ok());
    let (hour, minute, second) = (clock.next()??, clock.next()??, clock.next()??);
    let date = time::Date::from_calendar_date(year.parse().ok()?, month, day.parse().ok()?).ok()?;
    let time = time::Time::from_hms(hour, minute, second).ok()?;
    u64::try_from(
        time::PrimitiveDateTime::new(date, time)
            .assume_utc()
            .unix_timestamp(),
    )
    .ok()
}

/// `ps` prints decimal integers; signs, hex and exponents are not evidence.
fn decimal(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse()
        .ok()
        .filter(|value| tmt_core::limits::is_valid_js_safe_integer(*value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{CommandOutput, UnixCommandRunner};
    use std::{cell::RefCell, collections::HashMap, time::Duration};

    /// A scripted process tree: some pids answer natively, others only to
    /// `ps`. Every `ps` call is recorded.
    #[derive(Default)]
    struct Tree {
        native: HashMap<u64, (u64, u64, u32, ProcessState)>,
        ps: HashMap<u64, String>,
        ps_calls: RefCell<Vec<Vec<String>>>,
    }

    #[derive(Clone, Copy)]
    enum ProcessState {
        Live,
        Zombie,
    }

    impl Tree {
        fn native(mut self, pid: u64, parent: u64, seconds: u64, micros: u32) -> Self {
            self.native
                .insert(pid, (parent, seconds, micros, ProcessState::Live));
            self
        }

        fn zombie(mut self, pid: u64, parent: u64, seconds: u64) -> Self {
            self.native
                .insert(pid, (parent, seconds, 0, ProcessState::Zombie));
            self
        }

        fn ps(mut self, pid: u64, parent: u64, lstart: &str) -> Self {
            self.ps.insert(pid, format!("{pid} {parent} {lstart} Ss\n"));
            self
        }

        fn row(mut self, pid: u64, text: &str) -> Self {
            self.ps.insert(pid, text.into());
            self
        }
    }

    impl CommandRunner for Tree {
        fn process_link(&self, pid: u64, _: Instant) -> Option<ProcessLink> {
            let &(parent, seconds, micros, state) = self.native.get(&pid)?;
            let process = ProcessIncarnation::new(pid, &format!("native {seconds}")).unwrap();
            let observation = match state {
                ProcessState::Live => ProcessObservation::Live(process),
                ProcessState::Zombie => ProcessObservation::UnreapedZombie(process),
            };
            Some(ProcessLink {
                observation,
                parent,
                start: StartOrder::precise(seconds, micros),
            })
        }

        fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
            let args: Vec<String> = request
                .args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            let pid: u64 = args[args.iter().position(|arg| arg == "-p").unwrap() + 1]
                .parse()
                .unwrap();
            self.ps_calls.borrow_mut().push(args);
            // Real ps exits 1 and prints nothing for a pid that is gone.
            let row = self.ps.get(&pid).ok_or_else(|| {
                CommandError::new(CommandFailure::Exit {
                    code: Some(1),
                    signal: None,
                })
            })?;
            Ok(CommandOutput {
                stdout: row.clone().into_bytes(),
                stderr: vec![],
            })
        }
    }

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    fn pids(chain: &PinnedChain) -> Vec<u64> {
        chain
            .links()
            .iter()
            .map(|link| link.process.pid())
            .collect()
    }

    fn refused(tree: &Tree, first: u64) -> bool {
        matches!(
            pinned_chain(tree, first, soon()),
            Err(AncestryError::Unavailable)
        )
    }

    /// caller 100 > 90 > 80 > launchd 1, read natively; each parent started
    /// before its child (100 in the same second as 90, later microseconds).
    fn native_tree() -> Tree {
        Tree::default()
            .native(100, 90, 3_000, 500)
            .native(90, 80, 3_000, 400)
            .native(80, 1, 2_000, 0)
            .native(1, 0, 1_000, 0)
    }

    #[test]
    fn a_native_chain_is_pinned_nearest_first_from_one_read_per_entry() {
        let tree = native_tree();
        let chain = pinned_chain(&tree, 100, soon()).unwrap();
        assert_eq!(pids(&chain), [100, 90, 80, 1]);
        assert_eq!(
            chain
                .links()
                .iter()
                .map(|link| link.parent)
                .collect::<Vec<_>>(),
            [90, 80, 1, 0]
        );
        assert_eq!(chain.position(80), Some(2));
        assert_eq!(chain.position(70), None);
        assert!(
            tree.ps_calls.borrow().is_empty(),
            "no ps call beside native reads"
        );
    }

    #[test]
    fn a_parent_that_started_after_its_child_is_a_reused_pid() {
        // Native microseconds: 90 started 0.1 s after its child 100.
        assert!(refused(&native_tree().native(90, 80, 3_000, 600), 100));
        // Whole seconds through ps: 90 started 1 s after its child.
        let ps = |parent_start: &str| {
            Tree::default()
                .ps(100, 90, "Thu Jan 1 00:50:00 1970")
                .ps(90, 1, parent_start)
                .ps(1, 0, "Thu Jan 1 00:00:00 1970")
        };
        assert!(refused(&ps("Thu Jan 1 00:50:01 1970"), 100));
        // Controls: the same trees with the parent not later admit.
        assert!(pinned_chain(&native_tree(), 100, soon()).is_ok());
        assert_eq!(
            pids(&pinned_chain(&ps("Thu Jan 1 00:50:00 1970"), 100, soon()).unwrap()),
            [100, 90, 1]
        );
    }

    #[test]
    fn mixed_resolutions_compare_at_whole_seconds() {
        // The child is read by ps at 3000 s (its real start may be 3000.7 s);
        // a native parent at 3000.3 s is in order, one at 3001 s is not.
        let tree = |parent_seconds, parent_micros| {
            Tree::default()
                .ps(100, 90, "Thu Jan 1 00:50:00 1970")
                .native(90, 1, parent_seconds, parent_micros)
                .native(1, 0, 1_000, 0)
        };
        assert_eq!(
            pids(&pinned_chain(&tree(3_000, 300_000), 100, soon()).unwrap()),
            [100, 90, 1]
        );
        assert!(refused(&tree(3_001, 0), 100));
    }

    #[test]
    fn an_entry_that_is_gone_a_zombie_or_unreadable_makes_no_chain() {
        // 90 is neither native nor in ps: gone.
        let gone = Tree::default()
            .native(100, 90, 3_000, 0)
            .native(1, 0, 1_000, 0);
        assert!(refused(&gone, 100));
        assert!(refused(&native_tree().zombie(90, 80, 3_000), 100));
        let unreadable = |row: &str| {
            Tree::default()
                .native(100, 90, 3_000, 0)
                .row(90, row)
                .native(1, 0, 1_000, 0)
        };
        assert!(refused(&unreadable("garbage\n"), 100));
        assert!(refused(
            &unreadable("91 1 Thu Jan 1 00:20:00 1970 S\n"),
            100
        ));
        assert!(refused(
            &unreadable("90 1 Thu Jan 1 00:20:00 1970 Z\n"),
            100
        ));
        assert!(refused(
            &unreadable("90 1 Thu Jan 1 00:20:00 1970 S\n90 1 Thu Jan 1 00:20:00 1970 S\n"),
            100
        ));
        // Control: the same tree with a valid row for 90.
        assert_eq!(
            pids(
                &pinned_chain(&unreadable("90 1 Thu Jan 1 00:20:00 1970 S\n"), 100, soon())
                    .unwrap()
            ),
            [100, 90, 1]
        );
    }

    #[test]
    fn cycles_depth_pid_zero_and_the_deadline_bound_the_read() {
        let cycle = Tree::default()
            .native(100, 90, 3_000, 0)
            .native(90, 100, 2_000, 0);
        assert!(refused(&cycle, 100));
        let deep = (2..=70).fold(Tree::default().native(1, 0, 0, 0), |tree, pid| {
            tree.native(pid, pid - 1, pid, 0)
        });
        assert!(refused(&deep, 65));
        assert_eq!(pids(&pinned_chain(&deep, 64, soon()).unwrap()).len(), 64);
        assert!(refused(&native_tree(), 0));
        assert!(matches!(
            pinned_chain(&native_tree(), 100, Instant::now()),
            Err(AncestryError::Command(error)) if error.kind == CommandFailure::Timeout
        ));
    }

    #[test]
    fn the_ps_fallback_reads_one_row_per_entry_in_utc_with_a_fixed_locale() {
        let tree = Tree::default().ps(100, 1, "Mon Oct 19 07:24:35 2026").ps(
            1,
            0,
            "Sat Oct 10 13:00:36 2026",
        );
        assert_eq!(pids(&pinned_chain(&tree, 100, soon()).unwrap()), [100, 1]);
        let calls = tree.ps_calls.borrow();
        assert_eq!(calls.len(), 2);
        for call in calls.iter() {
            assert_eq!(&call[..3], ["LC_ALL=C", "TZ=UTC", "/bin/ps"]);
            assert_eq!(
                &call[5..],
                ["-o", "pid=", "-o", "ppid=", "-o", "lstart=", "-o", "stat="]
            );
        }
        assert_eq!(
            utc_seconds(&["Thu", "Jan", "1", "00:00:00", "1970"]),
            Some(0)
        );
        assert_eq!(
            utc_seconds(&["Mon", "Oct", "19", "07:24:35", "2026"]),
            Some(1_792_394_675)
        );
        assert_eq!(utc_seconds(&["Mon", "Feb", "30", "07:24:35", "2026"]), None);
    }

    #[test]
    fn a_real_chain_is_the_same_natively_and_through_ps() {
        // Without native reads the runner takes the ps fallback. Equal tokens
        // prove both read the same start in UTC; equal pids match `chain`.
        struct PsOnly;
        impl CommandRunner for PsOnly {
            fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
                UnixCommandRunner.execute(request)
            }
        }
        let pid = u64::from(std::process::id());
        let native = pinned_chain(&UnixCommandRunner, pid, soon()).unwrap();
        let ps = pinned_chain(&PsOnly, pid, soon()).unwrap();
        assert_eq!(
            native
                .links()
                .iter()
                .map(|link| &link.process)
                .collect::<Vec<_>>(),
            ps.links()
                .iter()
                .map(|link| &link.process)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            pids(&native),
            chain(&UnixCommandRunner, pid, soon()).unwrap()
        );
        assert_eq!(native.position(pid), Some(0));
        // A reaped child is gone for both reads, and its exit is no command error.
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let gone = u64::from(child.id());
        child.wait().unwrap();
        for runner in [&UnixCommandRunner as &dyn CommandRunner, &PsOnly] {
            assert!(matches!(
                pinned_chain(&runner, gone, soon()),
                Err(AncestryError::Unavailable)
            ));
        }
    }
}
