use super::*;
use crate::{config::Config, filter::Row};
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::{AtomicU64, Ordering};

const LEAD: &str = "11111111-1111-4111-8111-111111111111";
const MEMBER: &str = "22222222-2222-4222-8222-222222222222";

struct Fixture {
    root: PathBuf,
    config: PathBuf,
    squad: Squad,
    settings: Reminders,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "squad-staleness-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let config = root.join("squad.toml");
        fs::write(
            &config,
            "[squad.p.reminders]\nenabled = true\nstale_after = \"1m\"\n",
        )
        .unwrap();
        let settings = Config::read(config.clone())
            .unwrap()
            .reminders("p")
            .unwrap();
        Self {
            root,
            config,
            squad: Squad {
                name: "p".into(),
                room_id: "33333333-3333-4333-8333-333333333333".into(),
            },
            settings,
        }
    }
    fn observer(&self) -> Observer {
        Observer::at(
            &self.config,
            &self.squad,
            self.settings,
            Some(self.root.join("cache")),
        )
    }
    fn record(
        &self,
        members: &[Member],
        notes: Option<&Value>,
        room: Option<&Window>,
        now: u64,
    ) -> Snapshot {
        self.observer()
            .record(members, &[], &provider::Cache::at(None), notes, room, now)
    }
    fn document(&self, snapshot: &Snapshot, members: &[Member]) -> Value {
        // Duplicate a row across sections: consumers must see one UUID value.
        let lead = members
            .iter()
            .find(|member| member.is_lead())
            .map(crate::status::row);
        let rows: Vec<_> = members
            .iter()
            .filter(|member| !member.is_lead())
            .map(crate::status::row)
            .collect();
        let mut doc = json!({"squad": {"name": "p", "lead": lead}, "sections": [{"rows": rows}, {"rows": rows}]});
        snapshot.apply(&mut doc);
        doc
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn member(id: &str, fields: &[(&str, &str)]) -> Member {
    Member {
        lead_marker: None,
        id: id.into(),
        name: id.into(),
        lifetime: "saved".into(),
        presence: "unknown".into(),
        pane: Value::Null,
        activity: Value::Null,
        fields: fields
            .iter()
            .map(|(key, value)| ((*key).into(), (*value).into()))
            .collect(),
        meta: BTreeMap::new(),
        seen: Value::Null,
        numbers: BTreeMap::new(),
        failed: Default::default(),
    }
}
fn members() -> Vec<Member> {
    vec![
        member(LEAD, &[("role", "lead")]),
        member(MEMBER, &[("task", "review"), ("state", "working")]),
    ]
}
fn notes(text: &str) -> Value {
    json!({"identityId": LEAD, "content": text})
}

#[test]
fn off_creates_nothing_and_has_a_stable_projection() {
    let mut fixture = Fixture::new();
    fixture.settings.enabled = false;
    let observer = fixture.observer();
    assert!(!observer.active());
    let snapshot = observer.record(
        &members(),
        &[],
        &provider::Cache::at(None),
        None,
        None,
        90_000,
    );
    let doc = fixture.document(&snapshot, &members());
    assert_eq!(
        doc["sections"][0]["rows"][0]["staleness"],
        unavailable("disabled")
    );
    assert_eq!(doc["squad"]["notesStaleness"], unavailable("disabled"));
    assert!(!fixture.root.join("cache").exists());
}

#[test]
fn first_observation_is_fresh_and_the_exact_threshold_is_stale_across_reopens() {
    let fixture = Fixture::new();
    let members = members();
    let content = notes("Now");
    let first = fixture.record(&members, Some(&content), None, 1000);
    assert_eq!(first.members[MEMBER]["ageMs"], 0);
    let before = fixture.record(&members, Some(&content), None, 60_999);
    assert_eq!(before.members[MEMBER]["state"], "fresh");
    let stale = fixture.record(&members, Some(&content), None, 61_000);
    assert_eq!(stale.members[MEMBER]["state"], "stale");
    assert_eq!(stale.members[MEMBER]["unchangedSinceMs"], 1000);
    assert_eq!(stale.members[MEMBER]["activityAfterUpdate"], false);
    assert_eq!(stale.notes["state"], "stale");
    let doc = fixture.document(&stale, &members);
    assert_eq!(
        doc["sections"][0]["rows"][0]["staleness"],
        doc["sections"][1]["rows"][0]["staleness"]
    );
    assert_eq!(label(&stale.notes).as_deref(), Some("stale 1m"));
    // Cache has fingerprints, not notebook bytes; modes are private.
    let observer = fixture.observer();
    let path = observer.path.as_ref().unwrap();
    let cached = fs::read_to_string(path).unwrap();
    assert!(!cached.contains("Now"));
    assert_eq!(fs::metadata(path).unwrap().mode() & 0o777, 0o600);
    assert_eq!(
        fs::metadata(path.parent().unwrap()).unwrap().mode() & 0o777,
        0o700
    );
}

#[test]
fn only_raw_task_state_or_changed_note_content_reset_their_own_clock() {
    let fixture = Fixture::new();
    let mut rows = members();
    fixture.record(&rows, Some(&notes("one")), None, 1000);
    rows[1].fields.insert("note".into(), "new row note".into());
    rows[1]
        .fields
        .insert("pr_link".into(), "https://example.com/pull/1".into());
    let old = fixture.record(&rows, Some(&notes("one")), None, 61_000);
    assert_eq!(old.members[MEMBER]["state"], "stale");
    assert_eq!(old.members[MEMBER]["reasons"], json!(["pr_link_changed"]));
    rows[1].fields.insert("state".into(), "review".into());
    let changed = fixture.record(&rows, Some(&notes("one")), None, 62_000);
    assert_eq!(changed.members[MEMBER]["ageMs"], 0);
    assert_eq!(changed.members[MEMBER]["reasons"], json!([]));
    assert_eq!(changed.notes["state"], "stale");
    let notes_changed = fixture.record(&rows, Some(&notes("two")), None, 63_000);
    assert_eq!(notes_changed.members[MEMBER]["ageMs"], 1000);
    assert_eq!(notes_changed.notes["ageMs"], 0);
}

#[test]
fn unknown_notes_do_not_create_or_backdate_a_notebook_and_replacement_uuid_is_new() {
    let fixture = Fixture::new();
    let mut rows = members();
    let initial = fixture.record(&rows, None, None, 1000);
    assert_eq!(initial.notes, unavailable("unknown"));
    let first = fixture.record(&rows, Some(&notes("late")), None, 200_000);
    assert_eq!(first.notes["ageMs"], 0);
    let unreadable = fixture.record(&rows, None, None, 300_000);
    assert_eq!(unreadable.notes, unavailable("unknown"));
    rows[0].id = "44444444-4444-4444-8444-444444444444".into();
    rows[1].id = "55555555-5555-4555-8555-555555555555".into();
    let replacement = fixture.record(&rows, Some(&notes("late")), None, 400_000);
    assert_eq!(replacement.notes, unavailable("unknown"));
    assert_eq!(replacement.members[&rows[1].id]["ageMs"], 0);
    let persisted = fixture.observer().document;
    assert!(persisted["members"].get(MEMBER).is_none());
    assert!(persisted["notes"].is_null());
}

#[test]
fn a_member_final_needs_the_same_recipient_and_a_known_post_update_time() {
    let fixture = Fixture::new();
    let rows = members();
    fixture.record(&rows, None, None, 1000);
    let room = |to: &str, status: &str, at: u64| Window {
        complete: true,
        items: vec![
            json!({"kind":"request", "recipientId":to, "final":{"status":status,"submittedAtMs":at}}),
        ],
    };
    for wrong in [
        room(LEAD, "retained", 2000),
        room(MEMBER, "not_submitted", 2000),
        room(MEMBER, "retained", 999),
        room(MEMBER, "retained", 90_000),
    ] {
        let snapshot = fixture.record(&rows, None, Some(&wrong), 61_000);
        assert_eq!(snapshot.members[MEMBER]["activityAfterUpdate"], false);
    }
    let snapshot = fixture.record(&rows, None, Some(&room(MEMBER, "retained", 2000)), 61_000);
    assert_eq!(snapshot.members[MEMBER]["reasons"], json!(["member_final"]));
    let later = fixture.record(&rows, None, None, 62_000);
    assert_eq!(later.members[MEMBER]["reasons"], json!(["member_final"]));
}

#[test]
fn missing_fields_and_clock_rollback_are_unknown_and_do_not_invent_age() {
    let fixture = Fixture::new();
    let mut rows = members();
    fixture.record(&rows, Some(&notes("one")), None, 61_000);
    let rollback = fixture.record(&rows, Some(&notes("one")), None, 1000);
    assert_eq!(rollback.members[MEMBER], unavailable("unknown"));
    assert_eq!(rollback.notes, unavailable("unknown"));
    let next = fixture.record(&rows, Some(&notes("one")), None, 2000);
    assert_eq!(next.members[MEMBER]["ageMs"], 1000);
    rows[1].fields.clear();
    let missing = fixture.record(&rows, None, None, 3000);
    let doc = fixture.document(&missing, &rows);
    assert_eq!(
        doc["sections"][0]["rows"][0]["staleness"],
        unavailable("unknown")
    );
    assert_eq!(rows[1].value("state"), None);
}

#[test]
fn lock_contention_is_nonblocking_and_cannot_regress_the_owner_observation() {
    let fixture = Fixture::new();
    let owner = fixture.observer();
    assert!(owner.active());
    let contender = fixture.observer();
    assert!(!contender.active());
    let unknown = contender.record(&members(), &[], &provider::Cache::at(None), None, None, 50);
    assert_eq!(unknown.notes, unavailable("unknown"));
    owner.record(
        &members(),
        &[],
        &provider::Cache::at(None),
        None,
        None,
        1000,
    );
    let next = fixture.record(&members(), None, None, 61_000);
    assert_eq!(next.members[MEMBER]["unchangedSinceMs"], 1000);
}

#[test]
fn lost_corrupt_oversize_and_linked_cache_restart_grace_or_stay_unknown() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    fixture.record(&members(), None, None, 1000);
    let observer = fixture.observer();
    let path = observer.path.clone().unwrap();
    drop(observer);
    for contents in [b"not json".to_vec(), vec![b'x'; FILE_LIMIT as usize + 1]] {
        fs::write(&path, contents).unwrap();
        let next = fixture.record(&members(), None, None, 300_000);
        assert_eq!(next.members[MEMBER]["ageMs"], 0);
    }
    fs::remove_file(&path).unwrap();
    let lost = fixture.record(&members(), None, None, 400_000);
    assert_eq!(lost.members[MEMBER]["ageMs"], 0);
    fs::remove_file(&path).unwrap();
    let other = fixture.root.join("other");
    fs::write(&other, "untouched").unwrap();
    symlink(&other, &path).unwrap();
    fixture.record(&members(), None, None, 500_000);
    assert_eq!(fs::read_to_string(other).unwrap(), "untouched");
}

