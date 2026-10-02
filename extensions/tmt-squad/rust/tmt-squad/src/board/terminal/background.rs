//! Board-owned background query, after raw mode and before input starts.
//! Startup input received during the query (at most 100 ms) is discarded.
//! Late OSC replies are filtered from crossterm events, never board actions.

use nix::{
    fcntl::OFlag,
    sys::{
        select::{FdSet, select},
        time::{TimeVal, TimeValLike},
    },
};
use ratatui::crossterm::event::{Event, KeyCode, KeyModifiers};
use std::{
    fs::OpenOptions,
    io::{self, Read, Write},
    os::{fd::AsFd, unix::fs::OpenOptionsExt},
    time::{Duration, Instant},
};
use tmt_cli_style::{
    Base, Interaction, Mode,
    theme::background::{self, Background, QUERY_TIMEOUT, QueryReply},
};

pub(in crate::board) fn allowed(
    base: Base,
    interaction: Interaction,
    color: bool,
    no_color: bool,
) -> bool {
    base == Base::Auto && interaction.view() == Mode::Interactive && color && !no_color
}

/// Resolve observation order without touching the terminal for a known signal
/// or an ineligible view. The caller supplies the existing screen owner.
pub(in crate::board) fn observe(
    value: Option<&str>,
    eligible: bool,
    query: impl FnOnce() -> io::Result<QueryReply>,
) -> (Option<Background>, Option<QueryReply>) {
    let signal = value.and_then(background::colorfgbg);
    if signal.is_some() || !eligible {
        return (signal, None);
    }
    let reply = query().ok();
    (reply.as_ref().and_then(|reply| reply.background), reply)
}

// Darwin's poll reports POLLNVAL for /dev/tty; select supports terminal fds.
fn ready(file: &std::fs::File, write: bool, remaining: Duration) -> io::Result<bool> {
    let mut fds = FdSet::new();
    fds.insert(file.as_fd());
    let micros = i64::try_from(remaining.as_micros()).map_err(io::Error::other)?;
    let mut timeout = TimeVal::microseconds(micros);
    let count = if write {
        select(None, None, Some(&mut fds), None, Some(&mut timeout))
    } else {
        select(None, Some(&mut fds), None, None, Some(&mut timeout))
    }
    .map_err(io::Error::other)?;
    Ok(count > 0 && fds.contains(file.as_fd()))
}

