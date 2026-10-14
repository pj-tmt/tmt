//! One isolated home of the proof: private HOME and state roots, the real executables on a
//! private PATH, loopback emulators only, and every child in its own process group that is
//! stopped with the home. Nothing here touches the host's tmux, browser or stores.
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

/// What the harness hands the proof: the built executables and the emulators it started.
pub(super) struct Inputs {
    pub bins: PathBuf,
    pub project: String,
    pub auth: String,
    pub firestore: String,
}
impl Inputs {
    pub(super) fn from_env() -> Self {
        let var = |name: &str| {
            std::env::var(name).unwrap_or_else(|_| {
                panic!("{name} is required: run this proof through its harness script")
            })
        };
        let inputs = Self {
            bins: PathBuf::from(var("TMT_PROOF_BIN_DIR")),
            project: var("TMT_PROOF_PROJECT"),
            auth: var("FIREBASE_AUTH_EMULATOR_HOST"),
            firestore: var("FIRESTORE_EMULATOR_HOST"),
        };
        assert!(inputs.project.starts_with("demo-"), "only a demo project");
        for host in [&inputs.auth, &inputs.firestore] {
            assert!(
                host.starts_with("127.0.0.1:"),
                "loopback emulators only: {host}"
            );
        }
        for name in ["tmt", "tmt-colab", "tmt-remote"] {
            assert!(
                inputs.bins.join(name).exists(),
                "missing {name} in the bin dir"
            );
        }
        inputs
    }
}

pub(super) struct World {
    pub root: PathBuf,
    /// `TMT_HOME`: Remote's state is `remote/`, Colab's is `colab/`.
    pub state: PathBuf,
    env: Vec<(String, String)>,
    wrap: PathBuf,
}
impl World {
    pub(super) fn new(inputs: &Inputs, name: &str) -> Self {
        // A short root: Unix socket paths are limited to about 100 bytes.
        let root = PathBuf::from(format!("/tmp/tmt-prf-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        let state = root.join("state");
        let wrap = root.join("wrap");
        for directory in [&root, &home, &state, &wrap] {
            fs::create_dir_all(directory).unwrap();
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        for sub in ["config", "data", "state", "cache"] {
            fs::create_dir(home.join(sub)).unwrap();
        }
        for tool in ["tmt", "tmt-colab", "tmt-remote"] {
            std::os::unix::fs::symlink(inputs.bins.join(tool), wrap.join(tool)).unwrap();
        }
        // Stand-ins for every host opener: a world must never open a browser.
        for opener in ["open", "xdg-open", "wslview"] {
            let path = wrap.join(opener);
            fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let set = |key: &str, value: &Path| (key.to_owned(), value.display().to_string());
        let env = vec![
            set("HOME", &home),
            set("XDG_CONFIG_HOME", &home.join("config")),
            set("XDG_DATA_HOME", &home.join("data")),
            set("XDG_STATE_HOME", &home.join("state")),
            set("XDG_CACHE_HOME", &home.join("cache")),
            set("TMT_HOME", &state),
            set("TMPDIR", &root),
            set("TMUX_TMPDIR", &root),
            set("TMT_EXECUTABLE", &wrap.join("tmt")),
            set("BROWSER", &wrap.join("open")),
            ("TMT_AGENT".into(), "1".into()),
            ("LANG".into(), "C.UTF-8".into()),
            ("PATH".into(), format!("{}:/usr/bin:/bin", wrap.display())),
            ("FIREBASE_AUTH_EMULATOR_HOST".into(), inputs.auth.clone()),
            ("FIRESTORE_EMULATOR_HOST".into(), inputs.firestore.clone()),
        ];
        Self {
            root,
            state,
            env,
            wrap,
        }
    }
    /// A command with only this home's environment, never the host's.
    pub(super) fn command(&self, program: &str) -> Command {
        let mut command = Command::new(self.wrap.join(program));
        command.env_clear().envs(self.env.iter().cloned());
        command.current_dir(&self.root);
        command
    }
    /// Run a short command to its end and keep its output; a nonzero exit is a failure.
    pub(super) fn run(&self, program: &str, args: &[&str]) -> Output {
        let output = self.command(program).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{program} {args:?} failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }
    pub(super) fn run_json(&self, program: &str, args: &[&str]) -> Value {
        let output = self.run(program, args);
        serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "{program} {args:?} printed no JSON: {}",
                String::from_utf8_lossy(&output.stdout)
            )
        })
    }
    /// `tmt remote ...` through the real core, as a person would type it.
    pub(super) fn remote(&self, args: &[&str]) -> Output {
        let mut all = vec!["remote"];
        all.extend_from_slice(args);
        self.run("tmt", &all)
    }
    /// Start a long-lived child in its own process group and wait for the first JSON line that
    /// `ready` accepts; its stderr goes to a file the proof can read.
    fn spawn(&self, name: &str, program: &str, args: &[&str], ready: fn(&Value) -> bool) -> Proc {
        let log = self.root.join(format!("{name}.err"));
        let mut child = self
            .command(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(fs::File::create(&log).unwrap())
            .process_group(0)
            .spawn()
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let stdout = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let ready = loop {
            let line = receiver
                .recv_timeout(Duration::from_secs(90))
                .unwrap_or_else(|_| panic!("{name} was not ready; stderr: {}", read(&log)));
            if let Ok(value) = serde_json::from_str::<Value>(&line)
                && ready(&value)
            {
                break value;
            }
        };
        Proc {
            child: Some(child),
            ready,
            log,
        }
    }
    /// The Remote door, started by the proof so its stderr is kept. Colab then attaches to it.
    pub(super) fn door(&self) -> Proc {
        self.spawn(
            "remote",
            "tmt-remote",
            &["serve", "--json", "--port", "0"],
            |value| value["address"].is_string(),
        )
    }
    /// Start `tmt colab serve` for this home and wait for its readiness record.
    pub(super) fn serve(&self) -> Proc {
        self.spawn(
            "colab",
            "tmt-colab",
            &["serve", "--json", "--no-open"],
            |value| value["state"] == "mounted",
        )
    }
}
fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}
impl Drop for World {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) struct Proc {
    child: Option<Child>,
    pub ready: Value,
    log: PathBuf,
}
impl Proc {
    pub(super) fn socket(&self) -> PathBuf {
        PathBuf::from(self.ready["socket"].as_str().unwrap())
    }
    /// Everything the serve has printed to stderr so far.
    pub(super) fn stderr(&self) -> String {
        read(&self.log)
    }
    /// Stop through the program's own stop request, then make sure nothing of it remains.
    pub(super) fn stop(mut self, world: &World, program: &str) {
        let _ = world.command(program).arg("stop").output();
        self.finish();
    }
    fn finish(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let group = nix::unistd::Pid::from_raw(child.id() as i32);
        let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL);
        let _ = child.wait();
    }
}
impl Drop for Proc {
    fn drop(&mut self) {
        if let Some(child) = &self.child {
            let group = nix::unistd::Pid::from_raw(child.id() as i32);
            let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGTERM);
        }
        self.finish();
    }
}
