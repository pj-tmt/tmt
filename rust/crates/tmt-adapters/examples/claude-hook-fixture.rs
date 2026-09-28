//! Deterministic runtime-shaped parent for isolated hook lifecycle tests.
//! No provider, credentials, transcript access or model calls are involved.

use serde::Deserialize;
use serde_json::json;
use std::{
    fs,
    io::{BufRead, Write},
    process::{Command, Stdio},
};

#[derive(Deserialize)]
struct Step {
    args: Vec<String>,
    input: Option<serde_json::Value>,
}

fn main() {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "app-server") {
        args.remove(0);
    }
    let listen = args.last().is_some_and(|value| value == "--listen");
    if listen {
        args.pop();
    }
    assert_eq!(
        args.len(),
        3,
        "CLI, scenario and report paths, optional --listen"
    );
    let steps: Vec<Step> = serde_json::from_slice(&fs::read(&args[1]).unwrap()).unwrap();
    let mut results = Vec::new();
    for step in steps {
        let mut child = Command::new(&args[0])
            .args(step.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn fixture-owned CLI");
        if let Some(input) = step.input {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.to_string().as_bytes())
                .unwrap();
        }
        drop(child.stdin.take());
        let output = child.wait_with_output().expect("reap fixture-owned CLI");
        let badge = Command::new("tmux")
            .args(["-u", "show-options", "-p", "-qv", "-t"])
            .arg(std::env::var("TMUX_PANE").unwrap())
            .arg("@tmux-team.badge")
            .output()
            .expect("read fixture pane badge");
        results.push(json!({"code":output.status.code(),
            "badge": String::from_utf8(badge.stdout).unwrap().trim(),
            "stdout": String::from_utf8(output.stdout).unwrap(),
            "stderr": String::from_utf8(output.stderr).unwrap()}));
    }
    let stage = std::path::PathBuf::from(&args[2]).with_extension("pending");
    fs::write(&stage, serde_json::to_vec(&results).unwrap()).unwrap();
    fs::rename(stage, &args[2]).unwrap();
    if listen {
        let mut received = Vec::new();
        for line in std::io::stdin().lock().lines() {
            let line = line.unwrap();
            let done = line.starts_with("Submit your response with the command above.");
            received.push(line);
            if done {
                break;
            }
        }
        fs::write(
            std::path::PathBuf::from(&args[2]).with_extension("input.json"),
            serde_json::to_vec(&received).unwrap(),
        )
        .unwrap();
    }
}
