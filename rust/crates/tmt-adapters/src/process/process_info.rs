//! Native selected-process acquisition. Callers retain policy and ps fallback.

use super::runtime::ProcessObservation;
use std::time::Instant;
use tmt_core::endpoint::ProcessIncarnation;

pub(super) fn observe(pid: u64, deadline: Instant) -> Option<ProcessObservation> {
    if Instant::now() >= deadline {
        return Some(ProcessObservation::Unknown);
    }
    let pid = i32::try_from(pid).ok().filter(|pid| *pid > 0)?;
    let observation =
        platform::info(pid).and_then(|(seconds, state, _)| observation(pid as u64, seconds, state));
    // A late read is never current evidence. No background work survives it.
    if Instant::now() >= deadline {
        Some(ProcessObservation::Unknown)
    } else {
        observation
    }
}

pub(super) fn parent(pid: u64, deadline: Instant) -> Option<u64> {
    if Instant::now() >= deadline {
        return None;
    }
    let pid = i32::try_from(pid).ok().filter(|pid| *pid > 0)?;
    let parent = platform::parent(pid)?;
    (Instant::now() < deadline && parent > 0 && parent <= i32::MAX as u64).then_some(parent)
}

fn observation(pid: u64, seconds: u64, state: u8) -> Option<ProcessObservation> {
    let date = time::OffsetDateTime::from_unix_timestamp(i64::try_from(seconds).ok()?).ok()?;
    if !(1000..=9999).contains(&date.year()) {
        return None;
    }
    let weekday = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
        [date.weekday().number_days_from_monday() as usize];
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][date.month() as usize - 1];
    // ps's whitespace is normalized by the existing parser: days have no padding.
    let token = format!(
        "ps-v1:{weekday} {month} {} {:02}:{:02}:{:02} {}",
        date.day(),
        date.hour(),
        date.minute(),
        date.second(),
        date.year()
    );
    let incarnation = ProcessIncarnation::new(pid, &token).ok()?;
    Some(match state {
        b'Z' => ProcessObservation::UnreapedZombie(incarnation),
        b'T' | b't' => ProcessObservation::Stopped(incarnation),
        b'R' | b'S' | b'I' | b'D' | b'U' => ProcessObservation::Live(incarnation),
        _ => ProcessObservation::Unknown,
    })
}

#[cfg(target_os = "macos")]
mod platform {
    use libproc::{bsd_info::BSDInfo, proc_pid::pidinfo};

    pub(super) fn parent(pid: i32) -> Option<u64> {
        Some(u64::from(pidinfo::<BSDInfo>(pid, 0).ok()?.pbi_ppid))
    }

