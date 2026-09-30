use super::*;
use crate::test_support::TestDirectory;
use nix::{
    fcntl::{FcntlArg, OFlag, fcntl},
    sys::stat::Mode,
    unistd::{mkfifo, pipe, write},
};
use std::{
    fs::{self, File},
    os::unix::fs::symlink,
};

#[test]
fn bounded_wire_input_keeps_its_own_cap_and_flags() {
    let directory = TestDirectory::new();
    let file = directory.path.join("wire");
    let bytes = vec![b'x'; MAX_EXCHANGE_TEXT_BYTES + 1];
    fs::write(&file, &bytes).unwrap();
    let stream = File::open(&file).unwrap();
    let flags = fcntl(&stream, FcntlArg::F_GETFL).unwrap();
    assert_eq!(
        read_stream_bounded(&stream, Duration::from_secs(2), bytes.len())
            .unwrap()
            .as_bytes(),
        bytes
    );
    assert_eq!(fcntl(&stream, FcntlArg::F_GETFL).unwrap(), flags);
    let stream = File::open(&file).unwrap();
    assert_eq!(
        read_stream_bounded(&stream, Duration::from_secs(2), bytes.len() - 1)
            .unwrap_err()
            .kind,
        ResponseInputFailure::TooLarge
    );
}

#[test]
fn regular_files_preserve_exact_text_and_follow_explicit_symlinks() {
    let directory = TestDirectory::new();
    let file = directory.path.join("body");
    let link = directory.path.join("link");
    symlink(&file, &link).unwrap();
    for text in ["", "\u{feff}résumé\0\r\n尾\n", "   "] {
        fs::write(&file, text).unwrap();
        assert_eq!(file_result(&file).unwrap(), text);
        assert_eq!(file_result(&link).unwrap(), text);
    }
}

