//! Read-only status through an isolated public Core process, with frozen DTO evidence.

use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

const ID: &str = "10000000-0000-4000-8000-000000000001";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "tmt-digest-status-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        let script = r#"#!/bin/sh
cd '__ROOT__' || exit 9
printf '%s\n' "$*" >> calls
case "$*" in
 'config show --json') cat config.json ;;
 'identity ls --json') cat inventory.json ;;
 'api')
   n=$(grep -c '^api$' calls)
   cat > "request.$n.json"
   if test -f stats-fail; then
     printf '%s\n' '{"error":{"code":"STORAGE_NOT_WRITABLE","message":"unavailable"}}'
     exit 1
   fi
   cat "stats.$n.json" ;;
 *) exit 9 ;;
esac
"#
        .replace("__ROOT__", fixture.0.to_str().unwrap());
        fs::write(fixture.0.join("core"), script).unwrap();
        fs::set_permissions(fixture.0.join("core"), fs::Permissions::from_mode(0o700)).unwrap();
        fixture.write(
            "config.json",
            json!({"paths":{"global":fixture.0.join("config.toml")}}),
        );
        fixture.write(
            "inventory.json",
            json!({"identities":[{"id":ID,"name":"worker"}]}),
        );
        fixture.write(
            "stats.1.json",
            json!({"stats":[{
                "identityId":ID,"heldCount":3,"observedAtMs":1000,"oldestHeldAgeMs":500
            }]}),
        );
        fs::write(fixture.0.join("digest.toml"), "default = '5m'\n").unwrap();
        fixture
    }
    fn write(&self, file: &str, value: Value) {
        fs::write(self.0.join(file), serde_json::to_vec(&value).unwrap()).unwrap();
    }
    fn run(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_tmt-digest"))
            .args(["status", "--json"])
            .current_dir(&self.0)
            .env("HOME", &self.0)
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("XDG_DATA_HOME", self.0.join("data"))
            .env("TMT_HOME", self.0.join("state"))
            .env("TMT_EXECUTABLE", self.0.join("core"))
            .env("TMT_AGENT", "1")
            .env("BROWSER", "/usr/bin/true")
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    }
    fn status(&self) -> Value {
        let before = fs::read(self.0.join("digest.toml")).ok();
        let output = self.run();
        assert!(output.status.success(), "{:?}", output.stderr);
        assert!(output.stderr.is_empty());
        let document = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(fs::read(self.0.join("digest.toml")).ok(), before);
        for name in [
            "state",
            "data",
            "config",
            "digest.toml.lock",
            "history.json",
        ] {
            assert!(!self.0.join(name).exists(), "status created {name}");
        }
        document
    }
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.0.join("calls"))
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn status_is_one_json_document_and_only_public_reads() {
    let fixture = Fixture::new();
    let document = fixture.status();
    assert_eq!(document["version"], 1);
    assert_eq!(document["members"][0]["identityId"], ID);
    assert_eq!(document["members"][0]["heldCount"], 3);
    assert_eq!(document["members"][0]["nextSendAtMs"], 300500);
    assert_eq!(document["members"][0]["savings"], Value::Null);
    assert_eq!(
        fixture.calls(),
        ["config show --json", "identity ls --json", "api"]
    );
    let request: Value =
        serde_json::from_slice(&fs::read(fixture.0.join("request.1.json")).unwrap()).unwrap();
    assert_eq!(
        request,
        json!({"version":1,"operation":"digest.stats.show","input":{"identities":[ID]}})
    );
}

#[test]
fn inventory_is_read_once_and_stats_are_batched_sorted_and_bounded() {
    let fixture = Fixture::new();
    let ids: Vec<_> = (0..513)
        .map(|n| format!("10000000-0000-4000-8000-{n:012}"))
        .collect();
    fixture.write("inventory.json", json!({"identities":ids.iter().rev().map(|id| json!({"id":id,"name":"same display name"})).collect::<Vec<_>>()}));
    for (index, batch) in ids.chunks(256).enumerate() {
        fixture.write(&format!("stats.{}.json", index+1), json!({"stats":batch.iter().map(|id| json!({"identityId":id,"heldCount":0,"observedAtMs":1000,"oldestHeldAgeMs":null})).collect::<Vec<_>>()}));
    }
    let document = fixture.status();
    assert_eq!(document["members"].as_array().unwrap().len(), 513);
    assert_eq!(
        fixture.calls(),
        [
            "config show --json",
            "identity ls --json",
            "api",
            "api",
            "api"
        ]
    );
    for (index, batch) in ids.chunks(256).enumerate() {
        let request: Value = serde_json::from_slice(
            &fs::read(fixture.0.join(format!("request.{}.json", index + 1))).unwrap(),
        )
        .unwrap();
        assert_eq!(request["input"]["identities"], json!(batch));
    }
    assert_eq!(document["members"][512]["identityId"], ids[512]);
}

#[test]
fn empty_inventory_needs_no_stats_and_missing_settings_inherit_without_creation() {
    let fixture = Fixture::new();
    fixture.write("inventory.json", json!({"identities":[]}));
    fs::remove_file(fixture.0.join("digest.toml")).unwrap();
    let document = fixture.status();
    assert_eq!(document["members"], json!([]));
    assert_eq!(
        fixture.calls(),
        ["config show --json", "identity ls --json"]
    );
}

#[test]
fn stats_failure_or_malformed_evidence_preserves_rows_as_unknown() {
    for stats in [
        json!({}),
        json!({"stats":[]}),
        json!({"stats":[{"identityId":ID,"heldCount":-1}]}),
        json!({"stats":[{"identityId":"wrong","heldCount":0,"observedAtMs":1000}]}),
    ] {
        let fixture = Fixture::new();
        fixture.write("stats.1.json", stats);
        let document = fixture.status();
        assert_eq!(document["members"][0]["heldCount"], Value::Null);
        assert_eq!(document["members"][0]["labels"][1]["text"], "Unknown");
    }
    let fixture = Fixture::new();
    fs::write(fixture.0.join("stats-fail"), "").unwrap();
    assert_eq!(
        fixture.status()["members"][0]["statsObservedAtMs"],
        Value::Null
    );
}

#[test]
fn discovery_failures_are_json_errors_and_never_empty_success() {
    for inventory in [
        json!({}),
        json!({"identities":[{"id":"not-uuid","name":"worker"}]}),
        json!({"identities":[{"id":ID,"name":"a"},{"id":ID,"name":"b"}]}),
    ] {
        let fixture = Fixture::new();
        fixture.write("inventory.json", inventory);
        let output = fixture.run();
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stderr.is_empty());
        let document: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(document["error"]["code"], "CORE_RESPONSE_INVALID");
        assert!(document.get("members").is_none());
        assert!(!fixture.calls().iter().any(|call| call == "api"));
    }
    let fixture = Fixture::new();
    fs::write(fixture.0.join("digest.toml"), "flushCount = 0\n").unwrap();
    let before = fs::read(fixture.0.join("digest.toml")).unwrap();
    let output = fixture.run();
    assert_eq!(output.status.code(), Some(1));
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["error"]["code"], "DIGEST_SETTINGS_INVALID");
    assert_eq!(fs::read(fixture.0.join("digest.toml")).unwrap(), before);
    assert_eq!(fixture.calls(), ["config show --json"]);
}
