use super::*;
use crate::board::glyph_guard::glyph_error;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

#[test]
fn authored_home_labels_use_only_registered_spaced_marks_and_structural_typography() {
    for width in [80, 100, 113, 160, 200] {
        let mut app = interaction::board(&[(
            "squad",
            document(
                "squad",
                row("L", "lead", "review"),
                vec![row("W", "worker", "idle")],
            ),
        )]);
        let summary = paint::summary(
            app.view.as_ref().unwrap().home.as_ref().unwrap(),
            width,
            app.look(),
        );
        assert_eq!(glyph_error(&summary.to_string()), None);
        app.tabs = std::iter::once(ALL.to_owned())
            .chain((0..30).map(|index| format!("squad-{index}")))
            .collect();
        app.attention = app
            .tabs
            .iter()
            .map(|name| {
                (
                    name.clone(),
                    crate::attention::Attention {
                        blocked: 1,
                        ..Default::default()
                    },
                )
            })
            .collect();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 30)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        for y in 0..30 {
            let line: String = (0..width)
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect();
            assert_eq!(glyph_error(&line), None, "width {width}, row {y}: {line}");
        }
    }
}

struct Fixture {
    root: PathBuf,
    config: Config,
    core: Core,
}

#[test]
fn workspace_action_rows_join_global_panes_without_replacing_roster_metadata() {
    let fixture = Fixture::new("");
    let roster = row("worker", "worker", "blocked");
    let acquired = fixture.acquired(&[("a", document("a", Value::Null, vec![roster]))]);
    let squad = Squad {
        name: "a".into(),
        room_id: "room-a".into(),
    };
    let listed = Ok(
        json!({"identities":[{"id":"worker", "name":"worker", "state":"working",
        "presence":"active", "pane":"%7", "location":{"host":"tmux"}}]}),
    );
    let snapshot = super::super::workspace::observe::OpsWorkspaceSnapshot::from_acquired(
        &[&squad],
        &acquired,
        &listed,
    );
    let target = super::super::workspace::OpsTarget::Member {
        room_id: Some("room-a".into()),
        identity_id: "worker".into(),
    };
    let member = &snapshot.rows[&target];
    assert_eq!(
        super::super::overview::actions::host(member, true),
        Ok("worker")
    );
    assert_eq!(member["state"], "blocked");
    assert_eq!(member["pane"]["id"], "%7");
}

