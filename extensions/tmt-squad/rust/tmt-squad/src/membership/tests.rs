use super::*;
use std::{fs, path::PathBuf};

struct Fixture {
    root: PathBuf,
    core: Core,
    squad: Squad,
}

impl Fixture {
    fn new(name: &str, members: Value) -> Self {
        let root =
            std::env::temp_dir().join(format!("squad-membership-{name}-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        for member in members.as_array().unwrap() {
            fs::write(
                root.join(format!("{}-metadata", member["id"].as_str().unwrap())),
                json!({"metadata": member["metadata"]}).to_string(),
            )
            .unwrap();
        }
        fs::write(root.join("roster"), json!({"members": members}).to_string()).unwrap();
        let executable = root.join("tmt");
        crate::test_support::write_ready_executable(
            &executable,
            r#"#!/bin/sh
root=${0%/*}
printf '%s\n' "$*" >> "$root/calls"
case "$1:$2:$3" in
  api:*) cat > /dev/null; cat "$root/roster" ;;
  ls:*) printf '%s\n' '{"identities":[]}' ;;
  identity:show:*) printf '{"identity":{"id":"%s","name":"%s","lifetime":"saved"}}\n' "$3" "$3" ;;
  identity:meta:list) cat "$root/$5-metadata" ;;
  identity:meta:set)
    if [ -e "$root/fail-$7" ]; then
      printf '%s\n' '{"error":{"code":"IDENTITY_METADATA_INVALID","message":"Injected metadata failure."}}'
      exit 1
    fi
    printf '%s' "$5" > "$root/$7-$4"
    printf '%s\n' '{}' ;;
  identity:meta:rm)
    rm -f "$root/$6-$4"
    printf '%s\n' '{}' ;;
  room:join:*|room:leave:*) printf '%s\n' '{}' ;;
  *) exit 2 ;;
esac
"#,
        );
        Self {
            core: Core::at(executable),
            root,
            squad: Squad {
                name: "p".into(),
                room_id: "room".into(),
            },
        }
    }

    fn value(&self, id: &str, suffix: &str) -> String {
        fs::read_to_string(self.root.join(format!("{id}-squad.p.{suffix}"))).unwrap()
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.root.join("calls")).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn member(id: &str, fields: Value) -> Value {
    json!({"id": id, "name": id, "lifetime": "saved", "metadata": fields})
}

#[test]
fn legacy_lead_is_materialized_before_role_is_changed_or_cleared() {
    for pair in ["role=reviews every merge", "role="] {
        let fixture = Fixture::new(
            if pair.ends_with('=') { "clear" } else { "set" },
            json!([member("sol", json!({"squad.p.role": "lead"})),]),
        );
        let outcome = set(&fixture.core, &fixture.squad, "sol", &[pair.into()]).unwrap();
        assert!(outcome.complete);
        assert_eq!(fixture.value("sol", "lead.marker"), "true");
        let calls = fixture.calls();
        if pair.ends_with('=') {
            assert!(!fixture.root.join("sol-squad.p.role").exists());
            assert!(
                calls.find("squad.p.lead.marker true").unwrap()
                    < calls.find("meta rm squad.p.role").unwrap()
            );
        } else {
            assert_eq!(fixture.value("sol", "role"), "reviews every merge");
            assert!(
                calls.find("squad.p.lead.marker true").unwrap()
                    < calls.find("squad.p.role reviews").unwrap()
            );
        }
    }
}

#[test]
fn ordinary_fields_and_unchanged_roles_do_not_allocate_markers() {
    for (name, role, pair) in [
        ("ordinary-field", "reviewer", "task=reviewing"),
        ("ordinary-role", "reviewer", "role=testing"),
        ("same-lead", "lead", "role=lead"),
    ] {
        let fixture = Fixture::new(name, json!([member("sol", json!({"squad.p.role": role}))]));
        assert!(
            set(&fixture.core, &fixture.squad, "sol", &[pair.into()])
                .unwrap()
                .complete
        );
        assert!(!fixture.root.join("sol-squad.p.lead.marker").exists());
    }
}

#[test]
fn setting_an_unconverted_members_role_to_lead_freezes_non_lead_status() {
    let fixture = Fixture::new(
        "ordinary-to-lead",
        json!([member("sol", json!({"squad.p.role": "reviewer"}))]),
    );
    assert!(
        set(&fixture.core, &fixture.squad, "sol", &["role=lead".into()])
            .unwrap()
            .complete
    );
    assert_eq!(fixture.value("sol", "lead.marker"), "false");
    assert_eq!(fixture.value("sol", "role"), "lead");
}

#[test]
fn failed_member_conversion_does_not_change_roles_or_other_legacy_leads() {
    let fixture = Fixture::new(
        "failed-conversion",
        json!([
            member("sol", json!({"squad.p.role": "lead"})),
            member("rin", json!({"squad.p.role": "lead"})),
        ]),
    );
    fs::write(fixture.root.join("fail-sol"), "").unwrap();
    let error = set(
        &fixture.core,
        &fixture.squad,
        "sol",
        &["role=reviewer".into()],
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "IDENTITY_METADATA_INVALID");
    assert!(!fixture.calls().contains("meta set squad.p.role"));
    assert!(!fixture.root.join("rin-squad.p.lead.marker").exists());
    let members = fixture.squad.roster(&fixture.core).unwrap();
    assert!(members.iter().all(squad::Member::is_lead));
}

#[test]
fn selecting_a_lead_changes_only_reserved_markers() {
    let fixture = Fixture::new(
        "move",
        json!([
            member("sol", json!({"squad.p.role": "lead"})),
            member("rin", json!({"squad.p.role": "reviews every merge"})),
        ]),
    );
    let outcome = lead(&fixture.core, &fixture.squad, "rin").unwrap();
    assert_eq!(outcome.document["replaced"], json!(["sol"]));
    assert_eq!(fixture.value("rin", "lead.marker"), "true");
    assert_eq!(fixture.value("sol", "lead.marker"), "false");
    assert!(!fixture.calls().contains("meta set squad.p.role"));
    assert!(!fixture.calls().contains("meta rm squad.p.role"));
}

#[test]
fn roster_reads_legacy_and_explicit_markers_without_writes() {
    let fixture = Fixture::new(
        "read-only",
        json!([
            member("legacy", json!({"squad.p.role": "lead"})),
            member(
                "lead",
                json!({"squad.p.role": "reviewer", "squad.p.lead.marker": "true"})
            ),
            member(
                "ordinary",
                json!({"squad.p.role": "lead", "squad.p.lead": "true", "squad.p.lead.marker": "false"})
            ),
        ]),
    );
    let members = fixture.squad.roster(&fixture.core).unwrap();
    assert_eq!(
        members
            .iter()
            .map(squad::Member::is_lead)
            .collect::<Vec<_>>(),
        vec![true, true, false]
    );
    for member in &members {
        assert!(!member.fields.contains_key(squad::LEAD_MARKER));
        assert!(
            !crate::status::row(member)["fields"]
                .as_object()
                .unwrap()
                .contains_key(squad::LEAD_MARKER)
        );
    }
    assert_eq!(
        members
            .iter()
            .map(|member| member.lead_marker)
            .collect::<Vec<_>>(),
        vec![None, Some(true), Some(false)]
    );
    assert!(!fixture.calls().contains("meta"));
}

#[test]
fn a_capped_previous_lead_fails_before_any_replacement_write() {
    let mut metadata = serde_json::Map::from_iter([("squad.p.role".into(), json!("lead"))]);
    for i in 0..63 {
        metadata.insert(format!("fixture{i}"), json!("value"));
    }
    let fixture = Fixture::new(
        "capped-previous",
        json!([
            member("sol", Value::Object(metadata)),
            member("rin", json!({"squad.p.role": "reviewer"})),
        ]),
    );
    let error = lead(&fixture.core, &fixture.squad, "rin").err().unwrap();
    assert_eq!(error.code, "IDENTITY_METADATA_INVALID");
    assert_eq!(
        error.message,
        "An identity may have at most 64 metadata entries."
    );
    let calls = fixture.calls();
    assert!(!calls.contains("meta set"));
    assert!(!calls.contains("room join"));
}

#[test]
fn retired_note_refuses_all_pairs_before_core_reads_or_writes() {
    let fixture = Fixture::new("retired-note", json!([member("sol", json!({}))]));
    let error = set(
        &fixture.core,
        &fixture.squad,
        "sol",
        &["task=new work".into(), "note=old summary".into()],
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "SQUAD_NOTE_RETIRED");
    let (what, hint) = error.human();
    assert_eq!(what, "The per-member note is retired");
    assert!(hint.unwrap().contains("tmt notes path --identity <member>"));
    assert!(hint.unwrap().contains("task="));
    assert!(hint.unwrap().contains("pending="));
    assert!(
        !fixture.root.join("calls").exists(),
        "validation precedes every core call"
    );
}

#[test]
fn legacy_note_is_excluded_on_read_and_can_still_be_cleared() {
    let fixture = Fixture::new(
        "clear-note",
        json!([member("sol", json!({"squad.p.note": "legacy"}))]),
    );
    let stored = fixture.root.join("sol-squad.p.note");
    fs::write(&stored, "legacy").unwrap();
    let members = fixture.squad.roster(&fixture.core).unwrap();
    assert!(!members[0].fields.contains_key("note"));
    assert_eq!(fs::read_to_string(&stored).unwrap(), "legacy");
    assert!(!fixture.calls().contains("meta rm"));
    let outcome = set(&fixture.core, &fixture.squad, "sol", &["note=".into()]).unwrap();
    assert!(outcome.complete);
    assert_eq!(outcome.document["applied"], json!(["squad.p.note"]));
    assert!(!stored.exists());
}
