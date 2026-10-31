//! Causal compound-save proofs on public wire fixtures; no real host or provider.
use super::*;
use crate::{
    board::app::workspace::tests::{MEMBER, ROOM},
    config::Config,
    core::Core,
};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
struct Fixture {
    root: PathBuf,
    core: Core,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("ops-edit-{}", crate::id::new_v4().unwrap()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("row.json"),json!({"id":MEMBER,"name":"worker","pane":"%old","driverSupport":"full","runningDriver":"codex","location":{"host":"tmux","session":"work"},"resume":{"driver":"codex","session":"thread-worker","model":"gpt-6.1-sol","observedModel":{"model":"gpt-6.1-sol","observedAtMs":100}}}).to_string()).unwrap();
        fs::write(
            root.join("room.json"),
            json!({"id":ROOM,"name":"squad-product","revision":2,"retired":false}).to_string(),
        )
        .unwrap();
        fs::write(root.join("preset.json"),json!({"identity":"worker","launch":{"executable":"codex","model":null,"effort":null,"associated":true}}).to_string()).unwrap();
        let script = root.join("core.py");
        fs::write(&script,r#"import json,pathlib,sys
r=pathlib.Path(__file__).parent
a=sys.argv[1:]
row=json.loads((r/'row.json').read_text())
with (r/'calls').open('a') as f:f.write(json.dumps({'argv':a})+'\n')
if a[0]=='ls':
 if (r/'launched').exists() and (r/'unknown-runtime').exists():
  print(json.dumps({'error':{'code':'READ_FAILED','message':'Observation unavailable'}}));sys.exit(1)
 print(json.dumps({'identities':[row]}))
elif a[:2]==['room','show']:print(json.dumps({'room':json.loads((r/'room.json').read_text())}))
elif a[:2]==['resume','--show']:print((r/'preset.json').read_text())
elif a[:2]==['host','templates']:print(json.dumps({'version':1,'hosts':[{'host':'tmux','state':'builtin','launch':{'argv':[str(r/'host'),'{session}','{command}'],'placeholders':['session','command']}}]}))
elif a[:2]==['config','show']:print(json.dumps({'paths':{'global':str(r/'config.json')},'resolved':{}}))
elif a[0]=='api':
 req=json.load(sys.stdin)
 if req['operation']=='storage.root':print(json.dumps({'dataRoot':str(r/'data')}))
 elif req['operation']=='identity.rename':
  value=req['input']
  with (r/'mutations').open('a') as f:f.write('rename\n')
  assert value['identityId']==row['id'] and value['expectedName']==row['name']
  if (r/'refuse-rename').exists():
   print(json.dumps({'error':{'code':'NAME_ALREADY_ACTIVE','message':'Name is taken'}}));sys.exit(5)
  row['name']=value['name'];(r/'row.json').write_text(json.dumps(row))
  if (r/'unknown-rename').exists():print('incomplete receipt');sys.exit(0)
  print(json.dumps({'identityId':row['id'],'outcome':'applied','name':row['name'],'changed':True}))
 else:raise Exception(req)
else:raise Exception(a)
"#).unwrap();
        crate::test_support::write_ready_executable(
            &root.join("core"),
            &format!(
                "#!/bin/sh\nexec /usr/bin/python3 {} \"$@\"\n",
                crate::launch::shell_word(script.to_str().unwrap())
            ),
        );
        let host = root.join("host.py");
        fs::write(
            &host,
            r#"import json,pathlib,shlex,sys
r=pathlib.Path(__file__).parent
with (r/'mutations').open('a') as f:f.write('launch\n')
a=shlex.split(sys.argv[2]);assert a[-2]=='--'
assert sum(v=='resume' for v in a)==1
row=json.loads((r/'row.json').read_text())
row['pane']='%new'
row['runningDriver']=next(v.split('=',1)[1] for v in a if v.startswith('--driver='))
model=next((v.split('=',1)[1] for v in a if v.startswith('--model=')),'gpt-6.1-sol')
row['resume']['observedModel']={'model':model,'observedAtMs':200}
(r/'row.json').write_text(json.dumps(row));(r/'launched').write_text('once')
"#,
        )
        .unwrap();
        crate::test_support::write_ready_executable(
            &root.join("host"),
            &format!(
                "#!/bin/sh\nexec /usr/bin/python3 {} \"$@\"\n",
                crate::launch::shell_word(host.to_str().unwrap())
            ),
        );
        Self {
            core: Core::at(root.join("core")),
            root,
        }
    }
    fn app(&self) -> App {
        let mut app = super::super::tests::edit_member_form();
        let c = app.new_member.as_mut().unwrap();
        let loaded = c.loaded.as_mut().unwrap();
        loaded.config = Config::read(self.root.join("ops.toml")).unwrap();
        loaded.executable = self.root.join("core").to_string_lossy().into();
        loaded.hosts=crate::launch::hosts(&json!({"version":1,"hosts":[{"host":"tmux","state":"builtin","launch":{"argv":[self.root.join("host"),"{session}","{command}"],"placeholders":["session","command"]}}]})).unwrap();
        app
    }
    fn plan(&self, app: &App) -> model::Plan {
        let c = app.new_member.as_ref().unwrap();
        c.edit
            .as_ref()
            .unwrap()
            .plan(&c.draft, c.loaded.as_ref().unwrap())
            .unwrap()
    }
    fn config(&self, app: &App) -> Config {
        app.new_member
            .as_ref()
            .unwrap()
            .loaded
            .as_ref()
            .unwrap()
            .config
            .clone()
    }
    fn mutations(&self) -> String {
        fs::read_to_string(self.root.join("mutations")).unwrap_or_default()
    }
    fn row(&self) -> Value {
        serde_json::from_slice(&fs::read(self.root.join("row.json")).unwrap()).unwrap()
    }
    fn write_row(&self, row: Value) {
        fs::write(self.root.join("row.json"), row.to_string()).unwrap();
    }
    fn flag(&self, name: &str) {
        fs::write(self.root.join(name), "").unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn save_noop_has_no_core_calls_and_name_only_uses_one_uuid_cas() {
    let f = Fixture::new();
    let mut app = f.app();
    let (message, _) = load::apply(&f.core, &f.plan(&app), f.config(&app)).unwrap();
    assert!(message.contains("Nothing changed"));
    assert!(!f.root.join("calls").exists());
    app.new_member
        .as_mut()
        .unwrap()
        .draft
        .set(super::super::model::Slot::Name, "new name");
    let (message, _) = load::apply(&f.core, &f.plan(&app), f.config(&app)).unwrap();
    assert!(message.contains("✓ Applied · Name: new name"));
    assert_eq!(f.row()["name"], "new name");
    assert_eq!(f.mutations(), "rename\n");
}
#[test]
fn stale_identity_room_config_and_preset_refuse_before_any_mutation() {
    for kind in ["identity", "room", "config", "preset"] {
        let f = Fixture::new();
        let mut app = f.app();
        let c = app.new_member.as_mut().unwrap();
        c.draft.set(super::super::model::Slot::Name, "new-name");
        if kind == "preset" {
            c.draft.set(super::super::model::Slot::Effort, "high");
        }
        let plan = f.plan(&app);
        let config = f.config(&app);
        match kind {
            "identity" => {
                let mut row = f.row();
                row["resume"]["session"] = json!("other-thread");
                f.write_row(row);
            }
            "room" => {
                fs::write(
                    f.root.join("room.json"),
                    json!({"id":ROOM,"name":"squad-product","revision":3,"retired":false})
                        .to_string(),
                )
                .unwrap();
            }
            "config" => {
                fs::write(f.root.join("ops.toml"), "# another editor\n").unwrap();
            }
            "preset" => {
                fs::write(f.root.join("preset.json"),json!({"identity":"worker","launch":{"executable":"codex","effort":"medium","associated":true}}).to_string()).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(load::apply(&f.core, &plan, config).is_err(), "{kind}");
        assert!(f.mutations().is_empty(), "{kind}");
        assert_eq!(f.row()["name"], "worker");
    }
}
#[test]
fn rename_then_one_same_driver_resume_uses_the_same_uuid_and_new_name() {
    let f = Fixture::new();
    let mut app = f.app();
    let c = app.new_member.as_mut().unwrap();
    c.draft.set(super::super::model::Slot::Name, "renamed");
    c.draft
        .set(super::super::model::Slot::Model, "gpt-6.1-terra");
    c.draft.set(super::super::model::Slot::Effort, "high");
    let (message, _) = load::apply(&f.core, &f.plan(&app), f.config(&app)).unwrap();
    assert_eq!(f.mutations(), "rename\nlaunch\n");
    assert_eq!(f.row()["id"], MEMBER);
    assert_eq!(f.row()["name"], "renamed");
    assert!(message.contains("✓ Applied · Runtime"), "{message}");
    assert!(message.contains("Observed model: gpt-6.1-terra"));
    assert!(message.contains("Requested effort: high"));
}
#[test]
fn refused_or_uncertain_rename_stops_before_launch_and_is_never_replayed() {
    for (flag, status) in [
        ("refuse-rename", "Not applied"),
        ("unknown-rename", "Unknown"),
    ] {
        let f = Fixture::new();
        let mut app = f.app();
        let c = app.new_member.as_mut().unwrap();
        c.draft.set(super::super::model::Slot::Name, "renamed");
        c.draft.set(super::super::model::Slot::Effort, "high");
        f.flag(flag);
        let (message, _) = load::apply(&f.core, &f.plan(&app), f.config(&app)).unwrap();
        assert_eq!(f.mutations(), "rename\n");
        assert!(message.contains(status), "{message}");
        assert!(message.contains("Not run"));
        assert_eq!(
            f.row()["name"],
            if flag == "refuse-rename" {
                "worker"
            } else {
                "renamed"
            }
        );
    }
}
#[test]
fn unknown_runtime_retains_applied_name_and_never_launches_twice() {
    let f = Fixture::new();
    let mut app = f.app();
    let c = app.new_member.as_mut().unwrap();
    c.draft.set(super::super::model::Slot::Name, "renamed");
    c.draft.set(super::super::model::Slot::Effort, "high");
    f.flag("unknown-runtime");
    let (message, _) = load::apply(&f.core, &f.plan(&app), f.config(&app)).unwrap();
    assert_eq!(f.mutations(), "rename\nlaunch\n");
    assert_eq!(f.row()["name"], "renamed");
    assert!(message.contains("✓ Applied · Name"));
    assert!(message.contains("Unknown · Runtime launch"));
}

impl Fixture {
    fn squad_app(&self) -> App {
        let mut app = crate::board::app::workspace::tests::board();
        app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
        app.workspace_reconcile();
        let Effect::NewMember(task) = app.open_member_edit() else {
            panic!()
        };
        let mut loaded = super::super::tests::loaded();
        loaded.config = Config::read(self.root.join("ops.toml")).unwrap();
        app.finished_new_member(Completed {
            task,
            result: Ok(Outcome::Loaded(Box::new(loaded))),
        });
        app
    }
}
#[test]
fn squad_plan_uses_squad_rename_and_the_confirmed_new_name_for_alerts() {
    let f = Fixture::new();
    let mut app = f.squad_app();
    let c = app.new_member.as_mut().unwrap();
    c.draft.set(super::super::model::Slot::Name, "renamed");
    let edit = c.edit.as_mut().unwrap();
    edit.enabled = !edit.enabled;
    edit.after = Editor::from("2h");
    edit.hidden = true;
    let plan = f.plan(&app);
    let lines = plan.lines("tmt");
    assert_eq!(lines[0], "tmt ops sq rename product renamed");
    assert!(
        lines
            .iter()
            .filter(|line| line.contains("config set"))
            .all(|line| line.ends_with("--squad renamed"))
    );
    assert_eq!(lines.last().unwrap(), "Hide squad · renamed");
    assert!(plan.membership.is_none());
    assert!(plan.runtime.is_none());
}
#[test]
fn squad_hide_uses_the_existing_preference_cas_and_returns_only_confirmed_publication() {
    let f = Fixture::new();
    let mut app = f.squad_app();
    app.new_member
        .as_mut()
        .unwrap()
        .edit
        .as_mut()
        .unwrap()
        .hidden = true;
    let (message, published) = load::apply(&f.core, &f.plan(&app), f.config(&app)).unwrap();
    assert!(message.contains("✓ Applied · Hide squad"));
    assert!(f.mutations().is_empty());
    let saved = Config::read(f.root.join("ops.toml"))
        .unwrap()
        .workspace_preferences()
        .unwrap();
    assert!(saved.hidden.contains(ROOM));
    assert_eq!(published, Some(saved));
}
#[test]
fn multiple_memberships_never_offer_common_pool_and_a_move_keeps_the_other_ids() {
    let f = Fixture::new();
    let mut app = f.app();
    let c = app.new_member.as_mut().unwrap();
    let edit = c.edit.as_mut().unwrap();
    let Target::Member { memberships, .. } = &mut edit.target else {
        panic!()
    };
    memberships.push("44444444-4444-4444-8444-444444444444".into());
    assert!(
        !c.edit_choices(super::super::model::Slot::Squad)
            .unwrap()
            .iter()
            .any(|choice| choice.value.as_deref() == Some(""))
    );
    c.draft.set(super::super::model::Slot::Squad, "");
    assert!(
        c.edit
            .as_ref()
            .unwrap()
            .plan(&c.draft, c.loaded.as_ref().unwrap())
            .is_err()
    );
    c.draft
        .rooms
        .push(crate::membership::workspace::RoomTarget {
            id: "44444444-4444-4444-8444-444444444444".into(),
            name: "other".into(),
            revision: 7,
        });
    c.draft.set(super::super::model::Slot::Squad, "other");
    let plan = f.plan(&app);
    let Some(crate::membership::workspace::Operation::Assign {
        source: Some(source),
        destination,
        move_member: true,
        ..
    }) = plan.membership
    else {
        panic!()
    };
    assert_eq!(source.id, ROOM);
    assert_eq!(destination.id, "44444444-4444-4444-8444-444444444444");
}
#[test]
fn unassigned_agent_plan_adds_a_membership_without_removing_a_room() {
    let f = Fixture::new();
    let mut app = f.app();
    let c = app.new_member.as_mut().unwrap();
    let edit = c.edit.as_mut().unwrap();
    let Target::Member {
        selection,
        source,
        memberships,
        ..
    } = &mut edit.target
    else {
        panic!()
    };
    *selection = OpsTarget::Member {
        room_id: None,
        identity_id: MEMBER.into(),
    };
    *source = None;
    memberships.clear();
    c.draft.set(super::super::model::Slot::Squad, "product");
    let plan = f.plan(&app);
    assert!(matches!(
        plan.membership,
        Some(crate::membership::workspace::Operation::Assign {
            source: None,
            move_member: false,
            ..
        })
    ));
}
#[test]
fn viewer_or_selection_drift_disables_the_painted_save_without_retargeting_the_draft() {
    let f = Fixture::new();
    let mut app = f.app();
    app.new_member
        .as_mut()
        .unwrap()
        .draft
        .set(super::super::model::Slot::Name, "renamed");
    let c = app.new_member.as_ref().unwrap();
    let edit = c.edit.as_ref().unwrap();
    assert!(edit.current_error(&app).is_none());
    assert!(edit.buttons(c, true).last().unwrap().is_enabled());
    app.view.as_mut().unwrap().me_id = Some("44444444-4444-4444-8444-444444444444".into());
    let c = app.new_member.as_ref().unwrap();
    let edit = c.edit.as_ref().unwrap();
    assert!(edit.current_error(&app).is_some());
    assert!(!edit.buttons(c, false).last().unwrap().is_enabled());
    assert_eq!(edit.target.name(), "worker");
    assert_eq!(c.draft.text(super::super::model::Slot::Name), "renamed");
}

fn input(app: &mut App, code: KeyCode) -> Effect {
    app.overlay_event(&Event::Key(ratatui::crossterm::event::KeyEvent::new(
        code,
        KeyModifiers::NONE,
    )))
    .unwrap()
}
fn paint(app: &App, width: u16, height: u16) -> String {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
#[test]
fn squad_after_uses_shared_presets_or_a_typed_duration_and_escape_closes_only_the_list() {
    let f = Fixture::new();
    let mut app = f.squad_app();
    input(&mut app, KeyCode::Down);
    input(&mut app, KeyCode::Down);
    input(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.new_member
            .as_ref()
            .unwrap()
            .list
            .as_ref()
            .unwrap()
            .field,
        Field::After
    ));
    let text = paint(&app, 100, 36);
    assert!(text.contains("15m") && text.contains("2h"));
    app.overlay_event(&Event::Paste("45m".into()));
    input(&mut app, KeyCode::Enter);
    assert_eq!(
        app.new_member
            .as_ref()
            .unwrap()
            .edit
            .as_ref()
            .unwrap()
            .after
            .as_str(),
        "45m"
    );
    input(&mut app, KeyCode::Enter);
    input(&mut app, KeyCode::Esc);
    assert!(app.new_member.as_ref().unwrap().list.is_none());
    assert!(app.new_member_shown());
    assert_eq!(
        app.new_member
            .as_ref()
            .unwrap()
            .edit
            .as_ref()
            .unwrap()
            .after
            .as_str(),
        "45m"
    );
}
#[test]
fn keep_current_choices_are_unique_so_arrows_can_reach_another_model() {
    let f = Fixture::new();
    let mut app = f.app();
    let c = app.new_member.as_mut().unwrap();
    let choices = c.edit_choices(super::super::model::Slot::Model).unwrap();
    assert_eq!(
        choices
            .iter()
            .filter(|choice| choice.value.as_deref() == Some("gpt-6.1-sol"))
            .count(),
        1
    );
    c.cycle(super::super::model::Slot::Model, false);
    assert_eq!(
        c.draft.text(super::super::model::Slot::Model),
        "gpt-6.1-terra"
    );
}
#[test]
fn a_missing_original_target_keeps_discard_accessible_but_save_disabled() {
    let f = Fixture::new();
    let mut app = f.app();
    app.new_member
        .as_mut()
        .unwrap()
        .draft
        .set(super::super::model::Slot::Name, "kept");
    input(&mut app, KeyCode::Esc);
    app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
    app.open_member_edit();
    let c = app.new_member.as_mut().unwrap();
    assert!(c.shown && c.reopened);
    c.edit.as_mut().unwrap().dialog.focus = tmt_tui::components::Focus::Buttons(1);
    paint(&app, 160, 36);
    let c = app.new_member.as_ref().unwrap();
    let edit = c.edit.as_ref().unwrap();
    assert!(!edit.buttons(c, false).last().unwrap().is_enabled());
    input(&mut app, KeyCode::Enter);
    assert!(app.new_member.is_none());
    app.open_member_edit();
    assert!(matches!(
        app.new_member
            .as_ref()
            .unwrap()
            .edit
            .as_ref()
            .unwrap()
            .target,
        Target::Squad(_)
    ));
}
#[test]
fn clipped_or_invalidated_discard_cannot_drop_the_draft_and_results_keep_their_scroll() {
    let f = Fixture::new();
    let mut app = f.app();
    input(&mut app, KeyCode::Esc);
    app.open_member_edit();
    app.new_member
        .as_mut()
        .unwrap()
        .edit
        .as_mut()
        .unwrap()
        .dialog
        .focus = tmt_tui::components::Focus::Buttons(1);
    paint(&app, 4, 3);
    input(&mut app, KeyCode::Enter);
    assert!(app.new_member.is_some());
    let c = app.new_member.as_mut().unwrap();
    c.stage = Stage::Result(
        (0..60)
            .map(|i| format!("Result line {i:02}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    paint(&app, 60, 20);
    input(&mut app, KeyCode::PageDown);
    let text = paint(&app, 60, 20);
    assert!(!text.contains("Result line 00"));
    assert!(app.new_member.as_ref().unwrap().scroll.borrow().offset() > 0);
}
#[test]
fn painted_save_hover_uses_the_shared_board_button_owner() {
    let f = Fixture::new();
    let mut app = f.app();
    app.new_member
        .as_mut()
        .unwrap()
        .draft
        .set(super::super::model::Slot::Name, "renamed");
    paint(&app, 160, 36);
    let rect = app
        .new_member
        .as_ref()
        .unwrap()
        .edit
        .as_ref()
        .unwrap()
        .hits
        .borrow()
        .buttons
        .last()
        .copied()
        .flatten()
        .unwrap();
    assert!(
        app.buttons.move_pointer(
            (rect.x, rect.y).into(),
            Some(crate::board::app::Overlay::NewMember)
        ),
        "paint registered the Save with its actual overlay owner"
    );
}
