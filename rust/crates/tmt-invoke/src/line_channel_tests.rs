use super::*;
use crate::{
    Cleanup,
    tests::{Fixture, START_BUDGET, gone, ready},
};
use std::{
    fs,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

fn open(script: &str, limit: usize) -> (Fixture, LineChannel) {
    let fixture = Fixture::new(&format!("echo $$ > pid\n{script}"));
    let args = [fixture.program().into_os_string()];
    let channel = LineChannel::start(
        LineChannelOptions {
            program: Path::new("/bin/sh"),
            args: &args,
            deadline: Instant::now() + START_BUDGET,
            max_frame_bytes: limit,
            launch: Default::default(),
        },
        || false,
    )
    .unwrap();
    (fixture, channel)
}

fn exchange(channel: &mut LineChannel, payload: &[u8]) -> Result<Vec<u8>, InvokeError> {
    channel.exchange(payload, Instant::now() + START_BUDGET, || false)
}

#[test]
fn several_byte_frames_use_one_child_and_clean_eof_reaps_it() {
    let (fixture, mut channel) = open("printf diagnostic >&2; cat; printf done > eof", 64);
    assert_eq!(exchange(&mut channel, b"first").unwrap(), b"first");
    let pid = fs::read(fixture.0.join("pid")).unwrap();
    assert_eq!(
        exchange(&mut channel, b"\0second\xff").unwrap(),
        b"\0second\xff"
    );
    assert_eq!(fs::read(fixture.0.join("pid")).unwrap(), pid);
    let output = channel
        .close(Instant::now() + START_BUDGET, || false)
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"diagnostic");
    assert_eq!(fs::read(fixture.0.join("eof")).unwrap(), b"done");
    gone(&fixture.0.join("pid"));
}

#[test]
fn local_frame_rejection_keeps_the_unmodified_channel_usable() {
    let (fixture, mut channel) = open("cat", 4);
    for input in [b"a\nb".as_slice(), b"12345"] {
        let error = exchange(&mut channel, input).unwrap_err();
        assert!(matches!(error.cleanup, Cleanup::NotStarted));
        assert_eq!(error.cause.unwrap().kind(), io::ErrorKind::InvalidInput);
    }
    assert_eq!(
        exchange(&mut channel, b"four").unwrap(),
        b"four",
        "delimiter is outside the byte limit"
    );
    assert!(
        channel
            .close(Instant::now() + START_BUDGET, || false)
            .unwrap()
            .status
            .success()
    );
    gone(&fixture.0.join("pid"));
}

#[test]
fn incomplete_response_closes_the_child_without_replaying_the_request() {
    let (fixture, mut channel) = open(
        "IFS= read -r line; printf '%s' \"$line\" > seen; printf partial; exit 7",
        64,
    );
    let error = exchange(&mut channel, b"effect").unwrap_err();
    assert!(matches!(error.kind, FailureKind::Io(Phase::Communicate)));
    assert_eq!(error.cause.unwrap().kind(), io::ErrorKind::UnexpectedEof);
    assert!(matches!(error.cleanup, Cleanup::Confirmed));
    let next = exchange(&mut channel, b"another").unwrap_err();
    assert_eq!(next.cause.unwrap().kind(), io::ErrorKind::BrokenPipe);
    assert!(matches!(next.cleanup, Cleanup::NotStarted));
    assert_eq!(fs::read(fixture.0.join("seen")).unwrap(), b"effect");
    gone(&fixture.0.join("pid"));
}

#[test]
fn both_output_streams_have_independent_bounds_and_confirmed_cleanup() {
    for (script, stream) in [
        ("IFS= read -r line; printf '12345\\n'", Stream::Stdout),
        (
            "IFS= read -r line; printf 12345 >&2; printf 'ok\\n'",
            Stream::Stderr,
        ),
    ] {
        let (fixture, mut channel) = open(script, 4);
        let error = exchange(&mut channel, b"go").unwrap_err();
        assert_eq!(error.kind, FailureKind::OutputLimit(stream));
        assert!(matches!(error.cleanup, Cleanup::Confirmed));
        gone(&fixture.0.join("pid"));
    }
}

#[test]
fn stderr_is_bounded_across_exchanges() {
    let (fixture, mut channel) = open(
        "while IFS= read -r line; do printf xxx >&2; printf '%s\\n' \"$line\"; done",
        4,
    );
    assert_eq!(exchange(&mut channel, b"one").unwrap(), b"one");
    let error = exchange(&mut channel, b"two").unwrap_err();
    assert_eq!(error.kind, FailureKind::OutputLimit(Stream::Stderr));
    assert!(matches!(error.cleanup, Cleanup::Confirmed));
    gone(&fixture.0.join("pid"));
}

#[test]
fn cancellation_after_request_admission_ends_the_channel() {
    let (fixture, mut channel) = open(
        "IFS= read -r line; printf '%s' \"$line\" > seen; echo ready > ready; cat > /dev/null",
        64,
    );
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            ready(&fixture.0.join("ready"));
            stop.store(true, Ordering::Release);
        });
        let error = channel
            .exchange(b"effect", Instant::now() + START_BUDGET, || {
                stop.load(Ordering::Acquire)
            })
            .unwrap_err();
        assert_eq!(error.kind, FailureKind::Interrupted);
        assert!(matches!(error.cleanup, Cleanup::Confirmed));
    });
    assert_eq!(fs::read(fixture.0.join("seen")).unwrap(), b"effect");
    gone(&fixture.0.join("pid"));
}

#[test]
fn a_response_deadline_after_readiness_ends_the_owned_child() {
    let (fixture, mut channel) = open(
        "IFS= read -r line; printf 'ready\\n'; IFS= read -r line; printf '%s' \"$line\" > seen; cat > /dev/null",
        64,
    );
    assert_eq!(exchange(&mut channel, b"start").unwrap(), b"ready");
    let error = channel
        .exchange(b"effect", Instant::now() + Duration::from_secs(1), || false)
        .unwrap_err();
    assert_eq!(error.kind, FailureKind::Deadline);
    assert!(matches!(error.cleanup, Cleanup::Confirmed));
    assert_eq!(fs::read(fixture.0.join("seen")).unwrap(), b"effect");
    gone(&fixture.0.join("pid"));
}

#[test]
fn drop_closes_stdin_and_preserves_the_childs_eof_cleanup() {
    let (fixture, mut channel) = open("cat; printf done > eof", 64);
    assert_eq!(exchange(&mut channel, b"hello").unwrap(), b"hello");
    drop(channel);
    assert_eq!(fs::read(fixture.0.join("eof")).unwrap(), b"done");
    gone(&fixture.0.join("pid"));
}
