use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
#[path = "support/executable_fixture.rs"]
mod executable_fixture;

const BINARY: &str = env!("CARGO_BIN_EXE_tmt-remote");
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Pilot {
    root: PathBuf,
    child: Option<Child>,
}
impl Pilot {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-613-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let pilot = Self { root, child: None };
        executable_fixture::write_executable(&pilot.root.join("core"), &format!("printf '%s\\n' \"$*\" >> '{}/calls'\ncat > '{}/input'\nprintf '{{\"version\":1,\"limits\":{{\"inputBytes\":1024,\"outputBytes\":4096}}}}'\n", pilot.root.display(),pilot.root.display())).unwrap();
        pilot
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(BINARY);
        cmd.env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("TMT_EXECUTABLE", self.root.join("core"));
        cmd
    }
}
impl Drop for Pilot {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn help_missing_core_and_invalid_options() {
    let pilot = Pilot::new();
    let a = pilot.command().args(["serve", "-h"]).output().unwrap();
    let b = pilot.command().args(["help", "serve"]).output().unwrap();
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b.stdout);
    assert!(String::from_utf8(a.stdout).unwrap().contains("deny-all"));
    assert!(
        !pilot
            .command()
            .args(["serve", "--host", "0.0.0.0"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        !pilot
            .command()
            .args(["serve", "--window-seconds", "86401"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let missing = pilot
        .command()
        .env_remove("TMT_EXECUTABLE")
        .args(["serve", "--json"])
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(
        String::from_utf8(missing.stdout)
            .unwrap()
            .contains("REMOTE_CORE_UNAVAILABLE")
    );
    assert!(
        !pilot.root.join("calls").exists(),
        "help/options never start core"
    );
}
#[test]
fn startup_is_one_public_read_remote_calls_are_zero_and_sigterm_reaps() {
    for _ in 0..2 {
        let mut pilot = Pilot::new();
        let child = pilot
            .command()
            .args(["serve", "--json"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        pilot.child = Some(child);
        let pipe = pilot.child.as_mut().unwrap().stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut line = String::new();
            BufReader::new(pipe).read_line(&mut line).unwrap();
            let _ = tx.send(line);
        });
        let descriptor: Value =
            serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
        reader.join().unwrap();
        assert_eq!(descriptor["state"], "closed");
        assert_eq!(descriptor["startupCoreCalls"], 1);
        let address = descriptor["address"]
            .as_str()
            .unwrap()
            .strip_prefix("http://")
            .unwrap();
        let (socket, prefix) = address.split_once('/').unwrap();
        for origin in [
            "",
            "Origin: null\r\n",
            "Origin: https://evil.example\r\n",
            "Origin: chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n",
        ] {
            let mut client = TcpStream::connect(socket).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            write!(client,"POST /{prefix}/append HTTP/1.1\r\nHost: {socket}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n{origin}\r\n{{}}").unwrap();
            let mut response = String::new();
            client.read_to_string(&mut response).unwrap();
            assert!(response.starts_with("HTTP/1.1 404"));
        }
        assert_eq!(
            fs::read_to_string(pilot.root.join("calls")).unwrap(),
            "api\n"
        );
        let request: Value =
            serde_json::from_slice(&fs::read(pilot.root.join("input")).unwrap()).unwrap();
        assert_eq!(request["operation"], "capabilities");
        let child = pilot.child.as_mut().unwrap();
        assert!(
            Command::new("/bin/kill")
                .args(["-TERM", &child.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline, "foreground process leaked");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(TcpStream::connect(socket).is_err());
        assert!(
            !pilot.root.join("data").exists(),
            "no request adoption/core DB"
        );
    }
}
