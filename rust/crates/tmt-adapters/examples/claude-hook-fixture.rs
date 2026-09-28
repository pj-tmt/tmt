//! Deterministic Claude-shaped parent for isolated hook lifecycle tests.
//! No provider, credentials, transcript access or model calls are involved.

use serde::Deserialize;
use serde_json::json;
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

#[derive(Deserialize)]
struct Step {
    args: Vec<String>,
    input: Option<serde_json::Value>,
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    assert_eq!(args.len(), 3, "CLI, scenario and report paths");
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
        results.push(json!({"code":output.status.code(),
            "stdout": String::from_utf8(output.stdout).unwrap(),
            "stderr": String::from_utf8(output.stderr).unwrap()}));
    }
    let stage = std::path::PathBuf::from(&args[2]).with_extension("pending");
    fs::write(&stage, serde_json::to_vec(&results).unwrap()).unwrap();
    fs::rename(stage, &args[2]).unwrap();
}
