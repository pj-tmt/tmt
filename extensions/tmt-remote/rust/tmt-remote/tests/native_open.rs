//! Core opener diagnostic canary: the URL remains a plain private argument, never output.
#[allow(dead_code)]
#[path = "support/door.rs"]
mod door;

#[test]
fn native_extension_launch_is_versioned_short_only_and_preserves_suppression() {
    use std::{fs, os::unix::fs::symlink, process::Command};
    let h = door::Harness::new(tmt_remote::pairing::Timing::CONTRACT);
    let core = door::core_fixture::Core::new();
    fs::write(core.root.join("storage-root"), h.root.to_str().unwrap()).unwrap();
    let bin = h.root.join("bin");
    fs::create_dir(&bin).unwrap();
    for name in ["open", "xdg-open"] {
        symlink("/usr/bin/printf", bin.join(name)).unwrap();
    }
    let call = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_tmt-remote"))
            .args(args)
            .env_clear()
            .env("HOME", &h.root)
            .env("XDG_CONFIG_HOME", &h.root)
            .env("PATH", &bin)
            .env("TMT_EXECUTABLE", core.root.join("tmt"))
            .env("TMT_AGENT", "1")
            .env("BROWSER", "true")
            .output()
            .unwrap()
    };
    for args in [
        vec!["launch", "/settings", "--no-open"],
        vec!["launch", "/settings"],
        vec!["open", "/settings", "--open", "--json"],
    ] {
        let output = call(&args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"version":1,"link":format!("{}/settings", h.origin),"opened":false,"state":"skipped"})
        );
        assert!(h.pairing.descriptor(&h.origin).is_none());
    }
    let output = call(&["open", "--no-open"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!("✓ Open this link in your browser: {}/", h.origin)
    );
    assert!(h.pairing.descriptor(&h.origin).is_none());
    assert!(
        !call(&["launch", "--no-open"]).status.success(),
        "extension target remains required"
    );
    let output = call(&["launch", "/settings", "--json"]);
    assert!(
        !output.status.success(),
        "seam accepts no --json; skew stays an ordinary usage refusal"
    );
    assert!(h.pairing.descriptor(&h.origin).is_none());
    let output = call(&["launch", "https://foreign.example/", "--open"]);
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(error["version"], 1);
    assert_eq!(error["error"]["code"], "REMOTE_INPUT_INVALID");
    assert!(h.pairing.descriptor(&h.origin).is_none());
    let output = call(&["launch", "/settings", "--open"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"version":1,"link":format!("{}/settings", h.origin),"opened":true,"state":"opened"})
    );
    for bytes in [&output.stdout, &output.stderr] {
        assert!(!String::from_utf8_lossy(bytes).contains("#auto="));
    }
    assert_eq!(
        h.pairing.descriptor(&h.origin).unwrap()["target"],
        "/settings"
    );
    for name in ["open", "xdg-open"] {
        fs::remove_file(bin.join(name)).unwrap();
        symlink("/usr/bin/false", bin.join(name)).unwrap();
    }
    let output = call(&["open", "/settings", "--open"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        format!(
            "warning: Could not open the browser. Open {}/settings yourself",
            h.origin
        ),
        "the shared human printer omits the final period"
    );
    assert!(h.pairing.descriptor(&h.origin).is_none());
    let (manual, events) = h.pairing.open().unwrap();
    let descriptor = h.pairing.descriptor(&h.origin).unwrap();
    for succeeds in [false, true] {
        for name in ["open", "xdg-open"] {
            fs::remove_file(bin.join(name)).unwrap();
            symlink(
                if succeeds {
                    "/usr/bin/printf"
                } else {
                    "/usr/bin/false"
                },
                bin.join(name),
            )
            .unwrap();
        }
        for command in ["launch", "open"] {
            let output = call(&[command, "/settings", "--open"]);
            assert!(
                output.status.success(),
                "{command}: {}",
                String::from_utf8_lossy(&output.stdout)
            );
            if command == "launch" {
                let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(
                    value,
                    serde_json::json!({"version":1,"link":format!("{}/settings",h.origin),"opened":succeeds,"state":if succeeds {"opened"} else {"failed"}})
                );
            } else {
                let line = String::from_utf8(output.stdout).unwrap();
                assert_eq!(
                    line.trim(),
                    if succeeds {
                        format!("✓ Opened {}/settings", h.origin)
                    } else {
                        format!(
                            "warning: Could not open the browser. Open {}/settings yourself",
                            h.origin
                        )
                    }
                );
            }
            assert_eq!(h.pairing.descriptor(&h.origin).unwrap(), descriptor);
            assert!(matches!(
                events.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Empty)
            ));
            assert_eq!(h.store.lock().unwrap().grants().unwrap().len(), 0);
        }
    }
    h.pairing.refuse(&manual.offer_id);
    assert!(matches!(
        events
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap(),
        tmt_remote::pairing::PairingEvent::Ended(tmt_remote::pairing::End::Refused)
    ));
    assert!(
        core.calls()
            .iter()
            .all(|call| call["operation"] == "storage.root")
    );
}
#[test]
fn core_opener_streams_json_errors_and_logs_never_echo_the_launch_url() {
    let marker = "AUTOOPEN_CANARY_DO_NOT_LOG";
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "core_opener_canary_child", "--nocapture"])
        .env("TMT_REMOTE_OPEN_CANARY_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: serde_json::Value = serde_json::from_str(
        stdout
            .lines()
            .find(|line| line.starts_with('{'))
            .expect("child emitted the real opener outcomes"),
    )
    .unwrap();
    assert_eq!(result["success"], "Opened");
    assert!(result["failure"].as_str().unwrap().starts_with("Failed("));
    for bytes in [output.stdout, output.stderr] {
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            !text.contains(marker) && !text.contains("#auto="),
            "ephemeral URL leaked: {text}"
        );
    }
}

#[test]
fn core_opener_canary_child() {
    if std::env::var_os("TMT_REMOTE_OPEN_CANARY_CHILD").is_none() {
        return;
    }
    let url = "http://127.0.0.1:1/pair#auto=AUTOOPEN_CANARY_DO_NOT_LOG";
    // printf tries to echo its argument. launch's null streams must suppress both outputs.
    let success = tmt_invoke::open::launch(std::path::Path::new("/usr/bin/printf"), url);
    let failure = tmt_invoke::open::launch(std::path::Path::new("/no-such-tmt-opener"), url);
    println!(
        "{}",
        serde_json::json!({"success":format!("{success:?}"),"failure":format!("{failure:?}")})
    );
}