#[test]
fn file_failures_reject_nonregular_input_without_waiting_for_a_fifo_writer() {
    let directory = TestDirectory::new();
    let fifo = directory.path.join("fifo");
    mkfifo(&fifo, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
    for file in [directory.path.clone(), directory.path.join("absent"), fifo] {
        assert_eq!(file_result(&file), Err(ResponseInputFailure::File));
    }
}

#[test]
fn file_and_stdin_share_the_byte_cap_and_strict_decoding() {
    let directory = TestDirectory::new();
    let file = directory.path.join("body");
    for (bytes, expected) in [
        (vec![b'x'; MAX_EXCHANGE_TEXT_BYTES], None),
        (
            vec![b'x'; MAX_EXCHANGE_TEXT_BYTES + 1],
            Some(ResponseInputFailure::TooLarge),
        ),
        (vec![0xef, 0xbb], Some(ResponseInputFailure::Invalid)),
        (vec![0xed, 0xa0, 0x80], Some(ResponseInputFailure::Invalid)),
    ] {
        fs::write(&file, &bytes).unwrap();
        let stream = File::open(&file).unwrap();
        let flags = fcntl(&stream, FcntlArg::F_GETFL).unwrap();
        for result in [
            file_result(&file),
            stream_result(&stream, Duration::from_secs(2)),
        ] {
            match expected {
                Some(error) => assert_eq!(result, Err(error)),
                None => assert_eq!(result.unwrap().as_bytes(), bytes),
            }
        }
        assert_eq!(fcntl(&stream, FcntlArg::F_GETFL).unwrap(), flags);
    }
}

#[test]
fn pipe_requires_eof_and_keeps_flags_after_success_and_invalid_utf8() {
    for (bytes, expected) in [
        (b"".as_slice(), Ok("")),
        (
            b"\xef\xbb\xbfbody\0\r\n".as_slice(),
            Ok("\u{feff}body\0\r\n"),
        ),
        (b"\xff".as_slice(), Err(ResponseInputFailure::Invalid)),
    ] {
        let (reader, writer) = pipe().unwrap();
        let flags = fcntl(&reader, FcntlArg::F_GETFL).unwrap();
        for byte in bytes {
            assert_eq!(write(&writer, &[*byte]).unwrap(), 1);
        }
        drop(writer);
        assert_eq!(
            stream_result(&reader, Duration::from_secs(1))
                .as_deref()
                .map_err(|error| *error),
            expected
        );
        assert_eq!(fcntl(&reader, FcntlArg::F_GETFL).unwrap(), flags);
    }
}

#[test]
fn missing_eof_discards_input_and_keeps_both_initial_flag_modes() {
    for input in [b"".as_slice(), b"partial".as_slice()] {
        for nonblocking in [false, true] {
            let (reader, writer) = pipe().unwrap();
            if nonblocking {
                let flags = OFlag::from_bits_retain(fcntl(&reader, FcntlArg::F_GETFL).unwrap());
                fcntl(&reader, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).unwrap();
            }
            let flags = fcntl(&reader, FcntlArg::F_GETFL).unwrap();
            write(&writer, input).unwrap();
            let before = Instant::now();
            assert_eq!(
                stream_result(&reader, Duration::from_millis(20)),
                Err(ResponseInputFailure::Timeout)
            );
            assert!(before.elapsed() < Duration::from_secs(1));
            assert_eq!(fcntl(&reader, FcntlArg::F_GETFL).unwrap(), flags);
            // Still-open writer proves timeout was not an accidental EOF success.
            write(&writer, b"still open").unwrap();
        }
    }
}

#[test]
fn invalid_deadline_does_not_modify_the_descriptor() {
    let (reader, _writer) = pipe().unwrap();
    let flags = fcntl(&reader, FcntlArg::F_GETFL).unwrap();
    assert_eq!(
        stream_result(&reader, Duration::ZERO),
        Err(ResponseInputFailure::Invalid)
    );
    assert_eq!(fcntl(&reader, FcntlArg::F_GETFL).unwrap(), flags);
}

#[test]
fn socket_backed_stdin_accepts_eof_from_a_process_launcher() {
    use std::{io::Write, net::Shutdown, os::unix::net::UnixStream};
    for input in ["", "\u{feff}socket\0\r\n"] {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let flags = fcntl(&reader, FcntlArg::F_GETFL).unwrap();
        writer.write_all(input.as_bytes()).unwrap();
        writer.shutdown(Shutdown::Write).unwrap();
        assert_eq!(
            stream_result(&reader, Duration::from_secs(1)).unwrap(),
            input
        );
        assert_eq!(fcntl(&reader, FcntlArg::F_GETFL).unwrap(), flags);
    }
}

#[test]
fn socket_with_a_silent_or_partial_open_writer_times_out_without_changing_flags() {
    use std::{io::Write, os::unix::net::UnixStream};
    for input in [b"".as_slice(), b"partial".as_slice()] {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let flags = fcntl(&reader, FcntlArg::F_GETFL).unwrap();
        writer.write_all(input).unwrap();
        let before = Instant::now();
        assert_eq!(
            stream_result(&reader, Duration::from_millis(20)),
            Err(ResponseInputFailure::Timeout)
        );
        assert!(before.elapsed() < Duration::from_secs(1));
        assert_eq!(fcntl(&reader, FcntlArg::F_GETFL).unwrap(), flags);
        writer.write_all(b"still open").unwrap();
    }
}

#[test]
fn pipe_byte_limit_accepts_exact_size_and_rejects_one_extra_byte() {
    use std::io::Write;
    for size in [MAX_EXCHANGE_TEXT_BYTES, MAX_EXCHANGE_TEXT_BYTES + 1] {
        let (reader, writer) = pipe().unwrap();
        let flags = fcntl(&reader, FcntlArg::F_GETFL).unwrap();
        std::thread::scope(|scope| {
            let sender = scope.spawn(move || File::from(writer).write_all(&vec![b'x'; size]));
            let result = stream_result(&reader, Duration::from_secs(2));
            let after = fcntl(&reader, FcntlArg::F_GETFL);
            drop(reader);
            sender.join().unwrap().unwrap();
            assert_eq!(after.unwrap(), flags);
            if size == MAX_EXCHANGE_TEXT_BYTES {
                assert_eq!(result.unwrap().as_bytes(), vec![b'x'; size]);
            } else {
                assert_eq!(result, Err(ResponseInputFailure::TooLarge));
            }
        });
    }
}

fn file_result(path: &Path) -> Result<String, ResponseInputFailure> {
    super::read_file(path).map_err(|error| error.kind)
}
fn stream_result(stream: &impl AsFd, timeout: Duration) -> Result<String, ResponseInputFailure> {
    super::read_stream(stream, timeout).map_err(|error| error.kind)
}
#[test]
fn file_failure_preserves_the_os_cause_without_exposing_the_path() {
    let directory = TestDirectory::new();
    let error = super::read_file(&directory.path.join("private-missing-body")).unwrap_err();
    assert_eq!(error.kind, ResponseInputFailure::File);
    assert!(error.source().is_some());
    assert!(!error.to_string().contains("private-missing-body"));
}

#[test]
fn terminal_input_is_rejected_before_flags_change() {
    let terminal = nix::pty::openpty(None, None).unwrap();
    let before = fcntl(&terminal.slave, FcntlArg::F_GETFL).unwrap();
    assert_eq!(
        stream_result(&terminal.slave, Duration::from_secs(1)),
        Err(ResponseInputFailure::Invalid)
    );
    assert_eq!(fcntl(&terminal.slave, FcntlArg::F_GETFL).unwrap(), before);
}
