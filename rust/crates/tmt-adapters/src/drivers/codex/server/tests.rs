use super::*;
use crate::test_support::{TestChild, TestDirectory};
use std::{
    io::{BufRead, BufReader, Read},
    os::unix::{net::UnixStream, process::CommandExt, process::ExitStatusExt},
    process::{Command, Stdio},
};

// First execution of a published fixture is preparation, not listener startup.
// Monitored reads/exit share a thirty-second deadline; product budgets remain
// unchanged. This deadline does not preempt synchronous OS spawn or cleanup.
const PREPARATION_TIMEOUT: Duration = Duration::from_secs(30);

fn command(root: &Path, ready: bool) -> RuntimeCommand {
    let path = root.join("fake-codex");
    let banner = if ready {
        "printf '  listening on: ws://127.0.0.1:49000\\n' >&2\nprintf 'announcement-written\\n' > \"$fixture_root/announcement-proof\"\n"
    } else {
        ""
    };
    tmt_test_support::write_executable(
        &path,
        format!(
            "#!/bin/sh\n\
             fixture_root=${{0%/*}}\n\
             if [ \"${{1-}}\" = --tmt-fixture-prepare ]; then\n\
               printf 'preparation-entered\\n'\n\
               if [ -p \"$fixture_root/preparation-gate\" ]; then\n\
                 IFS= read -r release < \"$fixture_root/preparation-gate\" || exit 71\n\
                 [ \"$release\" = release ] || exit 72\n\
               fi\n\
               exit 0\n\
             fi\n\
             printf '%s\\n' \"$$\" > \"$fixture_root/server-entry\"\n\
             if [ -p \"$fixture_root/preparation-gate\" ]; then\n\
               IFS= read -r release < \"$fixture_root/preparation-gate\" || exit 71\n\
               [ \"$release\" = release ] || exit 72\n\
             fi\n\
             pwd -P > cwd-proof\n{banner}exec /bin/sleep 30\n"
        )
        .as_bytes(),
        0o700,
    )
    .unwrap();
    RuntimeCommand {
        executable: path.into_os_string(),
        args: Vec::new(),
    }
}

fn preparation(
    command: &RuntimeCommand,
    root: &Path,
    deadline: Instant,
) -> (TestChild, BufReader<UnixStream>) {
    let (reader, writer) = UnixStream::pair().unwrap();
    assert!(
        Instant::now() < deadline,
        "fixture preparation deadline expired"
    );
    let child = TestChild::new(
        Command::new(&command.executable)
            .arg("--tmt-fixture-prepare")
            .env_clear()
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::from(std::os::fd::OwnedFd::from(
                writer.try_clone().unwrap(),
            )))
            .stderr(Stdio::from(std::os::fd::OwnedFd::from(writer)))
            .spawn()
            .unwrap(),
    );
    let mut output = BufReader::new(reader);
    let mut line = String::new();
    // Spawn may consume the budget. Refuse with the child already owned, and
    // derive the read timeout from the remaining absolute budget, not its age
    // before spawn. TestChild stops/reaps on every refused or panicking path.
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .expect("fixture preparation deadline expired after spawn");
    output.get_ref().set_read_timeout(Some(remaining)).unwrap();
    output.read_line(&mut line).unwrap();
    assert!(
        Instant::now() < deadline,
        "fixture preparation first line arrived after deadline"
    );
    assert_eq!(line, "preparation-entered\n");
    (child, output)
}

fn finish_preparation(
    child: &mut TestChild,
    output: &mut BufReader<UnixStream>,
    deadline: Instant,
) {
    let status = child.wait_for_exit(deadline.saturating_duration_since(Instant::now()));
    assert!(
        Instant::now() < deadline,
        "fixture preparation exceeded its failure bound"
    );
    output
        .get_ref()
        .set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())))
        .unwrap();
    let mut remaining = String::new();
    output.read_to_string(&mut remaining).unwrap();
    eprintln!(
        "fixture preparation exit: pid={}, status={status}, output={remaining:?}",
        child.child.id()
    );
    assert_eq!(status.code(), Some(0));
    assert_eq!(remaining, "", "unexpected preparation output");
    assert_reaped(child.child.id());
    eprintln!(
        "fixture preparation: exact zero exit and reap, pid={}",
        child.child.id()
    );
}