/// The existing Screen owner calls this only after entering raw mode.
/// Nonblocking terminal I/O cannot leave a reader thread behind on timeout.
pub(in crate::board) fn query() -> io::Result<QueryReply> {
    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(OFlag::O_NONBLOCK.bits())
        .open("/dev/tty")?;
    let started = Instant::now();
    let mut request = b"\x1b]11;?\x07".as_slice();
    while !request.is_empty() {
        let Some(remaining) = QUERY_TIMEOUT
            .checked_sub(started.elapsed())
            .filter(|remaining| !remaining.is_zero())
        else {
            return Ok(QueryReply {
                background: None,
                received: Vec::new(),
                reply: None,
            });
        };
        if !ready(&tty, true, remaining)? {
            break;
        }
        match tty.write(request) {
            Ok(0) => break,
            Ok(count) => request = &request[count..],
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    if !request.is_empty() {
        return Ok(QueryReply {
            background: None,
            received: Vec::new(),
            reply: None,
        });
    }
    Ok(background::read_osc11(
        Duration::ZERO,
        || started.elapsed(),
        |remaining, buffer| {
            if !ready(&tty, false, remaining)? {
                return Ok(0);
            }
            tty.read(buffer)
        },
    ))
}

/// Crossterm exposes OSC as Alt-] followed by key events. Hold that specific
/// prefix until it is recognized; mismatches are forwarded unchanged. A known
/// OSC 11 frame is discarded through BEL/ST, with a bounded inactivity window.
/// This filter is enabled only when we actually sent the background query.
#[derive(Default)]
pub(in crate::board) struct ReplyFilter {
    pending: Vec<Event>,
    prefix: usize,
    payload: bool,
    last: Option<Instant>,
}

const PREFIX: &str = "11;rgb:";

impl ReplyFilter {
    /// Seed a partial reply consumed by the startup query. Other startup bytes
    /// are deliberately dropped; they never get synthesized into board keys.
    pub(in crate::board) fn seed(bytes: &[u8], now: Instant) -> Self {
        let mut filter = Self::default();
        if let Some(start) = bytes.windows(2).rposition(|bytes| bytes == b"\x1b]") {
            let tail = &bytes[start + 2..];
            if tail.starts_with(PREFIX.as_bytes())
                && !tail.contains(&7)
                && !tail.windows(2).any(|bytes| bytes == b"\x1b\\")
            {
                filter.payload = true;
                filter.last = Some(now);
            } else if PREFIX.as_bytes().starts_with(tail) {
                filter.prefix = tail.len();
                filter.last = Some(now);
            }
        }
        filter
    }

    pub(in crate::board) fn expire(&mut self, now: Instant) -> Vec<Event> {
        if self
            .last
            .is_some_and(|last| now.duration_since(last) >= QUERY_TIMEOUT)
        {
            self.last = None;
            self.prefix = 0;
            self.payload = false;
            return std::mem::take(&mut self.pending);
        }
        Vec::new()
    }

    pub(in crate::board) fn push(&mut self, event: Event, now: Instant) -> Vec<Event> {
        let mut output = self.expire(now);
        let Event::Key(key) = &event else {
            output.push(event);
            return output;
        };
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let start = alt && key.code == KeyCode::Char(']');
        if self.payload {
            let end = (key.code == KeyCode::Char('g')
                && key.modifiers.contains(KeyModifiers::CONTROL))
                || (alt && key.code == KeyCode::Char('\\'));
            if end {
                self.payload = false;
                self.last = None;
            } else {
                self.last = Some(now);
            }
            return output;
        }
        if self.last.is_some() {
            let expected = PREFIX.as_bytes()[self.prefix];
            if key.code == KeyCode::Char(char::from(expected)) && key.modifiers.is_empty() {
                self.prefix += 1;
                self.pending.push(event);
                self.last = Some(now);
                if self.prefix == PREFIX.len() {
                    self.pending.clear();
                    self.prefix = 0;
                    self.payload = true;
                }
                return output;
            }
            output.append(&mut self.pending);
            self.last = None;
            self.prefix = 0;
        }
        if start {
            self.pending.push(event);
            self.last = Some(now);
        } else {
            output.push(event);
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEvent;
    const PERSON: Interaction = Interaction {
        json: false,
        stdin: true,
        stdout: true,
        stderr: true,
        dumb: false,
    };
    #[test]
    fn query_requires_an_auto_interactive_colored_view() {
        assert!(allowed(Base::Auto, PERSON, true, false));
        for base in [Base::Tmt, Base::TmtLight, Base::Terminal, Base::Mono] {
            assert!(!allowed(base, PERSON, true, false));
        }
        for interaction in [
            Interaction {
                json: true,
                ..PERSON
            },
            Interaction {
                stdin: false,
                ..PERSON
            },
            Interaction {
                stdout: false,
                ..PERSON
            },
            Interaction {
                dumb: true,
                ..PERSON
            },
        ] {
            assert!(!allowed(Base::Auto, interaction, true, false));
        }
        assert!(!allowed(Base::Auto, PERSON, false, false));
        assert!(!allowed(Base::Auto, PERSON, true, true));
    }
    #[test]
    fn colorfgbg_precedes_the_query_and_plain_paths_never_read() {
        let fail = || -> io::Result<QueryReply> { panic!("must not query") };
        assert_eq!(
            observe(Some("15;default;7"), true, fail).0,
            Some(Background::Light)
        );
        assert_eq!(observe(None, false, fail), (None, None));
        let mut calls = 0;
        let (signal, _) = observe(Some("bad"), true, || {
            calls += 1;
            Ok(QueryReply {
                background: Some(Background::Dark),
                received: Vec::new(),
                reply: None,
            })
        });
        assert_eq!(calls, 1);
        assert_eq!(signal, Some(Background::Dark));
        assert_eq!(
            observe(None, true, || Err(io::Error::other("terminal absent"))),
            (None, None)
        );
    }
    fn key(c: char, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), modifiers))
    }
    #[test]
    fn late_replies_never_become_board_keys_and_unrelated_keys_survive() {
        let now = Instant::now();
        for end in [
            key('g', KeyModifiers::CONTROL),
            key('\\', KeyModifiers::ALT),
        ] {
            let mut filter = ReplyFilter::default();
            assert!(filter.push(key(']', KeyModifiers::ALT), now).is_empty());
            for c in "11;rgb:ffff/ffff/ffff".chars() {
                assert!(filter.push(key(c, KeyModifiers::NONE), now).is_empty());
            }
            assert!(filter.push(end, now).is_empty());
            let q = key('q', KeyModifiers::NONE);
            assert_eq!(filter.push(q.clone(), now), vec![q]);
        }
    }
    #[test]
    fn false_prefix_and_incomplete_prefix_replay_unchanged() {
        let now = Instant::now();
        let alt = key(']', KeyModifiers::ALT);
        let q = key('q', KeyModifiers::NONE);
        let mut filter = ReplyFilter::default();
        assert!(filter.push(alt.clone(), now).is_empty());
        assert_eq!(filter.push(q.clone(), now), vec![alt.clone(), q]);
        filter.push(alt.clone(), now);
        assert_eq!(filter.expire(now + QUERY_TIMEOUT), vec![alt]);
    }
    #[test]
    fn partial_startup_reply_is_discarded_and_truncation_does_not_capture_future_keys() {
        let now = Instant::now();
        let mut filter = ReplyFilter::seed(b"q\x1b]11;rgb:ff/ff", now);
        for c in "/ff".chars() {
            assert!(filter.push(key(c, KeyModifiers::NONE), now).is_empty());
        }
        assert!(filter.push(key('g', KeyModifiers::CONTROL), now).is_empty());
        let q = key('q', KeyModifiers::NONE);
        assert_eq!(filter.push(q.clone(), now), vec![q.clone()]);
        let mut filter = ReplyFilter::seed(b"\x1b]11;rgb:", now);
        assert_eq!(filter.push(q.clone(), now + QUERY_TIMEOUT), vec![q]);
    }
}
