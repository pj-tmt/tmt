use super::*;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture {
    root: PathBuf,
    config: Config,
    core: Core,
}
impl Fixture {
    fn new(text: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "squad-home-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("squad.toml");
        fs::write(&path, text).unwrap();
        Self {
            config: Config::read(path).unwrap(),
            core: Core::at(root.join("core")),
            root,
        }
    }
    fn acquired(&self, documents: &[(&str, Value)]) -> Acquired {
        let mut acquired = tab_view::roster_documents(&self.core, &self.config, &[], None, None);
        for (name, document) in documents {
            acquired.include(&self.config, name, document.clone());
        }
        acquired
    }
    fn install_core(&self) {
        crate::test_support::write_ready_executable(
            self.core.executable(),
            &format!(
                r#"#!/bin/sh
printf '%s\n' "$*" >> '{root}/calls'
case "$1" in
  inbox) cat '{root}/inbox'; test ! -f '{root}/inbox-fail' ;;
  api)
    input=$(cat)
    printf '%s\n' "$input" >> '{root}/inputs'
    case "$input" in
      *room-a*) cat '{root}/a'; test ! -f '{root}/a-fail' ;;
      *room-b*) cat '{root}/b' ;;
      *) exit 1 ;;
    esac ;;
  *) exit 1 ;;
esac
"#,
                root = self.root.display()
            ),
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn row(id: &str, name: &str, state: &str) -> Value {
    json!({"id": id, "name": name, "state": state, "pending": null, "fields": {"state": state}, "waitingOnYou": []})
}
fn document(name: &str, lead: Value, rows: Vec<Value>) -> Value {
    json!({"squad": {"name": name, "lead": lead}, "sections": [{"rows": rows}]})
}

#[test]
fn empty_and_quiet_home_keep_only_current_sections_and_squad_order() {
    let f = Fixture::new("");
    let empty = model(&[], &f.acquired(&[]), 100);
    assert_eq!(empty.summary.members, 0);
    assert_eq!(empty.sections.len(), 2);
    assert_eq!(empty.sections[0].key, "needs-you");
    assert_eq!(empty.sections[1].key, "blocked");
    assert!(empty.squads.is_empty());
    assert!(!empty.incomplete);
    let acquired = f.acquired(&[
        (
            "hidden",
            document(
                "hidden",
                Value::Null,
                vec![row("H", "hidden-worker", "working")],
            ),
        ),
        (
            "shown",
            document(
                "shown",
                row("L", "lead", "idle"),
                vec![row("W", "worker", "review")],
            ),
        ),
    ]);
    let home = model(&["shown".into()], &acquired, 100);
    assert_eq!(
        home.summary,
        Counts {
            members: 3,
            waiting: 0,
            blocked: 0,
            review: 1,
            working: 1,
            idle: 1
        }
    );
    assert_eq!(home.squads[0].squad, "shown");
    assert_eq!(
        home.squads[0].counts,
        Counts {
            members: 2,
            review: 1,
            idle: 1,
            ..Default::default()
        }
    );
    assert_eq!(home.squads[1].squad, "hidden");
    assert_eq!(home.squads[0].lead.as_ref().unwrap()["name"], "lead");
    assert_eq!(home.squads[0].members.members, 1);
    assert_eq!(home.squads[0].members.review, 1);
    assert!(home.sections.iter().all(|section| section.rows.is_empty()));
}