fn prepare(command: &RuntimeCommand, root: &Path) {
    let deadline = Instant::now() + PREPARATION_TIMEOUT;
    let (mut child, mut output) = preparation(command, root, deadline);
    finish_preparation(&mut child, &mut output, deadline);
    assert!(!root.join("cwd-proof").exists());
    assert!(!root.join("server-entry").exists());
    assert!(!root.join("generation").exists());
    assert!(!root.join("announcement-proof").exists());
}

fn assert_reaped(pid: u32) {
    assert_eq!(
        nix::sys::wait::waitpid(
            nix::unistd::Pid::from_raw(pid as i32),
            Some(nix::sys::wait::WaitPidFlag::WNOHANG),
        ),
        Err(nix::errno::Errno::ECHILD)
    );
    assert_eq!(
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None),
        Err(nix::errno::Errno::ESRCH)
    );
}

#[test]
fn owned_process_and_private_files_are_cleaned_after_success() {
    let fixture = TestDirectory::new();
    let root = fixture.path.canonicalize().unwrap();
    let command = command(&root, true);
    let gate = root.join("preparation-gate");
    nix::unistd::mkfifo(
        &gate,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    // The parent owns both FIFO ends; no gate helper or descendant can survive
    // TestChild's exact stop/reap on assertion failure or unwind.
    let mut release = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(nix::libc::O_NONBLOCK)
        .open(&gate)
        .unwrap();
    let deadline = Instant::now() + PREPARATION_TIMEOUT;
    let (mut child, mut output) = preparation(&command, &root, deadline);
    assert!(child.child.try_wait().unwrap().is_none());
    assert!(!root.join("cwd-proof").exists());
    assert!(!root.join("server-entry").exists());
    assert!(!root.join("generation").exists());
    assert!(!root.join("announcement-proof").exists());
    eprintln!(
        "held preparation: no server entry, announcement, cwd proof or generation before release"
    );
    release.write_all(b"release\n").unwrap();
    finish_preparation(&mut child, &mut output, deadline);
    fs::remove_file(&gate).unwrap();
    drop(release);
    let options = LaunchOptions::parse(&command, &root).unwrap();
    let generation = root.join("generation");
    let started = Instant::now();
    let result = OwnedServer::start(
        &command,
        &options,
        &generation,
        Instant::now() + Duration::from_secs(2),
    );
    eprintln!(
        "prepared startup: elapsed={:?}, script-entry={:?}, announcement={:?}, cwd-proof={}, result={:?}",
        started.elapsed(),
        fs::read_to_string(root.join("server-entry")),
        fs::read_to_string(root.join("announcement-proof")),
        root.join("cwd-proof").exists(),
        result.as_ref().map(|server| server.incarnation.pid()),
    );
    let mut server = result.unwrap();
    assert_eq!(server.endpoint.url(), "ws://127.0.0.1:49000");
    assert_eq!(fs::metadata(&generation).unwrap().mode() & 0o777, 0o700);
    assert_eq!(
        fs::metadata(root.join("fake-codex")).unwrap().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(generation.join("server.log")).unwrap().mode() & 0o777,
        0o600
    );
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
    assert_reaped(pid);
    eprintln!("prepared startup cleanup: exact reap pid={pid}, generation absent");
}

#[test]
fn bypassing_held_preparation_expires_the_real_startup_deadline() {
    let fixture = TestDirectory::new();
    let root = fixture.path.canonicalize().unwrap();
    let command = command(&root, true);
    let gate = root.join("preparation-gate");
    nix::unistd::mkfifo(
        &gate,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    let options = LaunchOptions::parse(&command, &root).unwrap();
    let generation = root.join("generation");
    // The identical published executable can enter, but cannot perform server
    // effects until preparation is released. Bypass only the preparation call.
    let error = OwnedServer::start(
        &command,
        &options,
        &generation,
        Instant::now() + Duration::from_secs(2),
    )
    .err()
    .expect("bypassed preparation must exhaust listener startup");
    eprintln!("bypassed preparation start-return: {error:?}");
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(error.error.to_string(), "Codex listener startup timed out");
    assert!(error.cleanup_confirmed());
    let pid = fs::read_to_string(root.join("server-entry"))
        .expect("the fixture actually entered before the targeted timeout")
        .trim()
        .parse::<u32>()
        .unwrap();
    assert_reaped(pid);
    assert!(!generation.exists());
    assert!(!root.join("cwd-proof").exists());
    assert!(!root.join("announcement-proof").exists());
    // Release is selected only after the original startup deadline and exact
    // reap. There is no successor start and no retry after this negative.
    fs::remove_file(&gate).unwrap();
}

#[test]
fn held_preparation_is_reaped_on_unwind_before_fixture_removal() {
    let fixture = TestDirectory::new();
    let root = fixture.path.canonicalize().unwrap();
    let command = command(&root, true);
    let gate = root.join("preparation-gate");
    nix::unistd::mkfifo(
        &gate,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    let (child, output) = preparation(&command, &root, Instant::now() + PREPARATION_TIMEOUT);
    let pid = child.child.id();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _child = child;
        let _output = output;
        panic!("exercise held fixture preparation unwind");
    }));
    assert!(result.is_err());
    assert_reaped(pid);
    assert!(!root.join("cwd-proof").exists());
    assert!(!root.join("server-entry").exists());
    assert!(!root.join("generation").exists());
    assert!(!root.join("announcement-proof").exists());
    assert!(root.join("fake-codex").exists());
}

