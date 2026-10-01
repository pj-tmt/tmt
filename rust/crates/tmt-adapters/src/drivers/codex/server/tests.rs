use super::*;
use crate::test_support::TestDirectory;
use std::process::{Command, Stdio};

fn command(root: &Path, ready: bool) -> RuntimeCommand {
    let path = root.join("fake-codex");
    let banner = if ready {
        "printf '  listening on: ws://127.0.0.1:49000\\n' >&2\n"
    } else {
        ""
    };
    // Follow Squad's write_executable pattern: only the short-lived shell
    // opens the executable for writing, so parallel test forks cannot inherit
    // its write descriptor and cause ETXTBSY on the production single-shot exec.
    let mut writer = Command::new("/bin/sh")
        .args(["-c", "cat > \"$1\" && chmod 700 \"$1\"", "sh"])
        .arg(&path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    writer
        .stdin
        .take()
        .unwrap()
        .write_all(
            format!("#!/bin/sh\npwd -P > cwd-proof\n{banner}exec /bin/sleep 30\n").as_bytes(),
        )
        .unwrap();
    assert!(writer.wait().unwrap().success());
    RuntimeCommand {
        executable: path.into_os_string(),
        args: Vec::new(),
    }
}

#[test]
fn owned_process_and_private_files_are_cleaned_after_success() {
    let fixture = TestDirectory::new();
    let root = fixture.path.canonicalize().unwrap();
    let command = command(&root, true);
    let options = LaunchOptions::parse(&command, &root).unwrap();
    let generation = root.join("generation");
    let mut server = OwnedServer::start(
        &command,
        &options,
        &generation,
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(server.endpoint.url(), "ws://127.0.0.1:49000");
    assert_eq!(
        fs::read_to_string(root.join("cwd-proof")).unwrap().trim(),
        root.to_str().unwrap()
    );
    assert_eq!(
        fs::read_to_string(generation.join("capability")).unwrap(),
        server.capability()
    );
    assert_eq!(
        fs::metadata(generation.join("capability")).unwrap().mode() & 0o777,
        0o600
    );
    let pid = server.incarnation.pid();
    server.stop().unwrap();
    assert!(!generation.exists());
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None),
        Err(nix::errno::Errno::ESRCH)
    );
}

#[test]
fn startup_timeout_and_spawn_failure_clean_owned_generation() {
    let fixture = TestDirectory::new();
    let root = fixture.path.canonicalize().unwrap();
    let command = command(&root, false);
    let options = LaunchOptions::parse(&command, &root).unwrap();
    let generation = root.join("generation");
    let error = OwnedServer::start(
        &command,
        &options,
        &generation,
        Instant::now() + Duration::from_millis(250),
    )
    .err()
    .expect("startup must time out without an announcement");
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(!generation.exists());
    let missing = RuntimeCommand {
        executable: root.join("missing").into_os_string(),
        args: Vec::new(),
    };
    assert!(
        OwnedServer::start(
            &missing,
            &options,
            &generation,
            Instant::now() + Duration::from_secs(1)
        )
        .is_err()
    );
    assert!(!generation.exists());
    assert!(root.join("fake-codex").exists());
}

#[test]
fn announcement_timeout_drops_exact_owned_process_and_files() {
    let fixture = TestDirectory::new();
    let generation = fixture.path.join("generation");
    let mut files = Files::create(&generation).unwrap();
    let mut log = files.create_file("server.log").unwrap();
    // Exercise the same private startup ownership boundary, using the PID
    // returned by spawn instead of racing a child-written marker against the
    // public deadline (which also includes file creation/fsync/spawn).
    let job = Exec::cmd("/bin/sleep")
        .arg("30")
        .setpgid()
        .stdin(Redirection::Null)
        .stdout(Redirection::Null)
        .stderr(log.try_clone().unwrap())
        .start()
        .unwrap();
    let resources = Resources {
        process: Process(Some(job)),
        files,
    };
    let pid = nix::unistd::Pid::from_raw(resources.process.pid() as i32);
    assert!(nix::sys::signal::kill(pid, None).is_ok());
    let error = wait_address(&mut log, Instant::now() + Duration::from_millis(40)).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    drop(resources);
    assert_eq!(
        nix::sys::signal::kill(pid, None),
        Err(nix::errno::Errno::ESRCH)
    );
    assert!(!generation.exists());
}

#[test]
fn replaced_cleanup_file_is_preserved() {
    let fixture = TestDirectory::new();
    let generation = fixture.path.join("generation");
    let mut files = Files::create(&generation).unwrap();
    let original = files.create_file("capability").unwrap();
    fs::remove_file(generation.join("capability")).unwrap();
    fs::write(generation.join("capability"), b"replacement").unwrap();
    assert!(files.remove().is_err());
    assert_eq!(
        fs::read(generation.join("capability")).unwrap(),
        b"replacement"
    );
    drop(original);
    drop(files);
    assert_eq!(
        fs::read(generation.join("capability")).unwrap(),
        b"replacement"
    );
}

#[test]
fn announcement_requires_loopback_nonzero_port() {
    assert_eq!(
        announced_address(b"  listening on: ws://127.0.0.1:49000\n"),
        Some(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 49000))
    );
    for text in [
        "listening on: ws://0.0.0.0:49000",
        "listening on: ws://127.0.0.1:0",
        "listening on: ws://remote:49000",
        "not ready",
    ] {
        assert!(announced_address(text.as_bytes()).is_none());
    }
}

#[test]
fn retained_cleanup_diagnostics_and_replaced_directory_are_preserved() {
    let fixture = TestDirectory::new();
    let generation = fixture.path.join("generation");
    let mut files = Files::create(&generation).unwrap();
    files
        .create_file("server.log")
        .unwrap()
        .write_all(b"diagnostic")
        .unwrap();
    // The process cleanup failure path marks files retained; Drop must not
    // undo that ordering decision, nor can a later stop erase them.
    files.retained = true;
    let mut resources = Resources {
        process: Process(None),
        files,
    };
    assert!(resources.stop().is_err());
    drop(resources);
    assert_eq!(
        fs::read(generation.join("server.log")).unwrap(),
        b"diagnostic"
    );

    let other = fixture.path.join("other-generation");
    let mut files = Files::create(&other).unwrap();
    let old = fixture.path.join("original-directory");
    fs::rename(&other, &old).unwrap();
    fs::create_dir(&other).unwrap();
    fs::write(other.join("replacement"), b"untouched").unwrap();
    assert!(files.remove().is_err());
    drop(files);
    assert_eq!(fs::read(other.join("replacement")).unwrap(), b"untouched");
}
