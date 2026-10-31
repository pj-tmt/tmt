//! Two private native CLI views over ordinary-user Auth/Firestore, never a real project.
//! Schedule ages advance deterministically; no heartbeat sleep or background polling loop.
use super::*;
use crate::squads::publisher::{Prepared, Schedule, SummarySampling};

fn cli(core: &core_fixture::Core, source: &Service, args: &[&str], input: &[u8]) -> Value {
    use std::ffi::OsString;
    let binary = std::path::PathBuf::from(
        std::env::var_os("TMT_REMOTE_QUALIFIER_BIN").expect("explicit qualifier binary"),
    );
    assert!(binary.is_absolute() && binary.is_file());
    let root = source.layout.directory.parent().unwrap();
    assert!(root.starts_with(&core.root));
    std::fs::write(
        core.root.join("storage-root"),
        root.as_os_str().as_encoded_bytes(),
    )
    .unwrap();
    let mut environment = Vec::<(OsString, OsString)>::new();
    for name in [
        "HOME",
        "TMT_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
    ] {
        environment.push((name.into(), root.as_os_str().to_owned()));
    }
    environment.extend([
        (
            "TMT_EXECUTABLE".into(),
            core.root.join("tmt").into_os_string(),
        ),
        ("TMT_AGENT".into(), "1".into()),
        ("BROWSER".into(), "/usr/bin/true".into()),
    ]);
    let args: Vec<_> = args.iter().map(OsString::from).collect();
    let output = tmt_invoke::invoke(
        tmt_invoke::Request {
            program: &binary,
            args: &args,
            input,
            deadline: Instant::now() + Duration::from_secs(45),
            max_stream_bytes: crate::limits::SQUAD_DIRECTORY_CONTROL_BYTES,
            launch: tmt_invoke::LaunchOptions {
                environment: tmt_invoke::EnvironmentPolicy::InheritWith(&environment),
                ..Default::default()
            },
        },
        None,
    )
    .expect("bounded native qualifier with owned child cleanup");
    assert!(
        output.status.success(),
        "native qualifier command failed (output deliberately omitted)"
    );
    assert!(output.stderr.is_empty());
    serde_json::from_slice(&output.stdout).unwrap()
}
fn board(state: &str) -> Value {
    json!({"squads":[{"squad":{"name":"Remote","lead":null},"sections":[{"rows":[{"id":TARGET,"name":"worker","state":state,"presence":"offline","task":"PRIVATE-TASK","cwd":"PRIVATE-PATH"}]}]}]})
}
fn advance_publication(source: &Service, peer: &str, heartbeat: bool) {
    // Force the existing publisher's due-age boundary, not a different publisher.
    // Sampling cadence/heartbeat constants are asserted in publisher's own unit cases.
    *source.summary_sample.lock().unwrap() = SummarySampling::default();
    let published = if heartbeat {
        let value = source
            .store()
            .unwrap()
            .summary_last_published(peer)
            .unwrap()
            .unwrap();
        Some((
            Prepared::published_fingerprint(&value).unwrap(),
            crate::limits::SQUAD_SUMMARY_HEARTBEAT_MS,
        ))
    } else {
        None
    };
    source.summary_schedules.lock().unwrap().insert(
        peer.into(),
        Schedule::restore(
            published,
            if heartbeat {
                None
            } else {
                Some(crate::limits::SQUAD_SUMMARY_COALESCE_MS)
            },
            0,
            0,
        ),
    );
}
#[test]
#[ignore = "Infra #5767 Auth+Firestore two-native-CLI qualifier; no provider fallback"]
fn native_cli_squad_change_heartbeat_restart_revoke_and_budget() {
    let emulator = Arc::new(Emulator::new()); // Validates BOTH numeric loopback hosts + demo project first.
    let core = core_fixture::Core::new();
    std::fs::write(core.root.join("multi"), "").unwrap();
    let script = std::fs::read_to_string(core.root.join("core.py")).unwrap();
    std::fs::write(core.root.join("core.py"),script.replace("wire=json.load(sys.stdin)","if args==['ops','squad','ls','--json']:\n    print((root/'squads.json').read_text());sys.exit(0)\nwire=json.load(sys.stdin)")).unwrap();
    std::fs::write(core.root.join("squads.json"), board("working").to_string()).unwrap();
    let (owner, owner_lease) = service(&core.root.join("owner"), &core, emulator.ports());
    let (sender, sender_lease) =
        service_with_summary_source(&core.root.join("sender"), &core, emulator.ports());
    emulator.seed_owner(&owner);
    let events = pair_with_reciprocal_choice(&owner, &sender, false);
    let remote = events.iter().find(|v| v["event"] == "connected").unwrap()["remote"]
        .as_str()
        .unwrap()
        .to_owned();
    emulator.route(&owner);
    emulator.route(&sender);
    let owner_control = message_control(&owner, &owner_lease);
    let sender_control = message_control(&sender, &sender_lease);
    let before = emulator.calls.lock().unwrap().clone();
    let unavailable = cli(&core, &owner, &["squads", "--json"], b"");
    assert_eq!(unavailable["machines"][0]["status"], "unavailable");
    assert_eq!(
        *emulator.calls.lock().unwrap(),
        before,
        "public list is cache only"
    );
    sender.publish_summaries().unwrap(); // Signed capability observation only.
    advance_publication(&sender, &owner.machine, false);
    sender.publish_summaries().unwrap();
    let first = cli(&core, &owner, &["squads", "--json"], b"");
    assert_eq!(first["machines"].as_array().unwrap().len(), 1);
    assert_eq!(
        first["machines"][0]["summary"]["squads"][0]["members"][0]["state"],
        "working"
    );
    assert!(!first.to_string().contains("PRIVATE"));
    assert_eq!(core.sends(), 0, "summary does not dispatch agent work");
    // Coalesce a real changed public Ops document, then exercise unchanged heartbeat.
    std::fs::write(core.root.join("squads.json"), board("testing").to_string()).unwrap();
    advance_publication(&sender, &owner.machine, false);
    sender.publish_summaries().unwrap();
    let changed = cli(&core, &owner, &["squads", "--json"], b"");
    assert_eq!(
        changed["machines"][0]["summary"]["squads"][0]["members"][0]["state"],
        "testing"
    );
    let sequence = owner
        .store()
        .unwrap()
        .connection
        .query_row(
            "SELECT sequence FROM squad_cache WHERE client_id=?1",
            [&sender.machine],
            |r| r.get::<_, i64>(0),
        )
        .unwrap();
    advance_publication(&sender, &owner.machine, true);
    sender.publish_summaries().unwrap();
    assert!(
        owner
            .store()
            .unwrap()
            .connection
            .query_row(
                "SELECT sequence FROM squad_cache WHERE client_id=?1",
                [&sender.machine],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
            > sequence
    );
    let before = emulator.calls.lock().unwrap().clone();
    cli(&core, &sender, &["squads", "--json"], b"");
    assert_eq!(
        *emulator.calls.lock().unwrap(),
        before,
        "second native CLI list is also cache only"
    );
    // Public send and original-ID reads reuse the existing dispatch/recovery owners.
    owner.store().unwrap().talk(&sender.machine, true).unwrap();
    let raw = prepared(&remote);
    let operation = crate::cloud_command::Request::read(&raw)
        .unwrap()
        .request_id;
    // The original dispatch commits but its Core acknowledgement is lost.
    std::fs::write(core.root.join("lost"), "").unwrap();
    let sent = cli(&core, &sender, &["message", "--json"], &raw);
    assert!(matches!(
        sent["state"].as_str(),
        Some("sent" | "unconfirmed")
    ));
    std::fs::remove_file(core.root.join("lost")).unwrap();
    owner.reconcile().unwrap();
    assert_eq!(core.sends(), 1);
    // An adopted Core request is not delivery evidence. Without messageState the
    // fixture correctly projects unknown (covered by the non-emulator regression).
    // Supply authoritative Core evidence, rather than accepting a wider state set.
    core.message_state(&json!({"state":"sent","reason":{"code":"fixture","text":"PRIVATE-REASON"},"latestMs":null,"remainingMs":null,"canSendNow":false}));
    let delivery = cli(
        &core,
        &sender,
        &["delivery", &operation, "--remote", &remote, "--json"],
        b"",
    );
    assert_eq!(delivery["operationId"], operation);
    assert_eq!(delivery["state"], "queued");
    assert!(!delivery.to_string().contains("PRIVATE"));
    assert_eq!(core.sends(), 1, "observation cannot resend");
    let receipt: Value = serde_json::from_slice(
        &std::fs::read(core.root.join(format!("receipt-{operation}"))).unwrap(),
    )
    .unwrap();
    let request = receipt["items"][0]["requestId"].as_str().unwrap();
    std::fs::write(
        core.root.join(format!("final-{request}")),
        json!({"status":"retained","response":"PRIVATE-FINAL"}).to_string(),
    )
    .unwrap();
    core.message_state(&json!({"state":"answered","reason":{"code":"answered","text":"PRIVATE-REASON"},"latestMs":null,"remainingMs":null,"canSendNow":false}));
    let answered = cli(
        &core,
        &sender,
        &["delivery", &operation, "--remote", &remote, "--json"],
        b"",
    );
    assert_eq!(answered["operationId"], operation);
    assert_eq!(answered["state"], "answered");
    assert!(!answered.to_string().contains("PRIVATE"));
    assert_eq!(
        core.sends(),
        1,
        "final observation keeps the same original without dispatch replay"
    );
    // Suspend the receiving process's notification, leaving real encrypted inbox work queued.
    emulator.receivers.lock().unwrap().remove(&owner.machine);
    std::fs::write(core.root.join("squads.json"), board("blocked").to_string()).unwrap();
    advance_publication(&sender, &owner.machine, false);
    sender.publish_summaries().unwrap();
    assert_eq!(
        cli(&core, &owner, &["squads", "--json"], b"")["machines"][0]["summary"]["squads"][0]["members"]
            [0]["state"],
        "testing",
        "offline cache retains the last verified content"
    );
    let retained = cli(&core, &owner, &["squads", "--json"], b"");
    owner_control.stop();
    sender_control.stop();
    drop(owner);
    drop(sender);
    drop(owner_lease);
    drop(sender_lease);
    // Reopen both private stores without any provider observation: cache bytes survive.
    let (owner, owner_lease) = service(&core.root.join("owner"), &core, emulator.ports());
    let (sender, sender_lease) =
        service_with_summary_source(&core.root.join("sender"), &core, emulator.ports());
    emulator.route(&owner);
    emulator.route(&sender);
    let owner_control = message_control(&owner, &owner_lease);
    let sender_control = message_control(&sender, &sender_lease);
    let before = emulator.calls.lock().unwrap().clone();
    assert_eq!(cli(&core, &owner, &["squads", "--json"], b""), retained);
    assert_eq!(*emulator.calls.lock().unwrap(), before);
    owner.reconcile().unwrap();
    assert_eq!(
        cli(&core, &owner, &["squads", "--json"], b"")["machines"][0]["summary"]["squads"][0]["members"]
            [0]["state"],
        "blocked",
        "reconnect drains the retained encrypted original"
    );
    assert_eq!(core.sends(), 1);
    // A revoked local association immediately removes the directory row.
    owner.store().unwrap().revoke(&sender.machine).unwrap();
    assert!(
        cli(&core, &owner, &["squads", "--json"], b"")["machines"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    owner.flush_routes().unwrap();
    assert!(sender.delivery(&remote, &operation).is_err());
    assert_eq!(core.sends(), 1);
    owner_control.stop();
    sender_control.stop();
    assert!(!owner.layout.directory.join(crate::control::SOCKET).exists());
    assert!(
        !sender
            .layout
            .directory
            .join(crate::control::SOCKET)
            .exists()
    );
    let desired = sender
        .store()
        .unwrap()
        .connection
        .query_row(
            "SELECT desired FROM squad_publications WHERE client_id=?1",
            [&owner.machine],
            |r| r.get::<_, Option<Vec<u8>>>(0),
        )
        .unwrap();
    assert!(desired.is_some());
    let budget = sender
        .layout
        .directory
        .join("native-budget")
        .join(format!("{PROJECT}.json"));
    drop(sender);
    drop(sender_lease);
    // Exhaust only this stopped private project's persisted guard, never an ambient account.
    let day = crate::firestore_budget::pacific_day(crate::pairing::now_ms().unwrap())
        .0
        .0;
    std::fs::write(
        &budget,
        json!({"version":1,"day":day,"reads":50_000}).to_string(),
    )
    .unwrap();
    let (sender, _sender_lease) =
        service_with_summary_source(&core.root.join("sender"), &core, emulator.ports());
    let before = emulator.calls.lock().unwrap().clone();
    assert!(sender.publish_summaries().is_err());
    let _ = sender.publish_summaries(); // A second scheduler observation cannot form a provider hot loop.
    assert_eq!(
        *emulator.calls.lock().unwrap(),
        before,
        "guard refuses before HTTP"
    );
    assert_eq!(
        sender
            .store()
            .unwrap()
            .connection
            .query_row(
                "SELECT desired FROM squad_publications WHERE client_id=?1",
                [&owner.machine],
                |r| r.get::<_, Option<Vec<u8>>>(0)
            )
            .unwrap(),
        desired,
        "budget refusal keeps pending content"
    );
    assert_eq!(core.sends(), 1);
}
