use super::*;
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, path::Path};

fn script(directory: &Path, body: &str) -> ProcessCore {
    let path = directory.join("tmt");
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    ProcessCore(CoreClient::with_executable(path))
}

fn directory() -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "tmt-office-core-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn a_successful_call_returns_the_json_document_and_sends_the_anonymous_envelope() {
    let directory = directory();
    let seen = directory.join("stdin");
    let core = script(
        &directory,
        &format!("cat > '{}'\necho '{{\"ok\":true}}'", seen.display()),
    );
    let value = core
        .api(
            "rooms.retire",
            json!({"roomId": "x"}),
            Some(Originator::Anonymous),
        )
        .unwrap();
    assert_eq!(value, json!({"ok": true}));
    let sent: Value = serde_json::from_str(&fs::read_to_string(seen).unwrap()).unwrap();
    assert_eq!(
        sent,
        json!({"version": 1, "operation": "rooms.retire",
               "input": {"roomId": "x"}, "originator": "anonymous"})
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn commands_run_with_the_json_flag() {
    let directory = directory();
    let core = script(&directory, "echo \"{\\\"args\\\":\\\"$*\\\"}\"");
    assert_eq!(
        core.command(&["room", "list"]).unwrap(),
        json!({"args": "--json room list"})
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn core_error_codes_pass_through_and_anything_else_is_unavailable() {
    let directory = directory();
    let coded = script(
        &directory,
        "echo '{\"error\":{\"code\":\"ROOM_NOT_FOUND\",\"message\":\"m\"}}'\nexit 3",
    );
    assert_eq!(
        coded.api("rooms.retire", json!({}), None).unwrap_err().code,
        "ROOM_NOT_FOUND"
    );
    let garbage = script(&directory, "echo not json\nexit 1");
    assert_eq!(
        garbage.api("rooms.retire", json!({}), None).unwrap_err(),
        CoreFault::unavailable()
    );
    let silent = script(&directory, "exit 0");
    assert_eq!(
        silent.command(&["list"]).unwrap_err(),
        CoreFault::unavailable()
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn the_service_never_opens_core_storage_outside_tests() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let name = path.file_name().unwrap().to_string_lossy();
            let test_only = name == "tests.rs"
                || name == "test_fixture.rs"
                || name == "in_process.rs"
                || relative.contains("/tests/")
                || relative.contains("tests/");
            if !relative.ends_with(".rs") || test_only {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            // Test modules inside production files start at `#[cfg(test)]`.
            let production = text.split("#[cfg(test)]\nmod tests {").next().unwrap();
            // `retirement_consumer.rs` only checks that core's file exists, never
            // its contents, so `office sync` does not make core create it.
            let allowed = relative == "retirement_consumer.rs";
            for needle in ["Storage::open", "tmt_adapters::storage", "paths.database"] {
                if production.contains(needle) && !(allowed && needle == "paths.database") {
                    offenders.push(format!("{relative}: {needle}"));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "core storage reached in process: {offenders:?}"
    );
}
