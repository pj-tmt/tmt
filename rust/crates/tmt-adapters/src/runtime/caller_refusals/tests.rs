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
    for bytes in [
        b"broken".to_vec(),
        br#"{"version":2,"entries":[]}"#.to_vec(),
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
