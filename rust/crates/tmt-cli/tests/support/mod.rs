//! Isolation and direct-child lifetime shared by real CLI integration fixtures.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

pub fn state_dir(root: &Path) -> PathBuf {
    root.join("xdg-config/tmt")
}

pub fn command(root: &Path, args: &[&str]) -> Command {
    for directory in [
        "home",
        "xdg-config",
        "xdg-data",
        "xdg-state",
        "xdg-cache",
        "tmp",
        "tmux",
    ] {
        fs::create_dir_all(root.join(directory)).expect("create owned CLI environment directory");
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_tmt"));
    command
        .args(args)
        .env_clear()
        .env("TMT_TEST_ROOT", root)
        .env("TMT_AGENT", "1")
        .env("BROWSER", "/usr/bin/true")
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env("XDG_STATE_HOME", root.join("xdg-state"))
        .env("XDG_CACHE_HOME", root.join("xdg-cache"))
        .env("CODEX_HOME", root.join("home/.codex"))
        .env("TMPDIR", root.join("tmp"))
        .env("TMUX_TMPDIR", root.join("tmux"))
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("LANG", "en_US.UTF-8")
        .env("LC_ALL", "en_US.UTF-8")
        .current_dir(root);
    command
}

pub fn wait(child: &mut Option<Child>, timeout: Duration) -> Child {
    let deadline = Instant::now() + timeout;
    loop {
        if child.as_mut().unwrap().try_wait().unwrap().is_some() {
            return child.take().unwrap();
        }
        assert!(Instant::now() < deadline, "owned CLI did not exit");
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn stop(child: &mut Option<Child>) {
    if let Some(mut child) = child.take() {
        if !matches!(child.try_wait(), Ok(Some(_))) {
            let _ = child.kill();
        }
        child
            .wait()
            .expect("reap owned CLI before removing fixture");
    }
}

/// Preserve private launch evidence before fixture unwinding removes it. The
/// action's assertion/deadline is unchanged; capture runs only after its panic.
#[allow(dead_code)]
pub fn with_launch_failure_packet<T>(
    database: &Path,
    owned: &[u64],
    action: impl FnOnce() -> T,
) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(action)) {
        Ok(value) => value,
        Err(panic) => {
            let mut pids = owned.to_vec();
            let rows = (|| -> rusqlite::Result<serde_json::Value> {
                let db = rusqlite::Connection::open_with_flags(
                    database,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )?;
                db.busy_timeout(Duration::ZERO)?;
                let mut tables = serde_json::Map::new();
                for (name, sql) in [
                    (
                        "bindings",
                        "SELECT id,pane_id,runtime_state,runtime_pid,runtime_start_identity,observed_provider_session_id,launch_owner_pid,launch_owner_start_identity FROM bindings LIMIT 2",
                    ),
                    (
                        "preferences",
                        "SELECT identity_id,preferred_harness,remembered_harness,provider_session_id,stale_at_ms,resume_pending_at_ms FROM identity_session_preferences LIMIT 2",
                    ),
                    (
                        "launches",
                        "SELECT binding_id,owner_pid,owner_start,driver,status,provider_pid,provider_start,provider_session,step,reason FROM launch_observations ORDER BY created_at_ms DESC LIMIT 4",
                    ),
                ] {
                    let mut statement = db.prepare(sql)?;
                    let columns = statement
                        .column_names()
                        .into_iter()
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    let values = statement
                        .query_map([], |row| {
                            let mut value = serde_json::Map::new();
                            for (i, column) in columns.iter().enumerate() {
                                let cell = match row.get_ref(i)? {
                                    rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                                    rusqlite::types::ValueRef::Integer(n) => {
                                        if column.ends_with("_pid") && n > 0 {
                                            pids.push(n as u64);
                                        }
                                        serde_json::json!(n)
                                    }
                                    rusqlite::types::ValueRef::Text(text) => serde_json::json!(
                                        String::from_utf8_lossy(text)
                                            .chars()
                                            .take(64)
                                            .collect::<String>()
                                    ),
                                    _ => serde_json::json!("non-text value"),
                                };
                                value.insert(column.clone(), cell);
                            }
                            Ok(serde_json::Value::Object(value))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    tables.insert(name.into(), serde_json::json!(values));
                }
                Ok(serde_json::Value::Object(tables))
            })();
            pids.sort_unstable();
            pids.dedup();
            pids.truncate(6);
            let deadline = Instant::now() + Duration::from_millis(500);
            let processes = pids
                .into_iter()
                .map(|pid| {
                    let observed = tmt_adapters::process::runtime::observe_runtime_process(
                        &tmt_adapters::process::UnixCommandRunner,
                        pid,
                        deadline,
                    );
                    serde_json::json!({"pid":pid,"observed":format!("{observed:?}")})
                })
                .collect::<Vec<_>>();
            let rows = rows.unwrap_or_else(|error| serde_json::json!({"error":error.to_string()}));
            let mut startup_logs = Vec::new();
            if let Some(state) = database.parent()
                && let Ok(entries) = fs::read_dir(state.join("pty"))
            {
                for entry in entries.flatten().take(16) {
                    if startup_logs.len() == 2 {
                        break;
                    }
                    let path = entry.path();
                    if !path.to_string_lossy().ends_with(".startup.log") {
                        continue;
                    }
                    let mut bytes = Vec::new();
                    if let Ok(file) = fs::File::open(&path) {
                        let _ = file.take(2048).read_to_end(&mut bytes);
                    }
                    startup_logs.push(serde_json::json!({
                        "file":entry.file_name().to_string_lossy(),
                        "text":String::from_utf8_lossy(&bytes),
                    }));
                }
            }
            // Capture after the storage snapshot: a slow failed connection must
            // not give the launch more time before its failure state is read.
            let mut screens = Vec::new();
            if let Some(state) = database.parent()
                && let Ok(entries) = fs::read_dir(state.join("pty"))
            {
                for entry in entries.flatten().take(16) {
                    if screens.len() == 2 {
                        break;
                    }
                    let path = entry.path();
                    if path.extension().is_none_or(|extension| extension != "sock") {
                        continue;
                    }
                    let Some(pane) = path.file_stem().and_then(|stem| stem.to_str()) else {
                        continue;
                    };
                    use tmt_adapters::pty::{
                        Connection,
                        protocol::{Control, ControlReply},
                    };
                    let screen =
                        Connection::connect(path.parent().unwrap(), pane).and_then(|mut client| {
                            client.control(Control::Capture, Duration::from_millis(500))
                        });
                    let text = match screen {
                        Ok(ControlReply::Screen(text)) => text,
                        other => format!("{other:?}"),
                    };
                    screens.push(serde_json::json!({
                        "pane":pane,"screen":text.chars().take(2048).collect::<String>(),
                    }));
                }
            }
            let mut packet = serde_json::json!({
                "rows":rows,"processes":processes,"startup_logs":startup_logs,"screens":screens,
            })
            .to_string();
            if packet.len() > 8192 {
                while packet.len() > 8150 {
                    packet.pop();
                }
                packet.push_str(" [launch packet truncated]");
            }
            eprintln!("launch failure packet: {packet}");
            std::panic::resume_unwind(panic)
        }
    }
}
