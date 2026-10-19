use super::*;
use crate::test_support::TestDirectory;
use std::{fs, os::unix::fs::PermissionsExt};
use tmt_core::{
    binding::session::{BindingSessionState, ProviderSessionId},
    endpoint::ServerEvidence,
    host::HostKind,
};

fn binding() -> Binding {
    Binding {
        id: "binding".into(),
        identity_id: "identity".into(),
        pane_id: "%1".into(),
        pane_pid: 20,
        pane_incarnation: Some("pane-start".into()),
        session: BindingSessionState::default(),
        server: ServerEvidence {
            host: HostKind::Tmux,
            server_id: "server".into(),
            socket_path: "/tmp/test.sock".into(),
            server_pid: 10,
            server_start_time: "server-start".into(),
        },
    }
}

fn key() -> Key {
    Key::new(
        &HarnessId::new("claude").unwrap(),
        &CallerSession {
            session: ProviderSessionId::new("thread").unwrap(),
            runtime_pid: Some(30),
        },
        &binding(),
    )
}

#[test]
fn private_hits_expire_without_refreshing_and_each_coordinate_change_misses() {
    let directory = TestDirectory::new();
    let cache = Refusals::in_directory(&directory.path);
    let key = key();
    cache.remember(key.clone(), "provider-not-root", 100);
    let original = fs::read(&cache.path).unwrap();
    assert_eq!(
        fs::metadata(&cache.path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        cache.lookup(&key, 101).as_deref(),
        Some("provider-not-root")
    );
    assert_eq!(
        fs::read(&cache.path).unwrap(),
        original,
        "a hit cannot extend the TTL"
    );
    assert_eq!(
        cache.lookup(&key, 100 + TTL_MS - 1).as_deref(),
        Some("provider-not-root")
    );
    assert_eq!(cache.lookup(&key, 100 + TTL_MS), None);
    assert_eq!(
        cache.lookup(&key, 99),
        None,
        "a backward clock cannot preserve a refusal"
    );
    for field in [
        "harness",
        "session",
        "runtime_pid",
        "binding",
        "pane",
        "pane_pid",
        "pane_incarnation",
        "host",
        "server",
        "socket",
        "server_pid",
        "server_incarnation",
    ] {
        let mut harness = HarnessId::new("claude").unwrap();
        let mut coordinates = CallerSession {
            session: ProviderSessionId::new("thread").unwrap(),
            runtime_pid: Some(30),
        };
        let mut binding = binding();
        match field {
            "harness" => harness = HarnessId::new("codex").unwrap(),
            "session" => coordinates.session = ProviderSessionId::new("different").unwrap(),
            "runtime_pid" => coordinates.runtime_pid = Some(31),
            "binding" => binding.id = "replacement".into(),
            "pane" => binding.pane_id = "%2".into(),
            "pane_pid" => binding.pane_pid = 21,
            "pane_incarnation" => binding.pane_incarnation = Some("replacement".into()),
            "host" => binding.server.host = HostKind::parse("other-host").unwrap(),
            "server" => binding.server.server_id = "other-server".into(),
            "socket" => binding.server.socket_path = "/tmp/other.sock".into(),
            "server_pid" => binding.server.server_pid = 11,
            "server_incarnation" => binding.server.server_start_time = "replacement".into(),
            _ => unreachable!(),
        }
        let changed = Key::new(&harness, &coordinates, &binding);
        assert_eq!(
            cache.lookup(&changed, 101),
            None,
            "changed {field} must re-run admission"
        );
    }
}

#[test]
fn bounded_entries_evict_oldest_and_prune_expired_on_next_refusal() {
    let directory = TestDirectory::new();
    let cache = Refusals::in_directory(&directory.path);
    let first = key();
    for index in 0..=ENTRIES {
        let mut next = first.clone();
        next.runtime_pid = Some(index as u32);
        cache.remember(next, "main-provider", 100 + index as u64);
    }
    assert_eq!(cache.read().unwrap().entries.len(), ENTRIES);
    let mut evicted = first.clone();
    evicted.runtime_pid = Some(0);
    assert_eq!(cache.lookup(&evicted, 200), None);
    cache.remember(first.clone(), "binding", 200 + TTL_MS);
    assert_eq!(cache.read().unwrap().entries.len(), 1);
    assert_eq!(
        cache.lookup(&first, 201 + TTL_MS).as_deref(),
        Some("binding")
    );
}

#[test]
fn missing_corrupt_changed_oversized_and_nonregular_cache_are_misses() {
    let directory = TestDirectory::new();
    let cache = Refusals::in_directory(&directory.path);
    let key = key();
    assert_eq!(cache.lookup(&key, 100), None);
    assert!(!cache.path.exists(), "a read never creates the cache");
    cache.remember(key.clone(), "binding", 100);
    let mut exact = fs::read(&cache.path).unwrap();
    exact.resize(BYTES as usize, b' ');
    fs::write(&cache.path, &exact).unwrap();
    assert_eq!(cache.lookup(&key, 100).as_deref(), Some("binding"));
    assert_eq!(fs::read(&cache.path).unwrap(), exact);
    exact.push(b' ');
    fs::write(&cache.path, exact).unwrap();
    assert_eq!(cache.lookup(&key, 100), None);
    // A version 1 document, from before external caller keys, is a miss
    // even for the key it held.
    let version_1 = br#"{"version":1,"entries":[{"key":{"harness":"claude","session":"thread","runtime_pid":30,"binding":"binding","pane":"%1","pane_pid":20,"pane_incarnation":"pane-start","host":"tmux","server":"server","socket":"/tmp/test.sock","server_pid":10,"server_incarnation":"server-start"},"refused_at_ms":100,"layer":"binding"}]}"#.to_vec();
    for bytes in [
        b"broken".to_vec(),
        version_1,
        br#"{"version":3,"entries":[]}"#.to_vec(),
        vec![b' '; BYTES as usize + 1],
    ] {
        fs::write(&cache.path, bytes).unwrap();
        assert_eq!(cache.lookup(&key, 100), None);
    }
    fs::remove_file(&cache.path).unwrap();
    let target = directory.path.join("target");
    fs::write(&target, b"preserve").unwrap();
    std::os::unix::fs::symlink(&target, &cache.path).unwrap();
    assert_eq!(cache.lookup(&key, 100), None);
    assert_eq!(fs::read(&target).unwrap(), b"preserve");
    fs::remove_file(&cache.path).unwrap();
    nix::unistd::mkfifo(
        &cache.path,
        nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR,
    )
    .unwrap();
    assert_eq!(cache.lookup(&key, 100), None, "FIFO lookup is nonblocking");
}

/// An approved external host declaring `FAKE_PANE_ID` and `FAKE_SOCKET`.
fn host(directory: &TestDirectory) -> DriverRecord {
    let path = directory.path.join("driver");
    fs::write(&path, b"driver").unwrap();
    DriverRecord {
        name: "fake".into(),
        path: path.clone(),
        digest: "digest".into(),
        fingerprint: crate::executable_trust::Fingerprint::of(&fs::metadata(&path).unwrap()),
        protocol: 1,
        locations: None,
        capabilities: serde_json::from_value(serde_json::json!({"protocols":[1],"kind":"host",
            "name":"fake","version":"0.0.0-test","ops":["caller"],"paneId":{"prefix":"fake-"},
            "target":"f{n}","callerEnv":["FAKE_SOCKET","FAKE_PANE_ID"]}))
        .unwrap(),
        approved_at_ms: 1,
        source: crate::driver_protocol::registry::DriverSource::Path,
    }
}

fn caller(variables: &[(&str, std::ffi::OsString)]) -> CallerEnvironment {
    CallerEnvironment {
        tmux: None,
        pane: None,
        process_id: 1,
        driver_env: variables
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect(),
    }
}

#[test]
fn an_external_caller_key_follows_its_variables_and_its_hosts_bindings() {
    use std::os::unix::ffi::OsStringExt;
    let directory = TestDirectory::new();
    let cache = Refusals::in_directory(&directory.path);
    let host = host(&directory);
    let harness = HarnessId::new("claude").unwrap();
    let coordinates = CallerSession {
        session: ProviderSessionId::new("thread").unwrap(),
        runtime_pid: Some(30),
    };
    let ids = ["b".to_owned(), "a".to_owned()];
    let set = caller(&[("FAKE_PANE_ID", "f1".into()), ("FAKE_SOCKET", "/s".into())]);
    let key = Key::caller(&harness, &coordinates, &host, &set, &ids);
    cache.remember(key.clone(), "native-admission", 100);
    assert_eq!(cache.lookup(&key, 101).as_deref(), Some("native-admission"));
    assert_eq!(cache.lookup(&key, 100 + TTL_MS), None);
    // No raw variable value is kept.
    let stored = fs::read_to_string(&cache.path).unwrap();
    assert!(!stored.contains("f1") && !stored.contains("/s"), "{stored}");
    // The binding IDs are a set: their order does not matter.
    let reordered = ["a".to_owned(), "b".to_owned()];
    assert_eq!(
        Key::caller(&harness, &coordinates, &host, &set, &reordered),
        key
    );
    // A binding key never matches a caller key.
    assert_ne!(Key::new(&harness, &coordinates, &binding()), key);
    let not_text = std::ffi::OsString::from_vec(vec![b'f', 0xff]);
    let lossy = std::ffi::OsString::from(not_text.to_string_lossy().into_owned());
    let changes: [(&str, Key); 8] = [
        (
            "a variable's value",
            Key::caller(
                &harness,
                &coordinates,
                &host,
                &caller(&[("FAKE_PANE_ID", "f2".into()), ("FAKE_SOCKET", "/s".into())]),
                &ids,
            ),
        ),
        (
            "an unset variable",
            Key::caller(
                &harness,
                &coordinates,
                &host,
                &caller(&[("FAKE_PANE_ID", "f1".into())]),
                &ids,
            ),
        ),
        (
            "an empty variable instead of an unset one",
            Key::caller(
                &harness,
                &coordinates,
                &host,
                &caller(&[("FAKE_PANE_ID", "f1".into()), ("FAKE_SOCKET", "".into())]),
                &ids,
            ),
        ),
        (
            "a bound pane more",
            Key::caller(
                &harness,
                &coordinates,
                &host,
                &set,
                &["a".into(), "b".into(), "c".into()],
            ),
        ),
        (
            "a rebound pane",
            Key::caller(
                &harness,
                &coordinates,
                &host,
                &set,
                &["a".into(), "d".into()],
            ),
        ),
        (
            "the session",
            Key::caller(
                &harness,
                &CallerSession {
                    session: ProviderSessionId::new("different").unwrap(),
                    ..coordinates.clone()
                },
                &host,
                &set,
                &ids,
            ),
        ),
        (
            "the provider pid",
            Key::caller(
                &harness,
                &CallerSession {
                    runtime_pid: Some(31),
                    ..coordinates.clone()
                },
                &host,
                &set,
                &ids,
            ),
        ),
        (
            "the harness",
            Key::caller(
                &HarnessId::new("codex").unwrap(),
                &coordinates,
                &host,
                &set,
                &ids,
            ),
        ),
    ];
    for (change, changed) in changes {
        assert_eq!(
            cache.lookup(&changed, 101),
            None,
            "{change} must re-run admission"
        );
    }
    // Raw bytes, not their lossy text, so two values cannot meet.
    assert_ne!(
        Key::caller(
            &harness,
            &coordinates,
            &host,
            &caller(&[("FAKE_PANE_ID", not_text)]),
            &ids
        ),
        Key::caller(
            &harness,
            &coordinates,
            &host,
            &caller(&[("FAKE_PANE_ID", lossy)]),
            &ids
        ),
    );
}

/// The digest has a fixed size, so a very large variable cannot push the
/// document over its limit and stop every later hint.
#[test]
fn a_large_caller_variable_still_fits_the_cache() {
    let directory = TestDirectory::new();
    let cache = Refusals::in_directory(&directory.path);
    let host = host(&directory);
    let large = caller(&[("FAKE_PANE_ID", "f".repeat(BYTES as usize).into())]);
    let key = Key::caller(
        &HarnessId::new("claude").unwrap(),
        &CallerSession {
            session: ProviderSessionId::new("thread").unwrap(),
            runtime_pid: Some(30),
        },
        &host,
        &large,
        &[],
    );
    cache.remember(key.clone(), "budget", 100);
    assert_eq!(cache.lookup(&key, 101).as_deref(), Some("budget"));
}

/// A version 2 document written before nested keys still reads: a binding
/// key keeps its JSON exactly, and a caller key still matches.
#[test]
fn version_2_binding_and_caller_hints_still_read_and_never_match_a_nested_key() {
    let directory = TestDirectory::new();
    let cache = Refusals::in_directory(&directory.path);
    let host = host(&directory);
    let harness = HarnessId::new("claude").unwrap();
    let coordinates = CallerSession {
        session: ProviderSessionId::new("thread").unwrap(),
        runtime_pid: Some(30),
    };
    let set = caller(&[("FAKE_PANE_ID", "f1".into())]);
    let ids = ["a".to_owned()];
    let external = Key::caller(&harness, &coordinates, &host, &set, &ids);
    let Target::Caller {
        coordinates: digest,
        ..
    } = &external.target
    else {
        unreachable!()
    };
    let version_2 = format!(
        r#"{{"version":2,"entries":[{{"key":{{"harness":"claude","session":"thread","runtime_pid":30,"target":{{"binding":{{"binding":"binding","pane":"%1","pane_pid":20,"pane_incarnation":"pane-start","host":"tmux","server":"server","socket":"/tmp/test.sock","server_pid":10,"server_incarnation":"server-start"}}}}}},"refused_at_ms":100,"layer":"binding"}},{{"key":{{"harness":"claude","session":"thread","runtime_pid":30,"target":{{"caller":{{"host":"fake","coordinates":"{digest}"}}}}}},"refused_at_ms":100,"layer":"native-admission"}}]}}"#
    );
    fs::write(&cache.path, &version_2).unwrap();
    assert_eq!(cache.lookup(&key(), 101).as_deref(), Some("binding"));
    assert_eq!(
        cache.lookup(&external, 101).as_deref(),
        Some("native-admission")
    );
    // A binding key's JSON is unchanged.
    let written = serde_json::to_string(&key()).unwrap();
    assert!(version_2.contains(&written), "{written}");
    // The same caller beside tmux variables is a nested key: neither hint
    // matches it, with or without the tmux binding.
    let nested_caller = CallerEnvironment {
        tmux: Some("/tmp/test.sock,10,0".into()),
        pane: Some("%1".into()),
        ..caller(&[("FAKE_PANE_ID", "f1".into())])
    };
    for tmux in [None, Some(&binding())] {
        let nested = Key::nested(&harness, &coordinates, &host, &nested_caller, tmux, &ids);
        assert_eq!(cache.lookup(&nested, 101), None);
        cache.remember(nested.clone(), "main-provider", 102);
        assert_eq!(cache.lookup(&nested, 103).as_deref(), Some("main-provider"));
    }
    // Without tmux variables the nested digest still differs from the
    // external one: it has its own tag.
    assert_ne!(
        Key::nested(&harness, &coordinates, &host, &set, None, &ids),
        external
    );
}

/// A nested key changes with each raw tmux variable, set or not, and with
/// every field of the tmux binding, as well as with the external side.
#[test]
fn a_nested_key_follows_both_tmux_variables_and_the_tmux_binding() {
    use std::os::unix::ffi::OsStringExt;
    let directory = TestDirectory::new();
    let host = host(&directory);
    let harness = HarnessId::new("claude").unwrap();
    let coordinates = CallerSession {
        session: ProviderSessionId::new("thread").unwrap(),
        runtime_pid: Some(30),
    };
    let ids = ["a".to_owned()];
    let with =
        |tmux: Option<std::ffi::OsString>, pane: Option<std::ffi::OsString>| CallerEnvironment {
            tmux,
            pane,
            ..caller(&[("FAKE_PANE_ID", "f1".into())])
        };
    let tmux = || Some(std::ffi::OsString::from("/tmp/test.sock,10,0"));
    let pane = || Some(std::ffi::OsString::from("%1"));
    let not_text = |bytes: &[u8]| Some(std::ffi::OsString::from_vec(bytes.to_vec()));
    let key = |environment: &CallerEnvironment, bound: Option<&Binding>, ids: &[String]| {
        Key::nested(&harness, &coordinates, &host, environment, bound, ids)
    };
    let base = key(&with(tmux(), pane()), Some(&binding()), &ids);
    assert_eq!(base, key(&with(tmux(), pane()), Some(&binding()), &ids));
    let mut changed = vec![
        ("no tmux binding", key(&with(tmux(), pane()), None, &ids)),
        (
            "TMUX unset",
            key(&with(None, pane()), Some(&binding()), &ids),
        ),
        (
            "TMUX_PANE unset",
            key(&with(tmux(), None), Some(&binding()), &ids),
        ),
        (
            "TMUX empty",
            key(&with(Some("".into()), pane()), Some(&binding()), &ids),
        ),
        (
            "TMUX_PANE empty",
            key(&with(tmux(), Some("".into())), Some(&binding()), &ids),
        ),
        (
            "TMUX value",
            key(
                &with(Some("/tmp/o.sock,1,0".into()), pane()),
                Some(&binding()),
                &ids,
            ),
        ),
        (
            "TMUX_PANE value",
            key(&with(tmux(), Some("%2".into())), Some(&binding()), &ids),
        ),
        (
            "TMUX not text",
            key(
                &with(not_text(b"/tmp/\xff"), pane()),
                Some(&binding()),
                &ids,
            ),
        ),
        (
            "TMUX_PANE not text",
            key(&with(tmux(), not_text(b"%\xff")), Some(&binding()), &ids),
        ),
        (
            "the external bindings",
            key(&with(tmux(), pane()), Some(&binding()), &["b".into()]),
        ),
        (
            "the external target",
            key(
                &CallerEnvironment {
                    tmux: tmux(),
                    pane: pane(),
                    ..caller(&[("FAKE_PANE_ID", "f2".into())])
                },
                Some(&binding()),
                &ids,
            ),
        ),
    ];
    // The two variables are not one value: moving bytes between them is a change.
    assert_ne!(
        key(&with(Some("ab".into()), Some("c".into())), None, &ids),
        key(&with(Some("a".into()), Some("bc".into())), None, &ids)
    );
    for field in [
        "binding",
        "pane",
        "pane_pid",
        "pane_incarnation",
        "server",
        "socket",
        "server_pid",
        "server_incarnation",
    ] {
        let mut bound = binding();
        match field {
            "binding" => bound.id = "replacement".into(),
            "pane" => bound.pane_id = "%2".into(),
            "pane_pid" => bound.pane_pid = 21,
            "pane_incarnation" => bound.pane_incarnation = Some("replacement".into()),
            "server" => bound.server.server_id = "other-server".into(),
            "socket" => bound.server.socket_path = "/tmp/other.sock".into(),
            "server_pid" => bound.server.server_pid = 11,
            "server_incarnation" => bound.server.server_start_time = "replacement".into(),
            _ => unreachable!(),
        }
        changed.push((field, key(&with(tmux(), pane()), Some(&bound), &ids)));
    }
    for (change, other) in changed {
        assert_ne!(other, base, "{change} must re-run admission");
    }
    // No raw tmux value is kept.
    let stored =
        serde_json::to_string(&key(&with(tmux(), Some("%77".into())), None, &ids)).unwrap();
    assert!(
        !stored.contains("test.sock") && !stored.contains("%77"),
        "{stored}"
    );
}
