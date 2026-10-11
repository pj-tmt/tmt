//! Discovery refuses a fake host world before touching application state.
#![cfg(unix)]
mod support;
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    root: PathBuf,
    child: Option<Child>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-isolation-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self { root, child: None }
    }
    fn run(&mut self, mut command: Command) -> Output {
        let stdout = self.root.join("stdout");
        let stderr = self.root.join("stderr");
        self.child = Some(
            command
                .stdin(Stdio::null())
                .stdout(fs::File::create(&stdout).unwrap())
                .stderr(fs::File::create(&stderr).unwrap())
                .spawn()
                .unwrap(),
        );
        let status = support::wait(&mut self.child, Duration::from_secs(15))
            .wait()
            .unwrap();
        Output {
            status,
            stdout: fs::read(stdout).unwrap(),
            stderr: fs::read(stderr).unwrap(),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        support::stop(&mut self.child);
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn dependency_compiled_cli_refuses_unadmitted_existing_fake_host_state() {
    let mut f = Fixture::new();
    let host = f.root.join("fake-host");
    let state = support::state_dir(&host);
    fs::create_dir_all(&state).unwrap();
    let sentinel = state.join("config.json");
    fs::write(&sentinel, b"{\"theme\":{\"base\":\"mono\"}}\n").unwrap();
    let mut command = support::command(&host, &["config", "show", "--json"]);
    command.env_remove("TMT_TEST_ROOT");
    let output = f.run(command);
    assert!(
        !output.status.success(),
        "a missing admission must refuse, even when the destination exists"
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("built with test isolation"), "{text}");
    assert!(text.contains("private test root"), "{text}");
    assert_eq!(
        fs::read(sentinel).unwrap(),
        b"{\"theme\":{\"base\":\"mono\"}}\n"
    );
    assert_eq!(fs::read_dir(state).unwrap().count(), 1);
}

#[test]
fn private_discovery_accepts_missing_paths_but_refuses_symlink_and_environment_escape() {
    use std::os::unix::fs::symlink;
    let mut f = Fixture::new();
    let private = f.root.join("private");
    let mut command = support::command(&private, &["config", "show", "--json"]);
    command.env("TMT_TEST_ROOT", &private);
    assert!(f.run(command).status.success());
    assert!(!support::state_dir(&private).exists());
    let host = f.root.join("fake-host");
    fs::create_dir(&host).unwrap();
    fs::write(host.join("sentinel"), b"unchanged").unwrap();
    symlink(&host, private.join("escape")).unwrap();
    for selected in [host.clone(), private.join("escape/missing")] {
        let mut command = support::command(&private, &["config", "show", "--json"]);
        command
            .env("TMT_TEST_ROOT", &private)
            .env("TMT_HOME", selected);
        let output = f.run(command);
        assert!(
            !output.status.success(),
            "escaped selected root must refuse"
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("test isolation"));
    }
    let mut command = support::command(&private, &["config", "show", "--json"]);
    command.env("TMT_TEST_ROOT", &private).env("HOME", &host);
    assert!(!f.run(command).status.success());
    symlink(host.join("absent"), private.join("dangling")).unwrap();
    let mut command = support::command(&private, &["config", "show", "--json"]);
    command
        .env("TMT_TEST_ROOT", &private)
        .env("TMT_HOME", private.join("dangling/child"));
    assert!(!f.run(command).status.success());
    fs::create_dir_all(private.join("deep/a/b")).unwrap();
    symlink(private.join("deep/a/b"), private.join("deep-alias")).unwrap();
    let mut command = support::command(&private, &["config", "show", "--json"]);
    command
        .env("TMT_TEST_ROOT", &private)
        .env("TMT_HOME", private.join("deep-alias/../../outside"));
    assert!(
        !f.run(command).status.success(),
        "derived lexical paths must stay private too"
    );
    let mut command = support::command(
        &private,
        &["config", "set", "theme.base", "mono", "--global", "--json"],
    );
    command.env("TMT_TEST_ROOT", &private);
    assert!(f.run(command).status.success());
    let value: serde_json::Value = serde_json::from_slice(
        &fs::read(support::state_dir(&private).join("config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(value["theme"]["base"], "mono");
    assert_eq!(fs::read(host.join("sentinel")).unwrap(), b"unchanged");
    assert_eq!(fs::read_dir(host).unwrap().count(), 1);
}

#[test]
fn private_discovery_never_reads_local_config_above_the_admitted_root() {
    let mut f = Fixture::new();
    fs::write(f.root.join("tmt.json"), b"not JSON").unwrap();
    let private = f.root.join("private");
    let mut command = support::command(&private, &["config", "show", "--json"]);
    command.env("TMT_TEST_ROOT", &private);
    let output = f.run(command);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(fs::read(f.root.join("tmt.json")).unwrap(), b"not JSON");
}
