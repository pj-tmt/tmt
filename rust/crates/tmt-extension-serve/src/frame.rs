//! Private startup records: a tag, a four-byte length and a JSON payload, read under one absolute
//! deadline so a byte-at-a-time peer cannot renew the budget.
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use serde_json::Value;
use std::{
    io::{Read, Write},
    os::{fd::AsFd, unix::net::UnixStream},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub(crate) const PULSE: Duration = Duration::from_millis(20);
pub(crate) const HEADER: usize = 5;
pub const READY: u8 = 1;
pub const FAILED: u8 = 2;
pub const ACCEPT: u8 = 3;
pub const ACCEPTED: u8 = 4;
pub const CANCEL: u8 = 5;

/// A record that could not be exchanged intact in time.
#[derive(Debug)]
pub struct Broken;

/// Absolute frame deadline, including partial headers and payloads.
pub(crate) fn read_exact_until(
    stream: &mut UnixStream,
    bytes: &mut [u8],
    deadline: Instant,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        if stop.load(Ordering::SeqCst) || Instant::now() >= deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .min(PULSE);
        let mut descriptors = [PollFd::new(stream.as_fd(), PollFlags::POLLIN)];
        match poll(
            &mut descriptors,
            PollTimeout::try_from(remaining).unwrap_or(PollTimeout::MAX),
        ) {
            Ok(0) | Err(nix::errno::Errno::EINTR) => continue,
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        match stream.read(&mut bytes[offset..]) {
            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => offset += n,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub fn write_frame(
    stream: &mut UnixStream,
    tag: u8,
    value: &Value,
    limit: usize,
) -> Result<(), Broken> {
    let payload = serde_json::to_vec(value).map_err(|_| Broken)?;
    if payload.len() > limit - HEADER {
        return Err(Broken);
    }
    let mut frame = Vec::with_capacity(HEADER + payload.len());
    frame.push(tag);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(&payload);
    stream.write_all(&frame).map_err(|_| Broken)
}

pub fn read_frame(
    stream: &mut UnixStream,
    deadline: Instant,
    stop: &AtomicBool,
    limit: usize,
) -> Result<(u8, Value), Broken> {
    let mut header = [0; HEADER];
    read_exact_until(stream, &mut header, deadline, stop).map_err(|_| Broken)?;
    if !matches!(header[0], READY | FAILED) {
        return Err(Broken);
    }
    let length = u32::from_be_bytes(header[1..].try_into().expect("four bytes")) as usize;
    if length == 0 || length > limit - HEADER {
        return Err(Broken);
    }
    let mut payload = vec![0; length];
    read_exact_until(stream, &mut payload, deadline, stop).map_err(|_| Broken)?;
    Ok((
        header[0],
        serde_json::from_slice(&payload).map_err(|_| Broken)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_length_and_partial_frame_obey_one_absolute_deadline() {
        let (mut parent, mut worker) = UnixStream::pair().unwrap();
        parent.write_all(&[READY, 0, 0, 16, 1]).unwrap();
        assert!(
            read_frame(
                &mut worker,
                Instant::now() + Duration::from_secs(1),
                &AtomicBool::new(false),
                4096,
            )
            .is_err()
        );
        parent.write_all(&[READY, 0]).unwrap();
        let start = Instant::now();
        assert!(
            read_frame(
                &mut worker,
                start + Duration::from_millis(30),
                &AtomicBool::new(false),
                4096,
            )
            .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn an_oversized_or_unknown_record_is_refused() {
        let (mut parent, mut worker) = UnixStream::pair().unwrap();
        let big = serde_json::json!({"text": "x".repeat(5000)});
        assert!(write_frame(&mut parent, READY, &big, 4096).is_err());
        write_frame(&mut parent, READY, &serde_json::json!({"a":1}), 4096).unwrap();
        let (tag, value) = read_frame(
            &mut worker,
            Instant::now() + Duration::from_secs(1),
            &AtomicBool::new(false),
            4096,
        )
        .unwrap();
        assert_eq!((tag, value), (READY, serde_json::json!({"a":1})));
        parent.write_all(&[9, 0, 0, 0, 1, b'{']).unwrap();
        assert!(
            read_frame(
                &mut worker,
                Instant::now() + Duration::from_secs(1),
                &AtomicBool::new(false),
                4096,
            )
            .is_err(),
            "only Ready and Failed are records"
        );
    }
}