#[test]
fn config_edits_replacements_and_disable_reenable_preserve_matching_content_age() {
    let mut fixture = Fixture::new();
    let rows = members();
    let content = notes("one");
    fixture.record(&rows, Some(&content), None, 1000);
    let observing = fixture.observer();
    let replacement = fixture.root.join("next.toml");
    fs::write(
        &replacement,
        "# unrelated settings change\n[squad.p.reminders]\nenabled = true\nstale_after = \"1m\"\n",
    )
    .unwrap();
    fs::rename(replacement, &fixture.config).unwrap();
    let during_edit = observing.record(
        &rows,
        &[],
        &provider::Cache::at(None),
        Some(&content),
        None,
        61_000,
    );
    assert_eq!(during_edit.members[MEMBER]["unchangedSinceMs"], 1000);
    assert_eq!(during_edit.notes["state"], "stale");
    fixture.settings.enabled = false;
    let disabled = fixture.record(&rows, None, None, 62_000);
    assert_eq!(disabled.notes, unavailable("disabled"));
    fixture.settings.enabled = true;
    fixture.settings.stale_after = std::time::Duration::from_secs(120);
    let enabled = fixture.record(&rows, Some(&content), None, 63_000);
    assert_eq!(enabled.members[MEMBER]["unchangedSinceMs"], 1000);
    assert_eq!(enabled.members[MEMBER]["state"], "fresh");
    assert_eq!(enabled.notes["unchangedSinceMs"], 1000);
    let stale = fixture.record(&rows, Some(&content), None, 121_000);
    assert_eq!(stale.members[MEMBER]["state"], "stale");
    assert_eq!(stale.notes["state"], "stale");
}

