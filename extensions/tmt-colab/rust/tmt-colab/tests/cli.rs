use nix::{
    errno::Errno,
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
const BINARY: &str = env!("CARGO_BIN_EXE_tmt-colab");
struct Pilot {
    root: PathBuf,
    child: Option<Child>,
    reader: Option<JoinHandle<()>>,
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
impl Pilot {
    fn new(reply: Option<&str>) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-847-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let pilot = Self {
            root,
            child: None,
            reader: None,
        };
        let response = reply
            .map(str::to_owned)
            .unwrap_or_else(|| json!({"dataRoot":pilot.root.join("selected")}).to_string());
        let payload = format!(
            "#!/bin/sh\nif [ \"$1\" = __fixture_ready ]; then exit 0; fi\n[ \"$#\" = 1 ] && [ \"$1\" = api ] || exit 9\ncd {} || exit 9\nprintf '%s\\n' \"$*\" >> calls\ncat > input\nprintf '%s\\n' {}\n",
            quote(pilot.root.to_str().unwrap()),
            quote(&response)
        );
        fs::write(pilot.root.join("core"), payload).unwrap();
        fs::set_permissions(pilot.root.join("core"), fs::Permissions::from_mode(0o700)).unwrap();
        // Probe only the no-effect fixture branch, never retry the product invocation.
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match Command::new(pilot.root.join("core"))
                .arg("__fixture_ready")
                .output()
            {
                Ok(output) => {
                    assert!(output.status.success());
                    break;
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::ExecutableFileBusy
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(e) => panic!("fixture publication failed: {e}"),
            }
        }
        assert!(!pilot.root.join("calls").exists());
        pilot
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(BINARY);
        cmd.env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("TMUX_TEAM_HOME", self.root.join("selected"))
            .env("TMT_EXECUTABLE", self.root.join("core"));
        cmd
    }
    fn call(&self, args: &[&str]) -> Value {
        let output = self.command().args(args).output().unwrap();
        assert!(output.status.success(), "{:?}", output);
        assert!(output.stderr.is_empty());
        assert!(!output.stdout.contains(&0x1b));
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn start(&mut self) -> Value {
        self.child = Some(
            self.command()
                .args(["serve", "--port", "0", "--json"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let pipe = self.child.as_mut().unwrap().stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        self.reader = Some(thread::spawn(move || {
            let mut line = String::new();
            BufReader::new(pipe).read_line(&mut line).unwrap();
            let _ = tx.send(line);
        }));
        let line = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        self.reader.take().unwrap().join().unwrap();
        assert!(!line.contains('\u{1b}'));
        serde_json::from_str(&line).unwrap()
    }
    fn stop(&mut self) {
        let child = self.child.as_mut().unwrap();
        let pid = Pid::from_raw(child.id() as i32);
        kill(pid, Signal::SIGTERM).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match child.try_wait().unwrap() {
                Some(status) => {
                    assert!(status.success());
                    break;
                }
                None => {
                    assert!(Instant::now() < deadline, "foreground process did not stop");
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
        self.child.take();
        assert_eq!(
            kill(pid, None),
            Err(Errno::ESRCH),
            "foreground process leaked"
        );
    }
}
impl Drop for Pilot {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            child.wait().unwrap();
        }
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
        fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn help_and_invalid_core_fail_without_creating_application_state() {
    let pilot = Pilot::new(None);
    let short = pilot.command().args(["serve", "-h"]).output().unwrap();
    let long = pilot.command().args(["help", "serve"]).output().unwrap();
    assert!(short.status.success() && long.status.success());
    assert_eq!(short.stdout, long.stdout);
    assert!(String::from_utf8_lossy(&short.stdout).contains("default 7341"));
    for args in [["serve", "--bind", "0.0.0.0"], ["serve", "--port", "65536"]] {
        assert!(
            !pilot
                .command()
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let invalid = pilot
        .command()
        .args(["serve", "--port", "65536", "--json"])
        .output()
        .unwrap();
    assert!(!invalid.status.success() && invalid.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&invalid.stdout).unwrap()["error"]["code"],
        "COLAB_INPUT_INVALID"
    );
    for supplied in [None, Some("relative-core")] {
        let mut cmd = pilot.command();
        match supplied {
            None => {
                cmd.env_remove("TMT_EXECUTABLE");
            }
            Some(path) => {
                cmd.env("TMT_EXECUTABLE", path);
            }
        }
        let output = cmd
            .args(["serve", "--port", "0", "--json"])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
            "COLAB_UNAVAILABLE"
        );
        assert!(output.stderr.is_empty());
    }
    assert!(!pilot.root.join("calls").exists());
    assert!(!pilot.root.join("selected").exists());
    for (reply, code) in [
        ("{}", "COLAB_UNAVAILABLE"),
        ("{\"dataRoot\":\"relative\"}", "COLAB_ROOT_INVALID"),
        (
            "{\"error\":{\"code\":\"API_OPERATION_UNKNOWN\"}}",
            "COLAB_UNAVAILABLE",
        ),
    ] {
        let bad = Pilot::new(Some(reply));
        let output = bad.command().args(["spaces", "--json"]).output().unwrap();
        assert!(!output.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["error"]["code"],
            code
        );
        assert!(!bad.root.join("selected").exists());
    }
}
#[test]
fn listing_is_read_only_and_foreground_shutdown_releases_state_and_sockets_twice() {
    for _ in 0..2 {
        let mut pilot = Pilot::new(None);
        assert_eq!(pilot.call(&["spaces", "--json"]), json!({"spaces":[]}));
        assert!(!pilot.root.join("selected").exists());
        let descriptor = pilot.start();
        let id = descriptor["spaceId"].as_str().unwrap();
        assert_eq!(descriptor["state"], "unauthenticated");
        assert_eq!(
            fs::read_to_string(pilot.root.join("calls")).unwrap(),
            "api\napi\n"
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(pilot.root.join("input")).unwrap()).unwrap(),
            json!({"version":1,"operation":"storage.root","input":{}})
        );
        let address = descriptor["url"]
            .as_str()
            .unwrap()
            .strip_prefix("http://")
            .unwrap()
            .trim_end_matches('/');
        let mut socket = TcpStream::connect(address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        socket
            .write_all(format!("GET / HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())
            .unwrap();
        let mut page = String::new();
        socket.read_to_string(&mut page).unwrap();
        assert!(page.starts_with("HTTP/1.1 200") && page.contains("TMT Colab"));
        assert_eq!(
            fs::read_to_string(pilot.root.join("calls")).unwrap(),
            "api\napi\n",
            "door invoked core"
        );
        assert_eq!(
            pilot.call(&["spaces", "--json"]),
            json!({"spaces":[{"spaceId":id,"backend":"local","running":true}]})
        );
        let duplicate = pilot
            .command()
            .args(["serve", "--port", "0", "--json"])
            .output()
            .unwrap();
        assert!(!duplicate.status.success());
        assert_eq!(
            serde_json::from_slice::<Value>(&duplicate.stdout).unwrap()["error"]["code"],
            "COLAB_ALREADY_SERVING"
        );
        let owner = pilot.root.join("selected/colab/owner.key");
        let saved = fs::read(&owner).unwrap();
        pilot.stop();
        assert!(TcpStream::connect(address).is_err(), "listener leaked");
        assert_eq!(
            pilot.call(&["spaces", "--json"]),
            json!({"spaces":[{"spaceId":id,"backend":"local","running":false}]})
        );
        assert_eq!(fs::read(owner).unwrap(), saved);
        assert_eq!(
            fs::read_dir(pilot.root.join("selected")).unwrap().count(),
            1,
            "created core files"
        );
    }
}

#[test]
fn occupied_port_fails_with_a_port_hint() {
    let pilot = Pilot::new(None);
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let output = pilot
        .command()
        .args(["serve", "--port", &port, "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let error: Value = serde_json::from_slice(&output.stdout).unwrap();
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains(&format!("port {port} is busy")));
    assert!(message.contains("--port 0"));
    // Failure releases the service lock and leaves an explicit free-port retry usable.
    let mut pilot = pilot;
    pilot.start();
    pilot.stop();
}
