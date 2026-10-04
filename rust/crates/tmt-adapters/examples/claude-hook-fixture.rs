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
    #[serde(default)]
    measure: bool,
    #[serde(default)]
    close_stdout: bool,
    checkpoint: Option<std::path::PathBuf>,
}

fn main() {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    if let Some(mock) = std::env::var_os("TMT_TEST_CLAUDE_MOCK") {
        let status = Command::new(std::env::var_os("TMT_TEST_CLAUDE_NODE").expect("fixture node"))
            .arg(mock)
            .args(&args)
            .status()
            .expect("run model-free MCP peer");
        std::process::exit(status.code().unwrap_or(1));
    }

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
        if let Some(checkpoint) = step.checkpoint {
            fs::write(&checkpoint, "ready").unwrap();
            let limit = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while fs::read_to_string(&checkpoint).unwrap() != "continue" {
                assert!(
                    std::time::Instant::now() < limit,
                    "checkpoint was not released"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let started = step.measure.then(std::time::Instant::now);
        let mut child = Command::new(&args[0])
            .args(step.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn fixture-owned CLI");
        if step.close_stdout {
            // Close the sole reader before the hook runs, exercising an actual
            // provider-output failure after any durable observation commits.
            drop(child.stdout.take());
        }
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
        let elapsed_ms = started.map(|start| start.elapsed().as_secs_f64() * 1000.0);
        let badge = Command::new("tmux")
            .args(["-u", "show-options", "-p", "-qv", "-t"])
            .arg(std::env::var("TMUX_PANE").unwrap())
            .arg("@tmux-team.badge")
            .output()
            .expect("read fixture pane badge");
        let mut result = json!({"code":output.status.code(),
            "badge": String::from_utf8(badge.stdout).unwrap().trim(),
            "stdout": String::from_utf8(output.stdout).unwrap(),
            "stderr": String::from_utf8(output.stderr).unwrap()});
        if let Some(elapsed_ms) = elapsed_ms {
            result["elapsedMs"] = json!(elapsed_ms);
        }
        results.push(result);
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