#[test]
fn roots_rooms_and_observation_limits_do_not_share_history() {
    let fixture = Fixture::new();
    fixture.record(&members(), None, None, 1000);
    let mut other = Fixture::new();
    // A different config/data root using the same cache directory.
    let snapshot = Observer::at(
        &other.config,
        &fixture.squad,
        other.settings,
        Some(fixture.root.join("cache")),
    )
    .record(
        &members(),
        &[],
        &provider::Cache::at(None),
        None,
        None,
        61_000,
    );
    assert_eq!(snapshot.members[MEMBER]["ageMs"], 0);
    other.squad.room_id = "66666666-6666-4666-8666-666666666666".into();
    let snapshot = Observer::at(
        &fixture.config,
        &other.squad,
        other.settings,
        Some(fixture.root.join("cache")),
    )
    .record(
        &members(),
        &[],
        &provider::Cache::at(None),
        None,
        None,
        62_000,
    );
    assert_eq!(snapshot.members[MEMBER]["ageMs"], 0);
    let many = vec![members()[1].clone(); MEMBERS_LIMIT + 1];
    let snapshot = fixture.record(&many, None, None, 63_000);
    assert_eq!(snapshot.notes, unavailable("unknown"));
}

#[test]
fn successful_pr_transitions_persist_evidence_until_the_raw_row_changes() {
    let fixture = Fixture::new();
    let config: toml_edit::DocumentMut =
        "[squad.p.fields.pr_state]\npreset = \"github-pr\"\nevery = \"1m\"\n"
            .parse()
            .unwrap();
    let providers = provider::read(
        config["squad"]["p"].as_table_like(),
        "p",
        crate::rows::field_name,
        |field| crate::rows::OWN_FIELDS.contains(&field),
    )
    .unwrap();
    let mut rows = members();
    rows[1]
        .fields
        .insert("pr_link".into(), "https://example.com/pull/1".into());
    let job = provider::due(&providers, &rows, &provider::Cache::at(None), 1000)
        .pop()
        .unwrap();
    let mut cache = provider::Cache::at(None);
    let output = |state: &str| {
        let payload = json!({
            "number": 1,
            "state": if state == "draft" { "OPEN".to_owned() } else { state.to_uppercase() },
            "isDraft": state == "draft",
        });
        provider::github_pr(payload.to_string().as_bytes())
    };
    cache.record(&job, &output("draft"), 1000);
    let first = fixture
        .observer()
        .record(&rows, &providers, &cache, None, None, 1000);
    assert_eq!(first.members[MEMBER]["reasons"], json!([]));
    cache.record(&job, &provider::Outcome::Failed, 2000);
    let failed = fixture
        .observer()
        .record(&rows, &providers, &cache, None, None, 2000);
    assert_eq!(failed.members[MEMBER]["reasons"], json!([]));
    cache.record(&job, &output("open"), 3000);
    let open = fixture
        .observer()
        .record(&rows, &providers, &cache, None, None, 3000);
    assert_eq!(open.members[MEMBER]["reasons"], json!(["pr_opened"]));
    cache.record(&job, &output("merged"), 4000);
    let merged = fixture
        .observer()
        .record(&rows, &providers, &cache, None, None, 4000);
    assert_eq!(
        merged.members[MEMBER]["reasons"],
        json!(["pr_opened", "pr_merged"])
    );
    // Expiry supplies no new evidence and never renews the row's clock.
    let expired = fixture
        .observer()
        .record(&rows, &providers, &cache, None, None, 70_000);
    assert_eq!(expired.members[MEMBER]["state"], "stale");
    assert_eq!(
        expired.members[MEMBER]["reasons"],
        merged.members[MEMBER]["reasons"]
    );
    rows[1].fields.insert("task".into(), "next task".into());
    let updated = fixture
        .observer()
        .record(&rows, &providers, &cache, None, None, 71_000);
    assert_eq!(updated.members[MEMBER]["ageMs"], 0);
    assert_eq!(updated.members[MEMBER]["reasons"], json!([]));
}

#[test]
fn failed_publication_does_not_return_age_that_was_not_committed() {
    let fixture = Fixture::new();
    let observer = fixture.observer();
    // A directory at the target makes the atomic rename fail deterministically.
    fs::create_dir(observer.path.as_ref().unwrap()).unwrap();
    let snapshot = observer.record(
        &members(),
        &[],
        &provider::Cache::at(None),
        Some(&notes("one")),
        None,
        1000,
    );
    assert_eq!(snapshot.notes, unavailable("unknown"));
    assert!(snapshot.members.is_empty());
    assert!(
        fs::read_dir(fixture.root.join("cache"))
            .unwrap()
            .all(|entry| {
                !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp")
            })
    );
}
