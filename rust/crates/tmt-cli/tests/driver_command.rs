//! `tmt driver install|ls|rm` end to end (#570 slice 6): a shell-script
//! driver in a disposable home. A driver is approved only with consent:
//! `--yes`, or an answer on a terminal (typescript/test/e2e covers the
//! question); anything refused leaves the registry untouched.
#![cfg(unix)]

use serde_json::Value;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

/// Answers `capabilities` from the file beside it.
const DRIVER: &str = r#"#!/bin/sh
cat "$(dirname "$0")/capabilities"
"#;

fn capabilities(name: &str, prefix: &str, target: &str) -> String {
    format!(
        r#"{{"ok":{{"protocols":[1],"kind":"host","name":"{name}","version":"0.0.0-test","ops":["caller","server","snapshot","publish","clear"],"paneId":{{"prefix":"{prefix}"}},"target":"{target}","callerEnv":["FAKE_PANE_ID"]}}}}"#
    )
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("tmt-driver-cmd-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for dir in ["home", "xdg", "state", "tmux"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        Self { root }
    }

    /// A driver executable in its own directory, declaring `capabilities`.
    fn driver(&self, dir: &str, capabilities: &str) -> PathBuf {
        let dir = self.root.join(dir);
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let driver = dir.join("tmt-driver");
        fs::write(&driver, DRIVER).unwrap();
        fs::set_permissions(&driver, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(dir.join("capabilities"), capabilities).unwrap();
        driver
    }

    fn registry(&self) -> PathBuf {
        self.root.join("state/drivers.json")
    }

    fn tmt(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_tmt"))
            .args(args)
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("TMUX_TEAM_HOME", self.root.join("state"))
            .env("XDG_CONFIG_HOME", self.root.join("xdg"))
            .env("TMUX_TMPDIR", self.root.join("tmux"))
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env("LANG", "en_US.UTF-8")
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn json(&self, args: &[&str]) -> (i32, Value) {
        // JSON answers and JSON errors both print on stdout.
        let output = self.tmt(args);
        let value = serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("tmt {args:?}: {output:?}"));
        (output.status.code().unwrap(), value)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Everything a human run printed, on either stream. The style layer ends a
/// status or error line without a period.
fn printed(output: &Output) -> String {
    text(&output.stdout) + &text(&output.stderr)
}

fn path(value: &Path) -> &str {
    value.to_str().unwrap()
}

#[test]
fn yes_approves_after_showing_what_the_driver_declares() {
    let fixture = Fixture::new("yes");
    let driver = fixture.driver("fake", &capabilities("fake", "fake-", "f{n}"));
    let output = fixture.tmt(&["driver", "install", path(&driver), "--yes"]);
    assert!(output.status.success(), "{output:?}");
    let stdout = text(&output.stdout);
    let canonical = fs::canonicalize(&driver).unwrap();
    for line in [
        "Host driver fake".to_owned(),
        "  version     0.0.0-test".to_owned(),
        "  protocol    1".to_owned(),
        format!("  executable  {}", canonical.display()),
        "  operations  caller, server, snapshot, publish, clear".to_owned(),
        "  reads       FAKE_PANE_ID".to_owned(),
        "TMT will run this program for these operations whenever it works with fake panes."
            .to_owned(),
        "Approved host driver fake. Remove it with: tmt driver rm fake".to_owned(),
    ] {
        assert!(stdout.contains(&line), "missing {line:?} in:\n{stdout}");
    }
    assert!(!stdout.contains("[y/N]"));
    let (code, listed) = fixture.json(&["driver", "ls", "--json"]);
    assert_eq!(code, 0);
    assert_eq!(listed["drivers"][0]["name"], "fake");
    assert_eq!(listed["drivers"][0]["state"], "ok");
    assert_eq!(listed["drivers"][0]["path"], path(&canonical));
}

#[test]
fn without_yes_and_a_terminal_nothing_is_approved() {
    let fixture = Fixture::new("refuse");
    let driver = fixture.driver("fake", &capabilities("fake", "fake-", "f{n}"));
    let output = fixture.tmt(&["driver", "install", path(&driver)]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        printed(&output)
            .contains("Approving host driver fake requires explicit --yes; nothing changed")
    );
    // The user still sees what they would approve.
    assert!(text(&output.stdout).contains("  sha256      "));
    let (code, refused) = fixture.json(&["driver", "install", path(&driver), "--json"]);
    assert_eq!(code, 1);
    assert_eq!(refused["error"]["code"], "DRIVER_CONSENT_REQUIRED");
    assert!(!fixture.registry().exists());
}

#[test]
fn json_approval_reports_the_record() {
    let fixture = Fixture::new("json");
    let driver = fixture.driver("fake", &capabilities("fake", "fake-", "f{n}"));
    let (code, approved) = fixture.json(&["driver", "install", path(&driver), "--yes", "--json"]);
    assert_eq!(code, 0);
    let record = &approved["approved"];
    assert_eq!(record["name"], "fake");
    assert_eq!(record["protocol"], 1);
    assert_eq!(record["callerEnv"], serde_json::json!(["FAKE_PANE_ID"]));
    assert_eq!(record["sha256"].as_str().unwrap().len(), 64);
    assert!(record["approvedAtMs"].as_u64().unwrap() > 0);
}

#[test]
fn a_refused_driver_is_refused_before_any_question() {
    let fixture = Fixture::new("refused");
    let not_driver = fixture.driver("broken", "not json");
    // A built-in host's name: refused even with consent.
    let builtin = fixture.driver("builtin", &capabilities("herdr", "hd-", "h{n}"));
    let unsafe_driver = fixture.driver("open", &capabilities("open", "open-", "o{n}"));
    fs::set_permissions(&unsafe_driver, fs::Permissions::from_mode(0o777)).unwrap();
    for (driver, code) in [
        (&not_driver, "DRIVER_REFUSED"),
        (&builtin, "DRIVER_REFUSED"),
        (&unsafe_driver, "DRIVER_UNSAFE"),
    ] {
        let output = fixture.tmt(&["driver", "install", path(driver)]);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(!text(&output.stdout).contains("Host driver"), "{output:?}");
        let (_, refused) = fixture.json(&["driver", "install", path(driver), "--yes", "--json"]);
        assert_eq!(refused["error"]["code"], code, "{driver:?}");
    }
    assert!(!fixture.registry().exists());
}

#[test]
fn ls_shows_a_changed_or_missing_driver_with_its_fix() {
    let fixture = Fixture::new("ls");
    assert_eq!(
        text(&fixture.tmt(&["driver", "ls"]).stdout).trim(),
        "No host drivers are approved."
    );
    let changed = fixture.driver("changed", &capabilities("changed", "ch-", "c{n}"));
    let missing = fixture.driver("missing", &capabilities("missing", "mi-", "m{n}"));
    for driver in [&changed, &missing] {
        assert!(
            fixture
                .tmt(&["driver", "install", path(driver), "--yes"])
                .status
                .success()
        );
    }
    let mut script = fs::read_to_string(&changed).unwrap();
    script.push_str("# edited after approval\n");
    fs::write(&changed, script).unwrap();
    fs::remove_file(&missing).unwrap();
    let (_, listed) = fixture.json(&["driver", "ls", "--json"]);
    let states: Vec<(&str, &str)> = listed["drivers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|driver| {
            (
                driver["name"].as_str().unwrap(),
                driver["state"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(states, [("changed", "changed"), ("missing", "missing")]);
    // One section, a state mark per row, and re-approval as the row's action.
    let human = text(&fixture.tmt(&["driver", "ls"]).stdout);
    let lines: Vec<&str> = human.lines().collect();
    assert_eq!(lines[0], "HOST DRIVERS 2", "{human}");
    for (line, (mark, state, driver)) in lines[1..]
        .iter()
        .zip([("✗", "changed", &changed), ("○", "missing", &missing)])
    {
        let canonical = fs::canonicalize(driver.parent().unwrap())
            .unwrap()
            .join("tmt-driver");
        let words: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(words[0], mark, "{line}");
        assert_eq!(words[3], state, "{line}");
        assert!(
            line.ends_with(&format!("tmt driver install {}", canonical.display())),
            "{line}"
        );
    }
}

#[test]
fn rm_withdraws_an_approval_without_asking() {
    let fixture = Fixture::new("rm");
    let driver = fixture.driver("fake", &capabilities("fake", "fake-", "f{n}"));
    assert!(
        fixture
            .tmt(&["driver", "install", path(&driver), "--yes"])
            .status
            .success()
    );
    let output = fixture.tmt(&["driver", "rm", "fake"]);
    assert!(output.status.success(), "{output:?}");
    assert!(printed(&output).contains(
        "Removed host driver fake. Its bindings stay stored and read as unavailable until it is approved again"
    ));
    let (_, listed) = fixture.json(&["driver", "ls", "--json"]);
    assert_eq!(listed["drivers"], serde_json::json!([]));
    let output = fixture.tmt(&["driver", "rm", "fake"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(printed(&output).contains("No host driver named fake is approved"));
    let (code, missing) = fixture.json(&["driver", "rm", "fake", "--json"]);
    assert_eq!(code, 1);
    assert_eq!(missing["error"]["code"], "DRIVER_NOT_FOUND");
}

/// Approval waits past the protocol's one-second `capabilities` bound, so a
/// driver slow to start on a busy machine isn't refused as "not a driver".
#[test]
fn a_driver_slow_to_start_is_still_approved() {
    let fixture = Fixture::new("slow");
    let driver = fixture.driver("slow", &capabilities("slow", "slow-", "s{n}"));
    fs::write(
        &driver,
        "#!/bin/sh\nsleep 1.5\ncat \"$(dirname \"$0\")/capabilities\"\n",
    )
    .unwrap();
    let output = fixture.tmt(&["driver", "install", path(&driver), "--yes"]);
    assert!(output.status.success(), "{output:?}");
}

/// A first-party driver ships with a managed native installation; a dev
/// build has none, and says how to approve an executable instead.
#[test]
fn a_first_party_name_outside_a_managed_install_is_refused_with_the_path_form() {
    let fixture = Fixture::new("first-party");
    let (code, refused) = fixture.json(&["driver", "install", "herdr", "--yes", "--json"]);
    assert_eq!(code, 1);
    assert_eq!(refused["error"]["code"], "DRIVER_REFUSED");
    let message = refused["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("ships only with a managed native installation")
            && message.contains("tmt driver install <path>"),
        "{message}"
    );
    assert!(!fixture.registry().exists());
}