    pub(super) fn info(pid: i32) -> Option<(u64, u8, u64)> {
        let info = pidinfo::<BSDInfo>(pid, 0).ok()?;
        if info.pbi_pid != pid as u32 || info.pbi_start_tvsec == 0 {
            return None;
        }
        // Darwin sys/proc.h states; proc_pidinfo returns the same BSD process
        // start timeval used by ps. Subseconds deliberately remain discarded.
        let state = match info.pbi_status {
            1 => return None, // SIDL: defer process creation to ps
            2 => b'R',        // SRUN
            3 => b'S',        // SSLEEP
            4 => b'T',        // SSTOP
            5 => b'Z',        // SZOMB
            _ => return None,
        };
        Some((info.pbi_start_tvsec, state, u64::from(info.pbi_ppid)))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use nix::unistd::{SysconfVar, sysconf};
    use std::path::Path;

    fn stat(pid: i32) -> Option<(u8, u64, u64)> {
        let bytes =
            crate::bounded_file::read_no_follow(Path::new(&format!("/proc/{pid}/stat")), 8192)
                .ok()?;
        parse_stat(pid, &bytes)
    }

    pub(super) fn parent(pid: i32) -> Option<u64> {
        stat(pid).map(|(_, _, parent)| parent)
    }

    pub(super) fn info(pid: i32) -> Option<(u64, u8, u64)> {
        let (state, ticks, parent) = stat(pid)?;
        let hz = u64::try_from(sysconf(SysconfVar::CLK_TCK).ok()??)
            .ok()
            .filter(|hz| *hz > 0)?;
        // Re-read boot time, as ps does, rather than caching across clock changes.
        let bytes =
            crate::bounded_file::read_no_follow(Path::new("/proc/stat"), 1024 * 1024).ok()?;
        let boot = std::str::from_utf8(&bytes)
            .ok()?
            .lines()
            .find_map(|line| line.strip_prefix("btime "))?
            .trim()
            .parse::<u64>()
            .ok()?;
        Some((boot.checked_add(ticks / hz)?, state, parent))
    }

    pub(super) fn parse_stat(pid: i32, bytes: &[u8]) -> Option<(u8, u64, u64)> {
        // comm can contain spaces, newlines and parentheses. The final ')' is
        // its delimiter; the numeric suffix cannot contain one.
        let open = bytes.iter().position(|b| *b == b'(')?;
        let close = bytes.iter().rposition(|b| *b == b')')?;
        if open >= close
            || std::str::from_utf8(&bytes[..open])
                .ok()?
                .trim()
                .parse::<i32>()
                .ok()?
                != pid
        {
            return None;
        }
        let mut fields = bytes
            .get(close + 1..)?
            .split(|b| b.is_ascii_whitespace())
            .filter(|f| !f.is_empty());
        let state = fields.next()?;
        if state.len() != 1 {
            return None;
        }
        // Suffix begins at field 3 (state); starttime is field 22.
        let parent = std::str::from_utf8(fields.next()?).ok()?.parse().ok()?;
        let ticks = std::str::from_utf8(fields.nth(17)?).ok()?.parse().ok()?;
        Some((state[0], ticks, parent))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    pub(super) fn parent(_: i32) -> Option<u64> {
        None
    }
    pub(super) fn info(_: i32) -> Option<(u64, u8, u64)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{UnixCommandRunner, runtime::observe_start};
    use std::time::Duration;

    #[test]
    fn native_start_and_parent_equal_the_existing_ps_identity() {
        let pid = u64::from(std::process::id());
        let deadline = Instant::now() + Duration::from_secs(5);
        let ProcessObservation::Live(native) = observe(pid, deadline).expect("native acquisition")
        else {
            panic!("test process is live");
        };
        assert_eq!(
            Some(native.start_identity()),
            observe_start(&UnixCommandRunner, pid, deadline)
                .unwrap()
                .as_deref()
        );
        assert_eq!(
            parent(pid, deadline),
            Some(nix::unistd::getppid().as_raw() as u64)
        );
    }

    #[test]
    fn stopped_and_exited_owned_children_keep_batch_parity() {
        use crate::process::{
            CommandError, CommandOutput, CommandRequest, CommandRunner, runtime::observe_starts,
        };
        use nix::{
            sys::{
                signal::{Signal, kill},
                wait::{WaitPidFlag, WaitStatus, waitpid},
            },
            unistd::Pid,
        };
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        struct Ps;
        impl CommandRunner for Ps {
            fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
                UnixCommandRunner.execute(request)
            }
        }
        let mut child = Child(
            std::process::Command::new("/bin/sleep")
                .arg("60")
                .spawn()
                .unwrap(),
        );
        let pid = u64::from(child.0.id());
        let raw = Pid::from_raw(child.0.id() as i32);
        kill(raw, Signal::SIGSTOP).unwrap();
        assert!(matches!(
            waitpid(raw, Some(WaitPidFlag::WUNTRACED)).unwrap(),
            WaitStatus::Stopped(_, Signal::SIGSTOP)
        ));
        let deadline = Instant::now() + Duration::from_secs(5);
        let native = observe_starts(&UnixCommandRunner, &[pid], deadline).unwrap();
        assert_eq!(
            native.len(),
            1,
            "stopped process still has a start identity"
        );
        assert_eq!(native, observe_starts(&Ps, &[pid], deadline).unwrap());
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        assert!(
            observe_starts(&UnixCommandRunner, &[pid], deadline)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn utc_tokens_and_states_preserve_the_ps_contract() {
        // 2000-01-01 00:00:00 UTC: single-digit day, leap-century year.
        let live = observation(42, 946684800, b'S').unwrap();
        let ProcessObservation::Live(key) = live else {
            panic!("live")
        };
        assert_eq!(key.start_identity(), "ps-v1:Sat Jan 1 00:00:00 2000");
        assert_eq!(
            observation(42, 946684800, b'T'),
            Some(ProcessObservation::Stopped(key.clone()))
        );
        assert_eq!(
            observation(42, 946684800, b'Z'),
            Some(ProcessObservation::UnreapedZombie(key))
        );
        assert_eq!(
            observation(42, 946684800, b'?'),
            Some(ProcessObservation::Unknown)
        );
        assert_eq!(observation(42, u64::MAX, b'S'), None);
        assert_eq!(parent(42, Instant::now()), None);
        assert_eq!(
            observe(42, Instant::now()),
            Some(ProcessObservation::Unknown)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn proc_stat_parses_comm_delimiters_and_rejects_incomplete_evidence() {
        let stat = b"42 (odd ) name\n(x)) S 7 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 12345 0";
        assert_eq!(platform::parse_stat(42, stat), Some((b'S', 12345, 7)));
        assert_eq!(platform::parse_stat(43, stat), None);
        assert_eq!(platform::parse_stat(42, b"42 (comm) S 7"), None);
    }
}
