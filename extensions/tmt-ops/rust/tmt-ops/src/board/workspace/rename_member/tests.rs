use super::*;
use std::{fs, path::PathBuf};

const ID: &str = "22222222-2222-4222-8222-222222222222";
fn request() -> Request {
    Request {
        identity_id: ID.into(),
        expected_name: " Worker ".into(),
        name: "--identity=$(never)".into(),
    }
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "ops-member-rename-{}",
            crate::id::new_v4().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        crate::test_support::write_ready_executable(
            &root.join("core"),
            r#"#!/bin/sh
cd "$(dirname "$0")" || exit 1
printf '%s\000' "$@" >> calls
cat > input
cat response
exit "$(cat exit)"
"#,
        );
        Self(root)
    }
    fn respond(&self, value: Value, exit: u8) {
        fs::write(self.0.join("response"), value.to_string()).unwrap();
        fs::write(self.0.join("exit"), exit.to_string()).unwrap();
    }
    fn core(&self) -> Core {
        Core::at(self.0.join("core"))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn public_request_keeps_literal_uuid_expected_name_and_destination_without_originator() {
    let f = Fixture::new();
    let request = request();
    f.respond(
        json!({"outcome":"applied","identityId":ID,"name":request.name,"changed":true}),
        0,
    );
    assert_eq!(
        request.apply(&f.core()).unwrap(),
        Outcome::Applied {
            name: request.name.clone(),
            changed: true
        }
    );
    let input: Value = serde_json::from_slice(&fs::read(f.0.join("input")).unwrap()).unwrap();
    assert_eq!(
        input,
        json!({"version":1,"operation":"identity.rename","input":{"identityId":ID,"expectedName":" Worker ","name":"--identity=$(never)"}})
    );
    assert_eq!(fs::read(f.0.join("calls")).unwrap(), b"api\0");
}
#[test]
fn typed_conflict_not_found_and_noop_keep_distinct_meanings() {
    let request = request();
    assert_eq!(
        request
            .decode(&json!({"outcome":"conflict","identityId":ID,"name":"competing-name"}))
            .unwrap(),
        Outcome::Conflict {
            name: "competing-name".into()
        }
    );
    assert_eq!(
        request
            .decode(&json!({"outcome":"not-found","identityId":ID}))
            .unwrap(),
        Outcome::NotFound
    );
    assert_eq!(request.decode(&json!({"outcome":"applied","identityId":ID,"name":"Worker","changed":false,"extra":"additive"})).unwrap(),Outcome::Applied{name:"Worker".into(),changed:false});
}
#[test]
fn mismatched_identity_unknown_outcome_and_incomplete_receipt_are_unknown() {
    for receipt in [
        json!({"outcome":"applied","identityId":"33333333-3333-4333-8333-333333333333","name":"Worker","changed":true}),
        json!({"outcome":"other","identityId":ID}),
        json!({"outcome":"applied","identityId":ID,"name":"Worker"}),
        json!({"outcome":"conflict","identityId":ID,"name":null}),
        json!({"outcome":"applied","identityId":ID,"name":"","changed":true}),
    ] {
        assert_eq!(
            request().decode(&receipt).unwrap_err().code,
            "SQUAD_IDENTITY_RENAME_INVALID"
        );
    }
}
#[test]
fn core_refusals_and_invalid_output_never_retry_or_become_applied() {
    for code in [
        "INVALID_NAME",
        "NAME_ALREADY_ACTIVE",
        "STORAGE_NOT_WRITABLE",
        "API_UNAVAILABLE",
    ] {
        let f = Fixture::new();
        f.respond(json!({"error":{"code":code,"message":"Not performed"}}), 1);
        assert_eq!(request().apply(&f.core()).unwrap_err().code, code);
        assert_eq!(fs::read(f.0.join("calls")).unwrap(), b"api\0");
    }
    let f = Fixture::new();
    f.respond(json!({}), 0);
    assert_eq!(
        request().apply(&f.core()).unwrap_err().code,
        "SQUAD_IDENTITY_RENAME_INVALID"
    );
    assert_eq!(fs::read(f.0.join("calls")).unwrap(), b"api\0");
}
