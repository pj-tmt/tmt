//! The built driver executable, as core runs it: the protocol conformance
//! harness over a stand-in `herdr` on `PATH`, and the same driver against a
//! real, disposable Herdr server when `TMT_TEST_HERDR` names a pinned binary.

use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tmt_driver_protocol::conformance::{self, DriverOutput, Fixture};

const DRIVER: &str = env!("CARGO_BIN_EXE_tmt-driver-herdr");

/// A private directory removed on drop; the path is never a variable a
/// shell expands.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        // macOS caps a socket path at 104 bytes, so this stays short.
        let path = PathBuf::from(format!("/tmp/tmt-dh-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Runs the driver once with exactly `env`, the way core's guard does.
fn invoke(env: &[(&str, String)], args: &[&str], request: &[u8]) -> DriverOutput {
    let started = Instant::now();
    let mut child = Command::new(DRIVER)
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(name, value)| (*name, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(request).unwrap();
    let output = child.wait_with_output().unwrap();
    DriverOutput {
        stdout: output.stdout,
        elapsed: started.elapsed(),
    }
}

fn call(env: &[(&str, String)], op: &str, mut request: Value) -> Value {
    request["deadlineMs"] = json!(10_000);
    let output = invoke(
        env,
        &["__tmt-driver", "1", op],
        request.to_string().as_bytes(),
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// A stand-in `herdr` that knows no pane and records every environment it
/// was started with.
fn stand_in(scratch: &Scratch) -> PathBuf {
    let bin = scratch.path("bin");
    fs::create_dir_all(&bin).unwrap();
    let log = scratch.path("env.log");
    let script = format!(
        r#"#!/bin/sh
env >> '{log}'
case "$1 $2" in
  "status server") printf '{{"running":true,"version":"0.9.1","socket":"%s"}}' "$HERDR_SOCKET_PATH" ;;
  "pane list") printf '{{"result":{{"panes":[]}}}}' ;;
  *) printf '{{"error":{{"code":"pane_not_found","message":"no pane"}}}}' >&2; exit 1 ;;
esac
"#,
        log = log.display()
    );
    let herdr = bin.join("herdr");
    fs::write(&herdr, script).unwrap();
    let mut permissions = fs::metadata(&herdr).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    fs::set_permissions(&herdr, permissions).unwrap();
    bin
}

#[test]
fn the_driver_conforms_and_its_children_never_see_the_call_guard() {
    let scratch = Scratch::new("fake");
    let bin = stand_in(&scratch);
    let env = [
        ("PATH", format!("{}:/usr/bin:/bin", bin.display())),
        ("TMT_DRIVER_CALL", "guard-token".into()),
        ("UNRELATED_SECRET", "kept-out".into()),
    ];
    let fixture = Fixture {
        socket: scratch.path("s.sock").display().to_string(),
    };
    // The harness allows 250 ms per call, and macOS checks a freshly
    // written executable on its first run; start the stand-in once first.
    call(&env, "server", json!({"socket": fixture.socket}));
    let findings = conformance::check(&mut |args, request| invoke(&env, args, request), &fixture);
    assert_eq!(findings, []);
    let log = fs::read_to_string(scratch.path("env.log")).unwrap();
    assert!(log.contains("HERDR_SOCKET_PATH="), "herdr was asked");
    assert!(!log.contains("TMT_DRIVER_CALL"));
    assert!(!log.contains("UNRELATED_SECRET"));
}

/// An agent stand-in Herdr recognizes by its name (never a real agent). Like
/// an agent's TUI it turns on bracketed paste, and it prints each submitted
/// line's raw bytes, so a paste shows as `033 [ 2 0 0 ~`.
const STAND_IN_AGENT: &str = r#"#!/bin/sh
printf '\033[?2004h'
stty -echo
while IFS= read -r line; do printf '%s' "$line" | od -c | sed -n 1p; done
"#;

/// A headless Herdr server on a private socket and HOME, stopped on drop.
struct Server {
    herdr: PathBuf,
    env: Vec<(&'static str, String)>,
    process: Option<std::process::Child>,
    scratch: Scratch,
}

impl Server {
    fn start(herdr: &Path) -> Self {
        let scratch = Scratch::new("real");
        let home = scratch.path("home");
        let config = home.join(".config/herdr");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("config.toml"),
            "[update]\nversion_check = false\nmanifest_check = false\n",
        )
        .unwrap();
        // Panes inherit this PATH, so `claude` in a pane is the stand-in.
        let agents = scratch.path("agents");
        fs::create_dir_all(&agents).unwrap();
        let agent = agents.join("claude");
        fs::write(&agent, STAND_IN_AGENT).unwrap();
        let mut permissions = fs::metadata(&agent).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
        fs::set_permissions(&agent, permissions).unwrap();
        let env = vec![
            (
                "PATH",
                format!(
                    "{}:{}:/usr/bin:/bin",
                    agents.display(),
                    herdr.parent().unwrap().display()
                ),
            ),
            ("HOME", home.display().to_string()),
            (
                "XDG_CONFIG_HOME",
                home.join(".config").display().to_string(),
            ),
            ("SHELL", "/bin/sh".into()),
            (
                "HERDR_SOCKET_PATH",
                scratch.path("s.sock").display().to_string(),
            ),
        ];
        let mut server = Self {
            herdr: herdr.into(),
            env,
            process: None,
            scratch,
        };
        let process = Command::new(&server.herdr)
            .arg("server")
            .env_clear()
            .envs(server.env.iter().map(|(name, value)| (*name, value)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        server.process = Some(process);
        server.until("the server to run", |server| {
            server
                .run(&["status", "server"])
                .contains("status: running")
        });
        server
    }

    fn socket(&self) -> String {
        self.scratch.path("s.sock").display().to_string()
    }

    fn run(&self, args: &[&str]) -> String {
        let output = Command::new(&self.herdr)
            .args(args)
            .env_clear()
            .envs(self.env.iter().map(|(name, value)| (*name, value)))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.run(args)).unwrap()
    }

    fn until(&self, what: &str, check: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !check(self) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Server processes still running from this server's binary and socket.
    fn leftovers(&self) -> usize {
        let ps = Command::new("ps")
            .args(["-A", "-o", "pid=,args="])
            .output()
            .unwrap();
        let server = format!("{} server", self.herdr.display());
        String::from_utf8_lossy(&ps.stdout)
            .lines()
            .filter(|line| line.contains(&server))
            .count()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.run(&["server", "stop"]);
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.leftovers() > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        // Reap the server this test started; it exits once stopped.
        let exited = self.process.as_mut().is_some_and(|process| {
            while Instant::now() < deadline {
                if process.try_wait().ok().flatten().is_some() {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            false
        });
        if !std::thread::panicking() {
            assert!(exited, "the Herdr server did not exit");
            assert_eq!(self.leftovers(), 0, "a Herdr server process remained");
        }
    }
}

#[test]
fn the_driver_reads_and_marks_a_real_herdr_server() {
    let Some(herdr) = std::env::var_os("TMT_TEST_HERDR") else {
        eprintln!("skipped: TMT_TEST_HERDR names no pinned herdr binary");
        return;
    };
    let server = Server::start(Path::new(&herdr));
    let socket = server.socket();
    // The driver gets only what core passes it: no socket of its own.
    let env: Vec<(&str, String)> = server
        .env
        .iter()
        .filter(|(name, _)| *name != "HERDR_SOCKET_PATH")
        .cloned()
        .collect();

    // A fresh server has no pane to prove its process with.
    assert_eq!(
        call(&env, "server", json!({"socket": socket}))["ok"]["server"],
        json!(null)
    );
    server.json(&["workspace", "create", "--cwd", "/tmp"]);
    let listed = server.json(&["pane", "list"]);
    let terminal = listed["result"]["panes"][0]["terminal_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let target = listed["result"]["panes"][0]["pane_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let shell = server.json(&["pane", "process-info", "--pane", &target])["result"]["process_info"]
        ["shell_pid"]
        .as_u64()
        .unwrap();

    let incarnation = call(&env, "server", json!({"socket": socket}))["ok"]["server"].clone();
    assert_eq!(incarnation["socket"], socket.as_str());
    let pid = incarnation["pid"].as_u64().unwrap();
    // Core's own observation agrees: the server is the shell's parent.
    let parent = Command::new("ps")
        .args(["-o", "ppid=", "-p", &shell.to_string()])
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&parent.stdout).trim(),
        pid.to_string()
    );

    let resolved = call(
        &env,
        "resolve-target",
        json!({"socket": socket, "target": target}),
    );
    assert_eq!(resolved["ok"]["paneId"], terminal.as_str());
    let missing = call(
        &env,
        "resolve-target",
        json!({"socket": socket, "target": "w9:p9"}),
    );
    assert_eq!(missing["ok"]["paneId"], json!(null));

    let caller = call(
        &env,
        "caller",
        json!({"env": {"HERDR_PANE_ID": target, "HERDR_SOCKET_PATH": socket}}),
    );
    assert_eq!(
        caller["ok"]["pane"],
        json!({"id": terminal, "socket": socket, "shellPid": shell})
    );

    // The built-in host's fixture marker, published and read back as stored.
    let fixture: Value =
        serde_json::from_str(include_str!("../src/fixtures/builtin-marker.json")).unwrap();
    let marker = fixture["marker"].clone();
    let publish = |pane_pid: u64| {
        call(
            &env,
            "publish",
            json!({"socket": socket, "paneId": terminal, "panePid": pane_pid, "marker": marker}),
        )
    };
    assert_eq!(publish(shell + 1)["error"]["code"], "not_found");
    assert_eq!(publish(shell), json!({"ok": {}}));
    let snapshot = call(
        &env,
        "snapshot",
        json!({"socket": socket, "panes": [terminal]}),
    );
    let pane = &snapshot["ok"]["panes"][0];
    assert_eq!(pane["id"], terminal.as_str());
    assert_eq!(pane["target"], target.as_str());
    assert_eq!(pane["panePid"], shell);
    assert_eq!(pane["marker"], marker);
    // Herdr holds exactly the built-in host's tokens.
    let tokens = &server.json(&["pane", "get", &target])["result"]["pane"]["tokens"];
    for token in fixture["tokens"].as_array().unwrap() {
        let (key, value) = token.as_str().unwrap().split_once('=').unwrap();
        assert_eq!(tokens[key], value, "{key}");
    }

    let clear = |binding: &str| {
        call(
            &env,
            "clear",
            json!({"socket": socket, "paneId": terminal, "bindingId": binding}),
        )["ok"]["cleared"]
            .clone()
    };
    assert_eq!(clear("123e4567-e89b-42d3-a456-426614174099"), false);
    assert_eq!(clear(marker["bindingId"].as_str().unwrap()), true);
    let snapshot = call(&env, "snapshot", json!({"socket": socket, "panes": null}));
    assert_eq!(snapshot["ok"]["panes"][0]["marker"], json!(null));

    // Staged input as core sends it: the line, then Enter alone. The line
    // runs only after Enter; a line break is refused with nothing typed.
    let input = |text: &str, enter: bool| {
        call(
            &env,
            "input",
            json!({"socket": socket, "paneId": terminal, "text": text, "enter": enter}),
        )
    };
    let capture = || {
        call(
            &env,
            "capture",
            json!({"socket": socket, "paneId": terminal, "lines": 20}),
        )["ok"]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let until = |what: &str, check: &dyn Fn(&str) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let text = capture();
            if check(&text) {
                return text;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}:\n{text}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    assert_eq!(
        input("echo staged\necho early", false)["error"]["code"],
        "bad_request"
    );
    assert_eq!(input("echo -- tmt-$((40+2))", false), json!({"ok": {}}));
    let typed = until("the typed line", &|text| {
        text.contains("echo -- tmt-$((40+2))")
    });
    assert!(
        !typed.contains("tmt-42"),
        "the line ran before Enter:\n{typed}"
    );
    assert!(!typed.contains("early"));
    assert_eq!(input("", true), json!({"ok": {}}));
    until("the line to run", &|text| text.contains("tmt-42"));

    // A plain shell has no agent: core may type instead, and nothing was.
    let prompt = |pane: &str, text: &str| {
        call(
            &env,
            "prompt",
            json!({"socket": socket, "paneId": pane, "text": text}),
        )
    };
    assert_eq!(prompt(&terminal, "hello")["error"]["code"], "no_agent");
    assert!(!capture().contains("hello"));

    // An agent takes the whole message as one paste, which Herdr submits.
    server.json(&["workspace", "create", "--cwd", "/tmp"]);
    let agent_pane = server.json(&["pane", "get", "w2:p1"])["result"]["pane"]["terminal_id"]
        .as_str()
        .unwrap()
        .to_owned();
    server.run(&["pane", "run", "w2:p1", "claude"]);
    server.until("Herdr to see the agent", |server| {
        server.json(&["pane", "get", "w2:p1"])["result"]["pane"]["agent"] == "claude"
    });
    assert_eq!(prompt(&agent_pane, "one\ntwo"), json!({"ok": {}}));
    let read = || {
        call(
            &env,
            "capture",
            json!({"socket": socket, "paneId": agent_pane, "lines": 20}),
        )["ok"]["text"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while !read().contains("t   w   o 033   [   2   0   1   ~") {
        assert!(Instant::now() < deadline, "no submitted paste:\n{}", read());
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(read().contains("033   [   2   0   0   ~   o   n   e"));
    // Option-like text is the message, not an option of Herdr's command:
    // typed and prompted intact (Herdr 0.9.1 has no `--` separator).
    assert_eq!(input("-h --version", false), json!({"ok": {}}));
    until("option-like text", &|text| text.contains("-h --version"));
    assert_eq!(input("", true), json!({"ok": {}}));
    assert_eq!(prompt(&agent_pane, "--wait"), json!({"ok": {}}));
    let deadline = Instant::now() + Duration::from_secs(10);
    // `-`, `-`, `w`: the stand-in's raw bytes of the pasted `--wait`.
    while !read().contains("-   -   w   a   i   t") {
        assert!(Instant::now() < deadline, "no --wait paste:\n{}", read());
        std::thread::sleep(Duration::from_millis(50));
    }
    // An agent waiting on its user refuses, and nothing more is typed.
    server.run(&[
        "pane",
        "report-agent",
        "w2:p1",
        "--source",
        "tmt-test",
        "--agent",
        "claude",
        "--state",
        "blocked",
    ]);
    let before = read();
    assert_eq!(prompt(&agent_pane, "more")["error"]["code"], "blocked");
    assert_eq!(read(), before);

    let findings = conformance::check(
        &mut |args, request| invoke(&env, args, request),
        &Fixture {
            socket: socket.clone(),
        },
    );
    assert_eq!(findings, []);
}
