//! Real parent-only SIGINT through the bounded process owner and apply checkpoints.
use super::*;
use crate::native_upgrade_command::{DatabaseSteps, FinishContext, Prior, finish};
use crate::setup_command::guided::tests::Directory;
use nix::{
    errno::Errno,
    sys::signal::{Signal, kill, killpg},
    unistd::Pid,
};
use std::{
    cell::Cell,
    fs,
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
};
use tmt_adapters::{
    interrupt::Interrupt,
    process::{CommandError, CommandOutput},
};

const PARENT: &str = "native_upgrade_command::complete::signal_tests::parent_fixture";
const APPLY: &str = "extension_install_command::upgrade_all::complete::tests::signal_apply_fixture";

fn approved() -> Plan {
    Plan {
        installed: upgrade_all::Plan { products: vec![], pending: vec![] },
        missing: ["ops", "remote", "digest"].into_iter().map(|name| json!({
            "product":name,"action":"install","channel":"alpha","version":"0.1.0-alpha.2",
            "status":"pending","selected":true,"hint":format!("tmt extension install {name} --yes")
        })).collect(),
        providers: vec![],
    }
}

struct ApplyRunner {
    root: PathBuf,
    mode: String,
    applies: Cell<usize>,
}
impl CommandRunner for ApplyRunner {
    fn execute(&self, request: CommandRequest<'_>) -> Result<CommandOutput, CommandError> {
        assert!(request.args.contains(&"--plan".into()));
        Ok(CommandOutput {
            stdout: serde_json::to_vec(&approved().document()).unwrap(),
            stderr: upgrade_all::complete::CAPABILITY.to_vec(),
        })
    }

    fn execute_cancellable(
        &self,
        request: CommandRequest<'_>,
        stderr_limit: usize,
        observer: &mut dyn FnMut(&[u8]),
        interrupt: &Interrupt,
    ) -> Result<CommandOutput, CommandError> {
        if request.args.contains(&"--plan".into()) {
            return self.execute(request);
        }
        self.applies.set(self.applies.get() + 1);
        assert_eq!(self.applies.get(), 1, "apply must never be replayed");
        // The shim keeps libtest's framing off the protocol pipe. SIGINT still
        // reaches the real Rust apply owner in this same private group.
        let args = ["-c".into(),
            "trap '' INT; printf '%s' \"$$\" > \"$1/group\"; TMT_UPGRADE_SIGNAL_ROOT=\"$1\" TMT_UPGRADE_SIGNAL_MODE=\"$3\" \"$2\" --exact \"$4\" --nocapture > /dev/null; status=$?; if [ -f \"$1/child-report\" ]; then cat \"$1/child-report\"; fi; exit \"$status\"".into(),
            "apply-fixture".into(), self.root.clone().into_os_string(),
            std::env::current_exe().unwrap().into_os_string(), self.mode.clone().into(), APPLY.into()];
        let budget = if self.mode == "deadline" {
            Duration::from_secs(2)
        } else if self.mode == "stubborn" {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(5)
        };
        // Borrowed-runner forwarding must retain the cancellation capability.
        let borrowed = &UnixCommandRunner;
        let result = (&borrowed).execute_cancellable(
            CommandRequest {
                program: std::ffi::OsStr::new("/bin/sh"),
                args: &args,
                input: request.input,
                deadline: request.deadline.min(Instant::now() + budget),
                max_output_bytes: request.max_output_bytes,
            },
            stderr_limit,
            observer,
            interrupt,
        );
        if self.mode == "late" {
            kill(Pid::this(), Signal::SIGINT).unwrap();
        }
        result
    }
}

