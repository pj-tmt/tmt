//! The public launch and hook path through a private Node/native provider pair.
use super::attach::{quote, wait_for};
use super::*;
use std::os::unix::fs::symlink;

#[test]
fn an_npm_codex_launch_keeps_its_current_session_and_final() {
    let mut world = World::new();
    let node = tmt_adapters::skill_installation::ProviderEnvironment::capture()
        .unwrap()
        .find_command("node")
        .expect("Node fixture prerequisite");
    symlink(node, world.root.join("bin/node")).unwrap();
    let native = world.root.join("native/codex");
    fs::create_dir(native.parent().unwrap()).unwrap();
    let example = std::path::Path::new(env!("CARGO_BIN_EXE_tmt"))
        .parent()
        .unwrap()
        .join("examples/provider-script-fixture");
    tmt_test_support::write_executable(
        &native,
        &fs::read(example).expect("build provider-script-fixture first"),
        0o755,
    )
    .unwrap();
    let package = world.root.join("node_modules/@openai/codex");
    fs::create_dir_all(package.join("bin")).unwrap();
    fs::write(
        package.join("package.json"),
        br#"{"name":"@openai/codex","bin":{"codex":"bin/codex.js"}}"#,
    )
    .unwrap();
    let shim = format!(
        r#"#!/usr/bin/env node
const fs = require('node:fs');
const {{spawn}} = require('node:child_process');
fs.writeFileSync('{root}/beta.pid', String(process.pid));
const child = spawn('{native}', process.argv.slice(2), {{stdio:'inherit'}});
child.on('exit', (code, signal) => {{ if (signal) process.kill(process.pid, signal); else process.exit(code ?? 1); }});
"#,
        root = world.root.display(),
        native = native.display()
    );
    tmt_test_support::write_executable(&package.join("bin/codex.js"), shim.as_bytes(), 0o755)
        .unwrap();
    symlink(package.join("bin/codex.js"), world.root.join("bin/codex")).unwrap();
    let session = "11111111-1111-4111-8111-111111111111";
    let cli = quote(env!("CARGO_BIN_EXE_tmt"));
    let script = format!(
        r#"#!/bin/sh
printf '%s' "$PPID" > '{root}/alpha.pid'
printf '%s' "$$" > '{root}/alpha-peer.pid'
printf '%s' '{{"hook_event_name":"SessionStart","session_id":"{session}","source":"startup","model":"fixture-model"}}' | {cli} __hook codex > '{root}/start.out' 2> '{root}/start.err'
printf '%s' "$?" > '{root}/start.status'
IFS= read -r turn
printf '%s' '{{"hook_event_name":"UserPromptSubmit","session_id":"{session}","turn_id":"native-turn","prompt":"private question"}}' | {cli} __hook codex > '{root}/prompt.out' 2> '{root}/prompt.err'
printf '%s' '{{"hook_event_name":"Stop","session_id":"{session}","turn_id":"native-turn","last_assistant_message":"native answer retained","transcript_path":null,"stop_hook_active":false}}' | {cli} __hook codex > '{root}/stop.out' 2> '{root}/stop.err'
printf '%s' "$?" > '{root}/stop.status'
IFS= read -r quit
exit 7
"#,
        root = world.root.display()
    );
    let interpreter = world.root.join("interpreter");
    tmt_test_support::write_executable(&interpreter, script.as_bytes(), 0o755).unwrap();
    world.interpreter = Some(interpreter);
    let settings = world.root.join("home/.codex/hooks.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(settings, serde_json::json!({"hooks":{"Stop":[tmt_adapters::runtime::hook_protocol::command_entry("codex", env!("CARGO_BIN_EXE_tmt"))]}}).to_string()).unwrap();
    let output = world.run(&["run", "--no-channel", "--save", "alpha", "codex"]);
    assert!(output.status.success(), "{output:?}");
    wait_for("the startup hook", || {
        world.root.join("start.status").exists()
    });
    assert_eq!(
        fs::read_to_string(world.root.join("start.status")).unwrap(),
        "0"
    );
    let database = support::state_dir(&world.root).join("tmt.db");
    let state = || {
        let db = rusqlite::Connection::open_with_flags(
            &database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        db.query_row("SELECT b.observed_provider_session_id,b.runtime_pid,b.launch_owner_pid FROM bindings b JOIN identities i ON i.id=b.identity_id WHERE i.name='alpha'", [], |r| Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<i64>>(1)?,r.get::<_,Option<i64>>(2)?))).unwrap()
    };
    wait_for("the exact native session", || {
        state().0.as_deref() == Some(session)
    });
    let admitted = state();
    assert_eq!(
        admitted.1,
        Some(
            fs::read_to_string(world.root.join("alpha.pid"))
                .unwrap()
                .parse()
                .unwrap()
        )
    );
    assert_ne!(
        admitted.1,
        Some(
            fs::read_to_string(world.root.join("beta.pid"))
                .unwrap()
                .parse()
                .unwrap()
        )
    );
    assert!(admitted.2.is_some());
    let listed = world.json(&["ls", "--json"]);
    let row = listed["identities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "alpha")
        .unwrap();
    let pane = row["pane"].as_str().unwrap();
    let mut terminal =
        Connection::connect(&support::state_dir(&world.root).join("pty"), pane).unwrap();
    terminal.input(b"answer\r").unwrap();
    wait_for("the Stop hook", || world.root.join("stop.status").exists());
    assert_eq!(
        fs::read_to_string(world.root.join("stop.status")).unwrap(),
        "0"
    );
    wait_for("the retained final", || {
        let db = rusqlite::Connection::open_with_flags(
            &database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        db.query_row("SELECT count(*) FROM recent_responses WHERE session_id=?1 AND text='native answer retained'", [session], |r|r.get::<_,i64>(0)).unwrap() == 1
    });
    terminal.input(b"quit\r").unwrap();
    drop(terminal);
    world.remember();
    world.remember_programs();
    assert!(world.clean());
}
