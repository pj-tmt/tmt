//! Native provider-shaped parent for model-free interpreted acceptance peers.
//! Catchable TERM/INT/HUP signals are forwarded and the child is reaped. The
//! acceptance world owns process-group/tmux cleanup after uncatchable parent
//! death and must assert that no child remains; this fixture cannot catch KILL.

mod provider_launch;

use std::{
    io::{self, Read},
    os::unix::{net::UnixStream, process::ExitStatusExt},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use signal_hook::{
    SigId,
    consts::{SIGCHLD, SIGHUP, SIGINT, SIGTERM},
    flag, low_level,
};

fn main() {
    let result = (|| {
        let interpreter = std::env::var_os("TMT_TEST_PROVIDER_INTERPRETER").ok_or_else(|| {
            io::Error::other("TMT_TEST_PROVIDER_INTERPRETER must name an absolute interpreter")
        })?;
        if !std::path::Path::new(&interpreter).is_absolute() {
            return Err(io::Error::other(
                "TMT_TEST_PROVIDER_INTERPRETER must be absolute",
            ));
        }
        let mut args = std::env::args_os().skip(1).collect();
        provider_launch::scripted_launch_options(&mut args);
        run(Command::new(interpreter).args(args))
    })();
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("provider-script-fixture: {error}");
            std::process::exit(1);
        }
    }
}

fn run(command: &mut Command) -> io::Result<i32> {
    // Register before spawn: an immediately exiting child or a teardown signal
    // must remain pending until the child PID is available. No second timer.
    let mut signals = ChildSignals::new()?;
    let mut child = command.spawn()?;
    let result = (|| {
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(status
                    .code()
                    .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)));
            }
            for (signal, pending) in &signals.pending {
                if pending.swap(false, Ordering::SeqCst) {
                    match kill(
                        Pid::from_raw(i32::try_from(child.id()).map_err(io::Error::other)?),
                        *signal,
                    ) {
                        Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
                        Err(error) => return Err(io::Error::other(error)),
                    }
                }
            }
            signals.wait()?;
        }
    })();
    if result.is_err() {
        // A fixture infrastructure failure must still leave no owned child.
        let _ = child.kill();
        child.wait()?;
    }
    result
}

// The same signal-hook flag + self-pipe mechanism as the native process owner,
// kept local because this fixture forwards INT and has no product job control.
struct ChildSignals {
    reader: UnixStream,
    pending: Vec<(Signal, Arc<AtomicBool>)>,
    registrations: Vec<SigId>,
}

impl ChildSignals {
    fn new() -> io::Result<Self> {
        let (reader, writer) = UnixStream::pair()?;
        writer.set_nonblocking(true)?;
        let mut signals = Self {
            reader,
            pending: Vec::new(),
            registrations: Vec::new(),
        };
        for signal in [Signal::SIGTERM, Signal::SIGINT, Signal::SIGHUP] {
            let pending = Arc::new(AtomicBool::new(false));
            signals
                .registrations
                .push(flag::register(signal as i32, pending.clone())?);
            signals.pending.push((signal, pending));
        }
        for signal in [SIGCHLD, SIGTERM, SIGINT, SIGHUP] {
            signals
                .registrations
                .push(low_level::pipe::register(signal, writer.try_clone()?)?);
        }
        Ok(signals)
    }

    fn wait(&mut self) -> io::Result<()> {
        loop {
            match self.reader.read(&mut [0; 256]) {
                Ok(0) => return Err(io::Error::other("fixture signal pipe closed")),
                Ok(_) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

impl Drop for ChildSignals {
    fn drop(&mut self) {
        for registration in self.registrations.drain(..) {
            low_level::unregister(registration);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        process::Stdio,
        time::{Duration, Instant},
    };

    struct World(PathBuf);
    impl World {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("provider-script-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for World {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn interpreter_receives_literal_arguments_and_stdio_and_preserves_exit_status() {
        let world = World::new();
        let script = world.0.join("peer.sh");
        fs::write(
            &script,
            "read -r line\nprintf '%s|%s' \"$1\" \"$line\"\nprintf 'diagnostic' >&2\nexit 37\n",
        )
        .unwrap();
        let input = world.0.join("input");
        fs::write(&input, "literal input\n").unwrap();
        let output = world.0.join("output");
        let error = world.0.join("error");
        assert_eq!(
            run(Command::new("/bin/sh")
                .arg(script)
                .arg("$(not a shell) ' spaced")
                .stdin(fs::File::open(input).unwrap())
                .stdout(fs::File::create(&output).unwrap())
                .stderr(fs::File::create(&error).unwrap()))
            .unwrap(),
            37
        );
        assert_eq!(
            fs::read_to_string(output).unwrap(),
            "$(not a shell) ' spaced|literal input"
        );
        assert_eq!(fs::read_to_string(error).unwrap(), "diagnostic");
        assert_eq!(
            run(Command::new("/bin/sh").args(["-c", "exit 0"])).unwrap(),
            0
        );
    }

    #[test]
    fn signal_child() {
        let Some(directory) = std::env::var_os("TMT_PROVIDER_SIGNAL_WORLD") else {
            return;
        };
        let script = PathBuf::from(directory).join("peer.sh");
        let code = run(Command::new("/bin/sh").arg(script)).unwrap();
        std::process::exit(code);
    }

    #[test]
    fn forwards_each_teardown_signal_and_reaps_the_interpreter() {
        for signal in [Signal::SIGTERM, Signal::SIGINT, Signal::SIGHUP] {
            let world = World::new();
            let ready = world.0.join("ready");
            fs::write(
                world.0.join("peer.sh"),
                format!(
                    "printf '%s' \"$$\" > '{}'\nmv '{}' '{}'\nexec /bin/sleep 60\n",
                    world.0.join("ready.tmp").display(),
                    world.0.join("ready.tmp").display(),
                    ready.display()
                ),
            )
            .unwrap();
            let mut parent = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::signal_child",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("TMT_PROVIDER_SIGNAL_WORLD", &world.0)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            let child_pid = loop {
                if let Ok(value) = fs::read_to_string(&ready)
                    && let Ok(pid) = value.parse::<i32>()
                {
                    break Some(pid);
                }
                if Instant::now() >= deadline {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            let result = child_pid.and_then(|pid| {
                kill(Pid::from_raw(i32::try_from(parent.id()).unwrap()), signal).unwrap();
                loop {
                    if let Some(status) = parent.try_wait().unwrap() {
                        break Some((pid, status));
                    }
                    if Instant::now() >= deadline {
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            });
            if result.is_none() {
                if let Some(pid) = child_pid {
                    let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
                }
                let _ = parent.kill();
                parent.wait().unwrap();
            }
            let (pid, status) =
                result.expect("fixture must become ready and terminate within its bound");
            assert_eq!(status.code(), Some(128 + signal as i32));
            assert_eq!(
                kill(Pid::from_raw(pid), None),
                Err(nix::errno::Errno::ESRCH),
                "interpreter must be reaped"
            );
        }
    }
}
