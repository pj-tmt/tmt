//! Public startup outcomes in an owned PTY world, without credentials or tmux.
use super::*;
use tmt_adapters::storage::{Storage, StorageError};
use tmt_core::binding::{
    BindingRepository,
    session::{HarnessId, ProviderSessionId, RuntimeMode},
};

#[test]
fn an_early_provider_exit_reports_its_reason_and_status_for_run_and_resume() {
    let mut world = World::new();
    let provider = world.root.join("bin/codex");
    // No hook/session is emitted: this models permission/config rejection before
    // admission. Publish the executable atomically, through the shared owner.
    tmt_test_support::write_executable(
        &provider,
        b"#!/bin/sh\nprintf 'fixture-permission-refused\\n' >&2\nexit 23\n",
        0o755,
    )
    .unwrap();
    let launched = world.run(&["run", "--save", "early", provider.to_str().unwrap()]);
    assert_exit(&launched, 23, "fixture-permission-refused");
    let list = world.json(&["ls", "--all", "--json"]);
    let id = list["identities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "early")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    // Seed only this private identity's remembered provider coordinate. The
    // retry skips transcript discovery, so the public resume reaches the same
    // deliberately failing executable without a real provider account.
    let mut storage = Storage::open(support::state_dir(&world.root).join("tmt.db")).unwrap();
    storage
        .with_binding_transaction::<_, StorageError>(|records| {
            let mut prefs = records.session_preferences(id)?;
            prefs.remember(
                HarnessId::new("codex").unwrap(),
                RuntimeMode::new(tmt_adapters::drivers::codex::MODE_EMBEDDED).unwrap(),
                ProviderSessionId::new("11111111-1111-4111-8111-111111111111").unwrap(),
            );
            records.set_session_preferences(id, &prefs)?;
            Ok(())
        })
        .unwrap();
    storage.close().unwrap();
    let resumed = world.run(&["resume", "--retry", "early"]);
    assert_exit(&resumed, 23, "fixture-permission-refused");
    assert!(world.clean());
}

fn assert_exit(output: &Output, status: i32, reason: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(status), "{output:?}");
    assert!(stderr.contains(reason), "{stderr}");
    assert!(stderr.contains("ended during startup"), "{stderr}");
    assert!(!stderr.contains("started early on host"), "{stderr}");
    assert!(!stderr.contains("not confirmed"), "{stderr}");
}

#[test]
fn successful_early_completion_preserves_zero_status_and_reports_completion() {
    let mut world = World::new();
    let output = world.run(&[
        "run",
        "--save",
        "done",
        "/bin/sh",
        "-c",
        "printf 'finished-ok\\n' >&2; exit 0",
    ]);
    assert_exit(&output, 0, "finished-ok");
    assert!(world.clean());
}

#[test]
fn a_signal_exit_during_startup_keeps_the_shell_status_and_original_reason() {
    let mut world = World::new();
    let output = world.run(&[
        "run",
        "--save",
        "signalled",
        "/bin/sh",
        "-c",
        "printf 'startup-signal-reason\\n' >&2; kill -TERM $$",
    ]);
    assert_exit(&output, 143, "startup-signal-reason");
    assert!(world.clean());
}