#[test]
fn parent_fixture() {
    let Some(root) = std::env::var_os("TMT_UPGRADE_PARENT_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let runner = ApplyRunner {
        root: root.clone(),
        mode: std::env::var("TMT_UPGRADE_SIGNAL_MODE").unwrap(),
        applies: Cell::new(0),
    };
    let mut interrupt = Interrupt::install().unwrap();
    let report = UpgradeReport {
        installation: native_install::InstallReport {
            executable: root.join("bin/tmt"),
            active_executable: root.join("active/tmt"),
            version: "5.0.0".into(),
            changed: false,
        },
        state: tmt_core::native_install::InstalledVersion {
            version: "5.0.0".parse().unwrap(),
            channel: Channel::Alpha,
            pinned_version: Some("5.0.0".parse().unwrap()),
        },
        skipped_pinned: true,
    };
    if runner.mode == "before" {
        kill(Pid::this(), Signal::SIGINT).unwrap();
    }
    // Exercise root finish, including a pinned CLI, with its real invocation
    // guard alive through final JSON publication and status selection.
    let status = finish(
        &report,
        None,
        None,
        true,
        &Prior {
            repeat: "tmt upgrade --yes".into(),
            ran_from_moved_prefix: false,
            versions: BTreeMap::new(),
            extensions: approved(),
        },
        FinishContext {
            database: DatabaseSteps::default(),
            interrupt: Some(&mut interrupt),
            runner: &runner,
        },
        OutputMode { json: true },
    )
    .unwrap();
    assert_eq!(runner.applies.get(), usize::from(runner.mode != "before"));
    fs::write(root.join("applies"), runner.applies.get().to_string()).unwrap();
    std::process::exit(i32::from(status));
}

struct Parent {
    child: Child,
    root: Option<Directory>,
}
impl Parent {
    fn root(&self) -> &Path {
        &self.root.as_ref().unwrap().path
    }

    fn start(mode: &str) -> Self {
        let root = Directory::new();
        let stdout = fs::File::create(root.path.join("stdout")).unwrap();
        let stderr = fs::File::create(root.path.join("stderr")).unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", PARENT, "--nocapture", "--test-threads=1"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &root.path)
            .env("XDG_CONFIG_HOME", root.path.join("config"))
            .env("XDG_DATA_HOME", root.path.join("data"))
            .env("XDG_STATE_HOME", root.path.join("state"))
            .env("XDG_CACHE_HOME", root.path.join("cache"))
            .env("TMPDIR", &root.path)
            .env("TMT_AGENT", "1")
            .env("BROWSER", "/usr/bin/true")
            .env("TMT_UPGRADE_PARENT_ROOT", &root.path)
            .env("TMT_UPGRADE_SIGNAL_MODE", mode)
            .current_dir(&root.path)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .unwrap();
        Self {
            child,
            root: Some(root),
        }
    }
    fn wait_for(&mut self, name: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.root().join(name).exists() {
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "parent exited before {name}: {}",
                fs::read_to_string(self.root().join("stderr")).unwrap()
            );
            assert!(Instant::now() < deadline, "missing {name}");
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn signal(&self) {
        kill(Pid::from_raw(self.child.id() as i32), Signal::SIGINT).unwrap();
    }
    fn finish(&mut self, expected: i32) -> Value {
        let deadline = Instant::now() + Duration::from_secs(6);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "parent failed to settle");
            thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(
            status.code(),
            Some(expected),
            "parent stderr: {}",
            fs::read_to_string(self.root().join("stderr")).unwrap()
        );
        let stdout = fs::read(self.root().join("stdout")).unwrap();
        let start = stdout.iter().position(|byte| *byte == b'{').unwrap();
        let result = serde_json::from_slice(&stdout[start..]).unwrap();
        let applies = fs::read_to_string(self.root().join("applies")).unwrap();
        assert!(applies == "0" || applies == "1");
        if applies == "0" {
            assert!(!self.root().join("group").exists());
            return result;
        }
        let group: i32 = fs::read_to_string(self.root().join("group"))
            .unwrap()
            .parse()
            .unwrap();
        let cleanup = Instant::now() + Duration::from_secs(2);
        loop {
            match killpg(Pid::from_raw(group), None) {
                Err(Errno::ESRCH) => break,
                Ok(()) if Instant::now() < cleanup => thread::sleep(Duration::from_millis(10)),
                other => panic!("apply group survived: {other:?}"),
            }
        }
        result
    }
}
impl Drop for Parent {
    fn drop(&mut self) {
        // On an assertion failure the process owner must still get its original
        // deadline plus cleanup budget. Killing the parent early strands its
        // separately grouped apply child and destroys the failure evidence.
        let deadline = Instant::now() + Duration::from_secs(32);
        let settled = loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break true,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                _ => break false,
            }
        };
        let absent = fs::read_to_string(self.root().join("group"))
            .ok()
            .and_then(|group| group.parse::<i32>().ok())
            .is_none_or(|group| killpg(Pid::from_raw(group), None) == Err(Errno::ESRCH));
        if !settled || !absent {
            if !settled {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
            eprintln!(
                "Unconfirmed apply cleanup; retained {}",
                self.root().display()
            );
            std::mem::forget(self.root.take());
        }
    }
}

