use super::*;
use nix::unistd::pipe;
use std::{cell::Cell, fs::File, io::Write, os::fd::OwnedFd, rc::Rc};

/// A manual monotonic clock. `jump_after_next_clock_read` moves it once the reader
/// has read the clock, so a stall reaches its deadline without a real five-second wait.
#[derive(Clone)]
struct Clock {
    now: Rc<Cell<Instant>>,
    jump: Rc<Cell<Option<Duration>>>,
}
impl Clock {
    fn new() -> Self {
        Self {
            now: Rc::new(Cell::new(Instant::now())),
            jump: Rc::new(Cell::new(None)),
        }
    }
    fn advance(&self, by: Duration) {
        self.now.set(self.now.get() + by);
    }
    fn jump_after_next_clock_read(&self, by: Duration) {
        self.jump.set(Some(by));
    }
    fn reader<'a>(
        &self,
        input: &'a OwnedFd,
        maximum: usize,
    ) -> Reader<'a, OwnedFd, impl Fn() -> Instant> {
        let clock = self.clone();
        Reader::with_clock(input, maximum, move || {
            let now = clock.now.get();
            if let Some(by) = clock.jump.take() {
                clock.advance(by);
            }
            now
        })
    }
}

fn channel() -> (OwnedFd, File) {
    let (input, client) = pipe().unwrap();
    (input, File::from(client))
}

const STEP: Duration = Duration::from_secs(4);

#[test]
fn pipelined_frames_keep_flowing_and_each_gets_its_own_interval() {
    // Detects a leftover partial frame that inherits an earlier frame's deadline.
    let clock = Clock::new();
    let (input, mut client) = channel();
    let mut reader = clock.reader(&input, 64);
    client.write_all(b"0\n1").unwrap();
    assert_eq!(reader.read_frame().unwrap().unwrap(), b"0\n");
    // Every frame finishes 4 s after the previous boundary, so the first
    // frame's 5 s deadline passes at the second step.
    for frame in 1..8 {
        clock.advance(STEP);
        client
            .write_all(format!("\n{}", frame + 1).as_bytes())
            .unwrap();
        assert_eq!(
            reader.read_frame().unwrap().unwrap(),
            format!("{frame}\n").as_bytes()
        );
    }
    // Time spent by the handler between frames is not acquisition time.
    clock.advance(IO_TIMEOUT + STEP);
    client.write_all(b"\n").unwrap();
    assert_eq!(reader.read_frame().unwrap().unwrap(), b"8\n");
    drop(client);
    assert!(reader.read_frame().unwrap().is_none());
}

#[test]
fn a_stalled_leftover_frame_still_times_out_at_its_own_bound() {
    let clock = Clock::new();
    let (input, mut client) = channel();
    let mut reader = clock.reader(&input, 64);
    client.write_all(b"0\n1").unwrap();
    assert_eq!(reader.read_frame().unwrap().unwrap(), b"0\n");
    clock.advance(STEP);
    // Closing the writer first makes a leftover without a deadline fail fast
    // with UnexpectedEof instead of blocking in poll.
    drop(client);
    // The leftover's interval starts at this clock read; every later read is at its end.
    clock.jump_after_next_clock_read(IO_TIMEOUT);
    let error = reader.read_frame().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}

#[test]
fn a_leftover_frame_is_accepted_just_inside_its_own_bound() {
    // Positive control for the stall: the same leftover 1 ms earlier completes.
    let clock = Clock::new();
    let (input, mut client) = channel();
    let mut reader = clock.reader(&input, 64);
    client.write_all(b"0\n1").unwrap();
    assert_eq!(reader.read_frame().unwrap().unwrap(), b"0\n");
    clock.advance(STEP);
    client.write_all(b"\n").unwrap();
    clock.jump_after_next_clock_read(IO_TIMEOUT - Duration::from_millis(1));
    assert_eq!(reader.read_frame().unwrap().unwrap(), b"1\n");
}

#[test]
fn frame_bounds_and_eof_keep_their_meaning() {
    let clock = Clock::new();
    let (input, mut client) = channel();
    let mut reader = clock.reader(&input, 4);
    client.write_all(b"1234\n12345\n").unwrap();
    assert_eq!(reader.read_frame().unwrap().unwrap(), b"1234\n");
    assert_eq!(
        reader.read_frame().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );

    let (input, mut client) = channel();
    let mut reader = clock.reader(&input, 4);
    client.write_all(b"1\n2").unwrap();
    drop(client);
    assert_eq!(reader.read_frame().unwrap().unwrap(), b"1\n");
    assert_eq!(
        reader.read_frame().unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );

    let (input, client) = channel();
    let mut reader = clock.reader(&input, 4);
    drop(client);
    assert!(reader.read_frame().unwrap().is_none());
}