#[test]
fn provider_activity_comes_only_from_the_global_core_observation() {
    let fixture = Fixture::new("");
    let mut roster = row("worker", "worker", "review");
    // A roster row never supplies activity, even if one carried a copy.
    roster["session"] = json!({"activity": {"state": "idle", "sinceMs": 1}});
    let unlisted = row("absent", "absent", "idle");
    let acquired = fixture.acquired(&[("a", document("a", Value::Null, vec![roster, unlisted]))]);
    let squad = Squad {
        name: "a".into(),
        room_id: "room-a".into(),
    };
    let listed = Ok(
        json!({"identities":[{"id":"worker", "name":"worker", "presence":"active",
        "session": {"activity": {"state": "working", "sinceMs": 5, "lastActivityMs": 6, "providers": {}}}}]}),
    );
    let snapshot = super::super::workspace::observe::OpsWorkspaceSnapshot::from_acquired(
        &[&squad],
        &acquired,
        &listed,
    );
    use super::super::activity::{ObservedActivity, Phase};
    let working = ObservedActivity {
        phase: Phase::Working,
        since_ms: Some(5),
        last_ms: Some(6),
    };
    assert_eq!(snapshot.roster.members["worker"].activity, working);
    assert_eq!(
        snapshot.roster.members["absent"].activity,
        ObservedActivity::default()
    );
    let squad_row = |id: &str| {
        &snapshot.rows[&super::super::workspace::OpsTarget::Member {
            room_id: Some("room-a".into()),
            identity_id: id.into(),
        }]
    };
    assert_eq!(ObservedActivity::of(squad_row("worker")), working);
    assert_eq!(squad_row("worker")["state"], "review");
    assert!(squad_row("absent")["session"].is_null());
}
#[test]
fn home_counts_one_agent_in_two_squads_once_from_core_activity() {
    let fixture = Fixture::new("");
    // The same UUID is a member of two squads, with the task "working" in both.
    let acquired = fixture.acquired(&[
        (
            "a",
            document("a", Value::Null, vec![row("W", "worker", "working")]),
        ),
        (
            "b",
            document("b", Value::Null, vec![row("W", "worker", "working")]),
        ),
    ]);
    let squads = [
        Squad {
            name: "a".into(),
            room_id: "room-a".into(),
        },
        Squad {
            name: "b".into(),
            room_id: "room-b".into(),
        },
    ];
    let listed = Ok(
        json!({"identities":[{"id":"W", "name":"worker", "presence":"active",
        "session": {"activity": {"state": "working", "sinceMs": 1, "lastActivityMs": 1, "providers": {}}}}]}),
    );
    let mut home = model(&["a".into(), "b".into()], &acquired);
    home.observe(
        super::super::workspace::observe::OpsWorkspaceSnapshot::from_acquired(
            &squads.iter().collect::<Vec<_>>(),
            &acquired,
            &listed,
        ),
    );
    assert_eq!((home.summary.working, home.summary.idle), (1, 0));
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
        let path = root.join("ops.toml");
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
        if !self.root.join("listed").exists() {
            fs::write(self.root.join("listed"), r#"{"identities":[]}"#).unwrap();
        }
        crate::test_support::write_ready_executable(
            self.core.executable(),
            &format!(
                r#"#!/bin/sh
printf '%s\n' "$*" >> '{root}/calls'
case "$1" in
  ls) cat '{root}/listed'; test ! -f '{root}/models-fail' ;;
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
fn aggregate_model_keeps_summary_and_acquired_squad_order() {
    let f = Fixture::new("");
    let empty = model(&[], &f.acquired(&[]));
    assert_eq!(empty.summary.members, 0);
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
    let home = model(&["shown".into()], &acquired);
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
    assert_eq!(home.squads[1].squad, "hidden");
    assert_eq!(home.squads[0].lead.as_ref().unwrap()["name"], "lead");
}

#[test]
fn acquired_squad_counts_deduplicate_authored_rows_and_keep_distinct_memberships() {
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
    let home = model(&["a".into(), "b".into()], &acquired);
    assert_eq!(home.summary.members, 4);
    assert_eq!(home.summary.waiting, 3);
    assert_eq!(home.summary.blocked, 3);
    assert_eq!(home.squads[0].squad, "a");
    assert_eq!(home.squads[1].squad, "b");
    assert_eq!(acquired.documents["a"]["sections"][1]["rows"][0]["id"], "A");
    assert_eq!(acquired.documents["b"]["sections"][0]["rows"][0]["id"], "A");
}

#[test]
fn partial_observations_and_failures_stay_explicit_in_the_aggregate_model() {
    let f = Fixture::new("");
    let mut blocked = row("B", "blocked", "blocked");
    blocked["staleness"] = json!({"unchangedSinceMs":101});
    let mut doc = document(
        "a",
        Value::Null,
        vec![blocked, row("U", "unknown-age", "blocked")],
    );
    doc["olderRequestsNotShown"] = json!(true);
    let mut acquired = f.acquired(&[("a", doc)]);
    acquired
        .failures
        .push(json!({"source":"inbox","error":{"code":"FAILED"}}));
    let home = model(&[], &acquired);
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
    let f = Fixture::new("[board.token_rate]\nenabled = false\nreduced_motion = true\n");
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
    fs::write(f.root.join("listed"), json!({"identities":[{"id":"id-a","resume":{"model":"known-model"}},{"id":"outside","resume":{"model":"outside-model"}}]}).to_string()).unwrap();
    f.install_core();
    let squads = squads();
    let order = vec!["b".into(), "a".into()];
    let me = Me {
        id: "me-id".into(),
        name: "me".into(),
    };
    let (home_public, home, rates) = load(
        &f.core,
        &f.config,
        &squads,
        &order,
        Some(&me),
        &f.core.json(&["ls"]),
    )
    .unwrap();
    let calls = fs::read_to_string(f.root.join("calls")).unwrap();
    assert_eq!(
        calls.lines().collect::<Vec<_>>(),
        [
            "ls --json",
            "inbox --identity me-id --limit 200 --json",
            "api",
            "api",
            "api"
        ]
    );
    let inputs = fs::read_to_string(f.root.join("inputs")).unwrap();
    let inputs = inputs
        .lines()
        .map(|input| serde_json::from_str::<Value>(input).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        inputs
            .iter()
            .map(|input| input["operation"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["rooms.roster", "rooms.roster", "digest.policy.show"]
    );
    assert_eq!(inputs[2]["input"]["identities"], json!(["id-a", "id-b"]));
    assert_eq!(rates.len(), squads.len());
    for name in ["a", "b"] {
        assert_eq!(rates[name].input.room, format!("room-{name}"));
        assert_eq!(
            rates[name].input.names[&format!("id-{name}")],
            format!("worker-{name}")
        );
        assert_eq!(
            rates[name].input.resumes.keys().collect::<Vec<_>>(),
            [&format!("id-{name}")]
        );
    }
    assert_eq!(rates["a"].input.resumes["id-a"]["model"], "known-model");
    assert!(!rates["a"].settings.enabled);
    assert!(home.reduced_motion);
    assert!(rates["a"].settings.reduced_motion);
    assert!(rates["b"].input.resumes["id-b"].is_null());
    assert!(
        rates
            .values()
            .all(|rate| !rate.input.resumes.contains_key("outside"))
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
    assert!(
        home.workspace
            .roster
            .members
            .values()
            .any(|member| member.name == "worker-a")
    );
}

#[test]
fn home_global_windows_do_not_replace_named_squad_overrides() {
    let f = Fixture::new(
        "[board]\ntok='1m/1h/2h'\n[board.token_rate]\nwindow='1h'\n[squad.a.board]\ntok='2m/5m/10m'\n",
    );
    fs::write(f.root.join("inbox"), r#"{"items":[],"more":false}"#).unwrap();
    for name in ["a", "b"] {
        fs::write(f.root.join(name), roster(name).to_string()).unwrap();
    }
    f.install_core();
    let (_, home, rates) = load(
        &f.core,
        &f.config,
        &squads(),
        &[],
        None,
        &f.core.json(&["ls"]),
    )
    .unwrap();
    assert_eq!(
        home.windows.map(|window| window.label()),
        ["1m", "1h", "2h"]
    );
    assert_eq!(
        rates["a"].settings.windows.map(|window| window.label()),
        ["2m", "5m", "10m"]
    );
    assert_eq!(rates["b"].settings.windows, home.windows);
    assert_eq!(home.window, crate::config::TokenWindow::HOUR);
    let mut published = crate::board::app::tests::snapshot(ALL, json!([]));
    let view = published.view.as_mut().unwrap();
    view.home = Some(home);
    view.home_rate = rates;
    let mut app = crate::board::app::App::new(Some(ALL.into()));
    app.apply(published);
    assert!(app.meter.is_none(), "aggregate uses per-squad rings");
    assert_eq!(app.token_window, crate::config::TokenWindow::HOUR);
    assert_eq!(app.next_token_window().unwrap().label(), "2h");
    app.token_window = crate::config::TokenWindow::MINUTE;
    let mut reloaded = crate::board::app::tests::snapshot(ALL, json!([]));
    reloaded.view = Ok(app.view.take().unwrap());
    app.apply(reloaded);
    assert_eq!(app.token_window, crate::config::TokenWindow::HOUR);
}

#[test]
fn failed_model_read_is_home_only_and_recovers_without_inventing_usage() {
    let f = Fixture::new("[board.token_rate]\nenabled = false\n");
    fs::write(f.root.join("inbox"), r#"{"items":[],"more":false}"#).unwrap();
    for name in ["a", "b"] {
        fs::write(f.root.join(name), roster(name).to_string()).unwrap();
    }
    fs::write(f.root.join("models-fail"), "").unwrap();
    f.install_core();
    let (public, home, rates) = load(
        &f.core,
        &f.config,
        &squads(),
        &[],
        None,
        &f.core.json(&["ls"]),
    )
    .unwrap();
    assert!(public.document.get("partial").is_none());
    assert!(
        !home.incomplete,
        "model failure does not invent truncated requests"
    );
    assert_eq!(home.failures.len(), 1);
    assert_eq!(home.failures[0]["source"], "models");
    assert!(
        rates
            .values()
            .all(|rate| rate.input.resumes.values().all(Value::is_null))
    );
    fs::remove_file(f.root.join("models-fail")).unwrap();
    let (recovered, home, _) = load(
        &f.core,
        &f.config,
        &squads(),
        &[],
        None,
        &f.core.json(&["ls"]),
    )
    .unwrap();
    assert!(!home.incomplete && home.failures.is_empty());
    assert_eq!(public.document, recovered.document);
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
    let (public, home, rates) = load(
        &f.core,
        &f.config,
        &squads(),
        &[],
        Some(&me),
        &f.core.json(&["ls"]),
    )
    .unwrap();
    assert_eq!(rates.keys().map(String::as_str).collect::<Vec<_>>(), ["b"]);
    assert_eq!(public.document["partial"], true);
    assert_eq!(home.failures.len(), 2);
    assert_eq!(home.squads.len(), 1);
    assert_eq!(home.squads[0].squad, "b");
    fs::remove_file(f.root.join("inbox-fail")).unwrap();
    fs::remove_file(f.root.join("a-fail")).unwrap();
    fs::write(f.root.join("inbox"), "{\"items\":[],\"more\":false}").unwrap();
    fs::write(f.root.join("a"), roster("a").to_string()).unwrap();
    let (public, home, _) = load(
        &f.core,
        &f.config,
        &squads(),
        &[],
        Some(&me),
        &f.core.json(&["ls"]),
    )
    .unwrap();
    assert!(public.document.get("partial").is_none());
    assert!(home.failures.is_empty());
    assert_eq!(home.squads.len(), 2);
}

mod cache;
mod cron;
mod interaction;
mod light_dismiss;
mod limits;
pub(in crate::board) mod oracle;

#[test]
fn aggregate_state_counts_preserve_public_attention_semantics_including_the_lead() {
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
    let home = model(&["a".into()], &acquired);
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
    let home = model(&["a".into()], &acquired);
    assert_eq!(home.summary.members, 1);
    assert_eq!(home.summary.idle, 1);
}