#[test]
fn shared_sections_deduplicate_within_a_squad_and_keep_cross_squad_memberships() {
    let f = Fixture::new("");
    let mut waiting = row("A", "asker", "blocked");
    waiting["waitingOnYou"] = json!([{"requestId":"q-new","preparedAtMs":90,"preview":"private question"}, {"requestId":"q-old","preparedAtMs":20}]);
    waiting["staleness"] = json!({"unchangedSinceMs": 10});
    let mut pending = row("P", "pending-lead", "working");
    pending["pending"] = json!("decision");
    let blocked = {
        let mut row = row("B", "blocked-worker", "blocked");
        row["staleness"] = json!({"unchangedSinceMs": 30});
        row
    };
    let mut a = document("a", pending.clone(), vec![waiting.clone(), blocked.clone()]);
    a["sections"]
        .as_array_mut()
        .unwrap()
        .push(json!({"rows":[waiting.clone(),pending]}));
    let acquired = f.acquired(&[("a", a), ("b", document("b", Value::Null, vec![waiting]))]);
    let home = model(&["a".into(), "b".into()], &acquired, 100);
    assert_eq!(home.summary.members, 4);
    assert_eq!(home.summary.waiting, 3);
    assert_eq!(home.summary.blocked, 3);
    let needs = &home.sections[0].rows;
    assert_eq!(needs.len(), 3);
    assert_eq!(needs[0].member["name"], "asker");
    assert_eq!(
        needs[0].age,
        Some(Age {
            source: AgeSource::Request,
            since_ms: 20
        })
    );
    assert_eq!(needs[0].lead.as_deref().unwrap(), "pending-lead");
    assert_eq!(
        needs[0].member["waitingOnYou"][0]["preview"],
        "private question"
    );
    assert_eq!(needs[1].member["name"], "pending-lead");
    assert_eq!(needs[1].age, None, "pending text cannot establish age");
    assert_eq!(needs[2].squad, "b");
    // Shared user sections retain overlapping matches; a waiting blocked
    // member appears in both while squad summary counts it once per state.
    let blocked = &home.sections[1].rows;
    assert_eq!(blocked.len(), 3);
    assert_eq!(blocked[0].member["id"], "A");
    assert_eq!(blocked[1].member["id"], "B");
    assert_eq!(
        blocked[1].age,
        Some(Age {
            source: AgeSource::Observed,
            since_ms: 30
        })
    );
    assert_eq!(home.squads[0].members.members, 2);
    assert_eq!(home.squads[0].members.waiting, 1);
    assert_eq!(home.squads[0].members.blocked, 1);
}

#[test]
fn ages_require_authoritative_nonfuture_timestamps_and_partial_is_visible() {
    let f = Fixture::new("");
    let mut request = row("R", "request", "working");
    request["waitingOnYou"] =
        json!([{"preparedAtMs":101},{"preparedAtMs":"20"},{"requestId":"missing-time"}]);
    let mut blocked = row("B", "blocked", "blocked");
    blocked["staleness"] = json!({"unchangedSinceMs":101});
    let mut doc = document(
        "a",
        Value::Null,
        vec![request, blocked, row("U", "unknown-age", "blocked")],
    );
    doc["olderRequestsNotShown"] = json!(true);
    let mut acquired = f.acquired(&[("a", doc)]);
    acquired
        .failures
        .push(json!({"source":"inbox","error":{"code":"FAILED"}}));
    let home = model(&[], &acquired, 100);
    for section in &home.sections {
        for row in &section.rows {
            assert!(row.age.is_none());
        }
    }
    assert!(home.incomplete);
    assert_eq!(home.failures[0]["source"], "inbox");
}

fn squads() -> Vec<Squad> {
    ["a", "b"]
        .map(|name| Squad {
            name: name.into(),
            room_id: format!("room-{name}"),
        })
        .into()
}
fn roster(name: &str) -> Value {
    json!({"members":[{"id":format!("id-{name}"),"name":format!("worker-{name}"),"lifetime":"saved", "status":null,
        "metadata":{format!("squad.{name}.state"):"blocked",format!("squad.{name}.task"):"build"}}]})
}

#[test]
fn home_acquisition_reuses_public_reads_and_preserves_all_json_and_text() {
    let f = Fixture::new("");
    fs::write(
        f.root.join("inbox"),
        json!({"items":[{
        "requestId":"q-a", "from":{"identityId":"id-a"},
        "preparedAtMs":20, "preview":"private question"
    }],"more":true})
        .to_string(),
    )
    .unwrap();
    for name in ["a", "b"] {
        fs::write(f.root.join(name), roster(name).to_string()).unwrap();
    }
    f.install_core();
    let squads = squads();
    let order = vec!["b".into(), "a".into()];
    let me = Me {
        id: "me-id".into(),
        name: "me".into(),
    };
    let (home_public, home, rates) = load(&f.core, &f.config, &squads, &order, Some(&me)).unwrap();
    let calls = fs::read_to_string(f.root.join("calls")).unwrap();
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        ["inbox --identity me-id --limit 200 --json", "api", "api"]
    );
    let inputs = fs::read_to_string(f.root.join("inputs")).unwrap();
    for input in inputs.lines() {
        let input: Value = serde_json::from_str(input).unwrap();
        assert_eq!(input["operation"], "rooms.roster");
    }
    assert_eq!(rates.len(), squads.len());
    for name in ["a", "b"] {
        assert_eq!(rates[name].input.room, format!("room-{name}"));
        assert_eq!(
            rates[name].input.resumes.keys().collect::<Vec<_>>(),
            [&format!("id-{name}")]
        );
    }
    assert!(
        rates
            .values()
            .all(|rate| rate.input.resumes.values().all(Value::is_null))
    );
    let public = tab_view::load(&f.core, &f.config, &squads, &order, Some(&me), ALL).unwrap();
    assert_eq!(
        serde_json::to_vec(&home_public.document).unwrap(),
        serde_json::to_vec(&public.document).unwrap()
    );
    let terminal = tmt_cli_style::Terminal::PLAIN;
    assert_eq!(
        crate::status::text(&home_public.document, terminal),
        crate::status::text(&public.document, terminal)
    );
    assert_eq!(home.summary.members, 2);
    assert_eq!(home.summary.blocked, 2);
    assert_eq!(home.summary.waiting, 1);
    assert!(home.incomplete);
    assert_eq!(
        home.sections[0].rows[0].age,
        Some(Age {
            source: AgeSource::Request,
            since_ms: 20
        })
    );
    assert_eq!(home.sections[1].rows[0].member["name"], "worker-a");
    assert!(home.sections[1].rows.iter().all(|row| row.age.is_none()));
}

