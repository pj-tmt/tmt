//! Keyboard and coordinator events share the terminal's existing wait.
use serde_json::Value;
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    os::{fd::AsFd, unix::net::UnixStream},
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) enum Answer {
    Keyboard(bool),
    Coordinator(Value),
}

pub(super) fn read_event(
    reader: &mut BufReader<UnixStream>,
    frame: &mut Vec<u8>,
) -> io::Result<Option<Value>> {
    loop {
        let bytes = match reader.fill_buf() {
            Ok([]) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(bytes) => bytes,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::Interrupted
                ) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let end = bytes.iter().position(|byte| *byte == b'\n');
        let count = end.map_or(bytes.len(), |end| end + 1);
        if frame.len() + count > tmt_remote::limits::CLOUD_PAIR_CONTROL_BYTES {
            return Err(io::ErrorKind::InvalidData.into());
        }
        frame.extend_from_slice(&bytes[..count]);
        reader.consume(count);
        if end.is_some() {
            let value = serde_json::from_slice(frame).map_err(|_| io::ErrorKind::InvalidData)?;
            frame.clear();
            return Ok(Some(value));
        }
    }
}

pub(super) fn confirm(
    input: &mut (impl Read + AsFd),
    output: &mut impl Write,
    stop: &AtomicBool,
    prompt: &str,
    reader: &mut BufReader<UnixStream>,
) -> io::Result<Answer> {
    use nix::{
        errno::Errno,
        poll::{PollFd, PollFlags, poll},
    };
    write!(output, "{prompt} ")?;
    output.flush()?;
    let mut frame = Vec::new();
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(Answer::Keyboard(false));
        }
        // BufReader may already own the browser decision after the candidate.
        let (coordinator, keyboard) = if !reader.buffer().is_empty() {
            (true, false)
        } else {
            let mut fds = [
                PollFd::new(reader.get_ref().as_fd(), PollFlags::POLLIN),
                PollFd::new(input.as_fd(), PollFlags::POLLIN),
            ];
            match poll(&mut fds, tmt_remote::limits::DEPLOY_CONFIRM_POLL_MS) {
                Ok(0) | Err(Errno::EINTR) => continue,
                Err(error) => return Err(io::Error::from_raw_os_error(error as i32)),
                Ok(_) => {}
            }
            (
                fds[0].revents().is_some_and(|events| !events.is_empty()),
                fds[1].revents().is_some_and(|events| !events.is_empty()),
            )
        };
        if coordinator && let Some(event) = read_event(reader, &mut frame)? {
            if event["event"] == "waiting" {
                continue;
            }
            writeln!(output)?;
            return Ok(Answer::Coordinator(event));
        }
        if keyboard {
            let mut bytes = [0; tmt_remote::limits::DEPLOY_CONFIRM_BYTES];
            let count = input.read(&mut bytes)?;
            return Ok(Answer::Keyboard(
                !stop.load(Ordering::Acquire)
                    && count < bytes.len()
                    && tmt_remote::deploy_cli::confirmation_answer(&bytes[..count]),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{sync::mpsc, thread, time::Duration};

    struct Prompt(mpsc::Sender<()>);
    impl Write for Prompt {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.send(()).unwrap();
            Ok(())
        }
    }

    #[test]
    fn browser_decision_interrupts_each_default_no_prompt_without_keyboard_input() {
        for prompt in ["Do the words match? [y/N]", "Allow sending? [y/N]"] {
            let (mut input, _held_keyboard) = UnixStream::pair().unwrap();
            let (socket, mut browser) = UnixStream::pair().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
            let (sent, ready) = mpsc::channel();
            let join = thread::spawn(move || {
                ready.recv_timeout(Duration::from_secs(2)).unwrap();
                writeln!(browser, "{}", json!({"event":"waiting"})).unwrap();
                writeln!(
                    browser,
                    "{}",
                    json!({"event":"owner-decision","outcome":"confirmed","talk":false})
                )
                .unwrap();
            });
            let answer = confirm(
                &mut input,
                &mut Prompt(sent),
                &AtomicBool::new(false),
                prompt,
                &mut BufReader::new(socket),
            )
            .unwrap();
            join.join().unwrap();
            assert!(
                matches!(answer, Answer::Coordinator(event) if event["outcome"] == "confirmed" && event["talk"] == false)
            );
        }
    }

    #[test]
    fn buffered_decision_precedes_ready_keyboard_and_no_answer_defaults_to_no() {
        let (mut input, mut keyboard) = UnixStream::pair().unwrap();
        let (socket, mut browser) = UnixStream::pair().unwrap();
        writeln!(keyboard, "yes").unwrap();
        writeln!(
            browser,
            "{}",
            json!({"event":"owner-decision","outcome":"refused"})
        )
        .unwrap();
        let mut reader = BufReader::new(socket);
        assert!(!reader.fill_buf().unwrap().is_empty());
        let answer = confirm(
            &mut input,
            &mut Vec::new(),
            &AtomicBool::new(false),
            "Match?",
            &mut reader,
        )
        .unwrap();
        assert!(matches!(answer, Answer::Coordinator(event) if event["outcome"] == "refused"));
        drop(keyboard);
        let mut byte = [0; 4];
        assert_eq!(input.read(&mut byte).unwrap(), 4); // browser consumed no consent
        assert_eq!(input.read(&mut byte).unwrap(), 0);
        let answer = confirm(
            &mut input,
            &mut Vec::new(),
            &AtomicBool::new(false),
            "Match?",
            &mut reader,
        );
        // The socket remains open without events: EOF on the keyboard is default-no.
        assert!(matches!(answer.unwrap(), Answer::Keyboard(false)));
    }

    #[test]
    fn malformed_oversized_or_disconnected_coordinator_never_becomes_consent() {
        for bytes in [
            b"not json\n".to_vec(),
            vec![b'x'; tmt_remote::limits::CLOUD_PAIR_CONTROL_BYTES + 1],
            Vec::new(),
        ] {
            let (mut input, _keyboard) = UnixStream::pair().unwrap();
            let (socket, mut browser) = UnixStream::pair().unwrap();
            let join = thread::spawn(move || {
                let _ = browser.write_all(&bytes);
            });
            let answer = confirm(
                &mut input,
                &mut Vec::new(),
                &AtomicBool::new(false),
                "Match?",
                &mut BufReader::new(socket),
            );
            join.join().unwrap();
            assert!(answer.is_err());
        }
    }
}
