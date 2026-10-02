use super::*;
use crate::test_support::TestDirectory;
use std::os::unix::fs::PermissionsExt;

fn command(root: &Path, ready: bool) -> RuntimeCommand {
    let path = root.join("fake-codex");
    let banner = if ready {
        "printf '  listening on: ws://127.0.0.1:49000\\n' >&2\n"
    } else {
        ""
    };
    fs::write(
        &path,
        format!("#!/bin/sh\npwd -P > cwd-proof\n{banner}exec /bin/sleep 30\n"),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
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
    assert!(
        OwnedServer::start(
            &command,
            &options,
            &generation,
            Instant::now() + Duration::from_millis(80)
        )
        .is_err()
    );
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