#[test]
fn failed_roster_and_inbox_are_reported_and_recovery_replaces_the_partial_model() {
    let f = Fixture::new("");
    fs::write(
        f.root.join("inbox"),
        "{\"error\":{\"code\":\"INBOX_FAILED\",\"message\":\"fixture\"}}",
    )
    .unwrap();
    fs::write(f.root.join("inbox-fail"), "").unwrap();
    fs::write(f.root.join("a-fail"), "").unwrap();
    fs::write(
        f.root.join("a"),
        "{\"error\":{\"code\":\"ROSTER_FAILED\",\"message\":\"fixture\"}}",
    )
    .unwrap();
    fs::write(f.root.join("b"), roster("b").to_string()).unwrap();
    f.install_core();
    let me = Me {
        id: "me-id".into(),
        name: "me".into(),
    };
    let (public, home, rates) = load(&f.core, &f.config, &squads(), &[], Some(&me)).unwrap();
    assert_eq!(rates.keys().map(String::as_str).collect::<Vec<_>>(), ["b"]);
    assert_eq!(public.document["partial"], true);
    assert_eq!(home.failures.len(), 2);
    assert_eq!(home.squads.len(), 1);
    assert_eq!(home.squads[0].squad, "b");
    fs::remove_file(f.root.join("inbox-fail")).unwrap();
    fs::remove_file(f.root.join("a-fail")).unwrap();
    fs::write(f.root.join("inbox"), "{\"items\":[],\"more\":false}").unwrap();
    fs::write(f.root.join("a"), roster("a").to_string()).unwrap();
    let (public, home, _) = load(&f.core, &f.config, &squads(), &[], Some(&me)).unwrap();
    assert!(public.document.get("partial").is_none());
    assert!(home.failures.is_empty());
    assert_eq!(home.squads.len(), 2);
}

mod interaction;

#[test]
fn tile_members_exclude_the_lead_and_choose_one_urgent_mark_per_membership() {
    let fixture = Fixture::new("");
    let mut lead = row("L", "lead", "blocked");
    lead["pending"] = json!("lead decision");
    let mut waiting = row("W", "waiting", "blocked");
    waiting["waitingOnYou"] = json!([{"requestId":"q"}]);
    let acquired = fixture.acquired(&[(
        "a",
        document(
            "a",
            lead,
            vec![
                waiting,
                row("B", "blocked", "blocked"),
                row("R", "review", "review"),
                row("A", "active", "working"),
                row("I", "idle", "idle"),
                row("C", "custom", "investigating"),
            ],
        ),
    )]);
    let home = model(&["a".into()], &acquired, 100);
    assert_eq!(
        home.squads[0].members,
        Counts {
            members: 6,
            waiting: 1,
            blocked: 1,
            review: 1,
            working: 1,
            idle: 1,
        }
    );
    assert_eq!(home.summary.members, 7);
    assert_eq!(home.summary.waiting, 2);
    assert_eq!(
        home.summary.blocked, 3,
        "whole-roster summary keeps its public attention semantics"
    );
    let acquired = fixture.acquired(&[(
        "a",
        document("a", Value::Null, vec![row("W", "worker", "idle")]),
    )]);
    let home = model(&["a".into()], &acquired, 100);
    assert_eq!(home.squads[0].members.members, 1);
    assert_eq!(home.squads[0].members.idle, 1);
}