#[test]
fn group_shutdown_kills_and_reaps_a_term_ignoring_child() {
    let fixture = TestDirectory::new();
    let root = fixture.path.canonicalize().unwrap();
    let command = command(&root, true);
    prepare(&command, &root);
    let options = LaunchOptions::parse(&command, &root).unwrap();
    let generation = root.join("generation");
    let mut server = OwnedServer::start(
        &command,
        &options,
        &generation,
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap();
    let leader = nix::unistd::Pid::from_raw(server.incarnation.pid() as i32);
    let (mut controller, member) = UnixStream::pair().unwrap();
    controller
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    controller
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    // The test parents this child so it can verify reaping without leaving an
    // orphan zombie. Group membership, not parentage, is the teardown boundary.
    let mut child = TestChild::new(
        Command::new("/bin/sh")
            .args(["-c", "trap '' TERM; printf 'ready\\n'; exec /bin/cat"])
            .process_group(leader.as_raw())
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(
                member.try_clone().unwrap(),
            )))
            .stdout(Stdio::from(std::os::fd::OwnedFd::from(member)))
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let member_pid = nix::unistd::Pid::from_raw(child.child.id() as i32);
    let mut output = BufReader::new(controller.try_clone().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    assert_eq!(line, "ready\n");
    assert_eq!(nix::unistd::getpgid(Some(member_pid)).unwrap(), leader);
    nix::sys::signal::kill(member_pid, nix::sys::signal::Signal::SIGTERM).unwrap();
    controller.write_all(b"survived TERM\n").unwrap();
    line.clear();
    output.read_line(&mut line).unwrap();
    assert_eq!(line, "survived TERM\n");
    assert!(child.child.try_wait().unwrap().is_none());
    let mut independent = TestChild::new(Command::new("/bin/sleep").arg("30").spawn().unwrap());

    let started = Instant::now();
    server.stop().unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    let status = child.wait_for_exit(Duration::from_secs(2));
    assert_eq!(status.signal(), Some(nix::libc::SIGKILL));
    for pid in [leader, member_pid] {
        assert_eq!(
            nix::sys::signal::kill(pid, None),
            Err(nix::errno::Errno::ESRCH)
        );
    }
    assert!(!generation.exists());
    assert!(independent.child.try_wait().unwrap().is_none());
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
    assert!(error.cleanup_confirmed());
    assert!(!generation.exists());
    let missing = RuntimeCommand {
        executable: root.join("missing").into_os_string(),
        args: Vec::new(),
    };
    let error = OwnedServer::start(
        &missing,
        &options,
        &generation,
        Instant::now() + Duration::from_secs(1),
    )
    .err()
    .expect("missing executable must fail spawn");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(error.cleanup_confirmed());
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