#[test]
fn parent_sigint_preserves_completed_work_and_stops_later_apply_items() {
    let mut parent = Parent::start("cooperative");
    parent.wait_for("ready");
    assert_eq!(
        fs::read(parent.root().join("ops.completed")).unwrap(),
        b"0.1.0-alpha.2"
    );
    parent.signal();
    let result = parent.finish(130);
    assert!(parent.root().join("signal-seen").exists());
    let rows = result["products"].as_array().unwrap();
    assert_eq!(
        rows.iter().find(|row| row["product"] == "ops").unwrap()["status"],
        "changed"
    );
    assert_eq!(
        rows.iter().find(|row| row["product"] == "digest").unwrap()["status"],
        "failed"
    );
    assert!(!parent.root().join("digest.started").exists());
    assert!(!parent.root().join("remote.completed").exists());
    assert!(parent.root().join("ops.completed").exists());
}

#[test]
fn repeated_parent_sigint_shortens_the_wait_without_replaying_apply() {
    let mut parent = Parent::start("stubborn");
    parent.wait_for("ready");
    parent.signal();
    parent.wait_for("signal-seen");
    parent.signal();
    let result = parent.finish(130);
    assert!(result["products"].as_array().unwrap().iter().any(|row| {
        row["message"]
            .as_str()
            .is_some_and(|message| message.contains("unconfirmed"))
    }));
    assert!(!parent.root().join("child-report").exists());
    assert!(!parent.root().join("digest.started").exists());
    assert!(parent.root().join("ops.completed").exists());
}

#[test]
fn an_uncooperative_apply_hits_its_original_deadline_and_reports_uncertainty() {
    let mut parent = Parent::start("deadline");
    parent.wait_for("ready");
    parent.signal();
    let result = parent.finish(130);
    assert!(result["products"].as_array().unwrap().iter().any(|row| {
        row["message"]
            .as_str()
            .is_some_and(|message| message.contains("unconfirmed"))
    }));
    assert!(!parent.root().join("child-report").exists());
    assert!(!parent.root().join("digest.started").exists());
    assert!(parent.root().join("ops.completed").exists());
}

#[test]
fn an_unsignalled_apply_completes_once_and_preserves_all_results() {
    let mut parent = Parent::start("success");
    let result = parent.finish(0);
    assert_eq!(
        result["products"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["status"] == "changed")
            .count(),
        3
    );
    for name in ["ops", "remote", "digest"] {
        assert!(parent.root().join(format!("{name}.completed")).exists());
    }
}

#[test]
fn cancellation_before_apply_starts_no_child_or_later_item() {
    let mut parent = Parent::start("before");
    let result = parent.finish(130);
    assert_eq!(fs::read(parent.root().join("applies")).unwrap(), b"0");
    assert!(!parent.root().join("ops.started").exists());
    assert!(!parent.root().join("child-report").exists());
    assert!(result["products"].as_array().unwrap().iter().any(|row| {
        row["message"]
            .as_str()
            .is_some_and(|message| message.contains("before extension apply"))
    }));
}

#[test]
fn a_signal_after_child_settlement_retains_results_and_interrupts_the_parent() {
    let mut parent = Parent::start("late");
    let result = parent.finish(130);
    assert_eq!(
        result["products"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["status"] == "changed")
            .count(),
        3
    );
    assert!(parent.root().join("digest.completed").exists());
}
