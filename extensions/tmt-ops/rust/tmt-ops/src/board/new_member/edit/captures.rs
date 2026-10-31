//! Literal Edit states use the production form painter and painted Dialog hits.
use super::*;
use serde_json::{Value, json};

pub(in crate::board::new_member) fn case(scene: &str, look: crate::look::Look) -> App {
    let mut app = if scene.starts_with("edit-squad-") {
        let mut app = crate::board::app::workspace::tests::board();
        app.workspace.selected = Some(OpsTarget::Squad(
            crate::board::app::workspace::tests::ROOM.into(),
        ));
        app.workspace_reconcile();
        let Effect::NewMember(task) = app.open_member_edit() else {
            panic!("Edit schedules its existing worker lane")
        };
        app.finished_new_member(Completed {
            task,
            result: Ok(Outcome::Loaded(Box::new(super::super::tests::loaded()))),
        });
        app
    } else {
        super::super::tests::edit_member_form()
    };
    let workspace = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    workspace.squad_looks.clear();
    app.view.as_mut().unwrap().look = look;
    let c = app.new_member.as_mut().unwrap();
    match scene {
        "edit-member-form" | "edit-squad-form" => {}
        "edit-member-changes" | "edit-member-copy-only" => {
            c.draft.set(super::super::model::Slot::Name, "worker-two");
            c.draft
                .set(super::super::model::Slot::Model, "gpt-6.1-terra");
            c.draft.set(super::super::model::Slot::Effort, "high");
            if scene == "edit-member-copy-only" {
                c.loaded
                    .as_mut()
                    .unwrap()
                    .hosts
                    .iter_mut()
                    .find(|host| host.name == "tmux")
                    .unwrap()
                    .argv = None;
            }
        }
        "edit-member-list" => {
            c.open_field_list(Field::Member(super::super::model::Slot::Model));
        }
        "edit-member-kept" => {
            c.draft.set(super::super::model::Slot::Name, "worker-two");
            c.reopened = true;
        }
        "edit-member-pending" => c.stage = Stage::Launching,
        "edit-member-partial" => {
            c.stage = Stage::Result("✓ Applied · Name: worker-two\n? Unknown · Runtime launch: no new container was observed.\n— Not run · Remaining steps. Close and refresh before reviewing again; nothing is retried automatically.".into());
        }
        "edit-member-stale" => {
            app.references.agents.clear();
        }
        "edit-squad-changes" => {
            c.draft.set(super::super::model::Slot::Name, "product-two");
            let edit = c.edit.as_mut().unwrap();
            edit.enabled = !edit.enabled;
            edit.after = Editor::from("2h");
            edit.hidden = true;
        }
        "edit-squad-invalid" => c.edit.as_mut().unwrap().after = Editor::from("later"),
        "edit-squad-after-list" => c.open_field_list(Field::After),
        _ => panic!("unknown Edit scene {scene}"),
    }
    app
}

pub(in crate::board::new_member) fn metadata(app: &App) -> Value {
    let c = app.new_member.as_ref().unwrap();
    let edit = c.edit.as_ref().unwrap();
    let hits = edit.hits.borrow();
    let rect = |r: Rect| [r.x, r.y, r.width, r.height];
    let fields = if c.list.is_some() {
        hits.fields
            .iter()
            .map(|area| json!({"field": "Filter", "rect": rect(*area)}))
            .collect::<Vec<_>>()
    } else {
        edit.fields(&c.draft, c.loaded.as_ref())
            .iter()
            .zip(&hits.fields)
            .map(|(field, area)| json!({"field": field.debug_name(), "rect": rect(*area)}))
            .collect()
    };
    json!({
        "kind": "edit",
        "fields": fields,
        "choices": c.fields.borrow().iter().map(|(field, area)| json!({"field": format!("{field:?}"), "rect": rect(*area)})).collect::<Vec<_>>(),
        "buttons": hits.buttons.iter().map(|r| r.map(rect)).collect::<Vec<_>>(),
        "outer": rect(hits.outer), "content": rect(hits.content),
        "focus": format!("{:?}", edit.dialog.focus),
        "list": c.list.as_ref().map(|l| json!({"field": l.field.debug_name(), "filter": l.filter.as_str(), "row": l.row})),
        "stage": format!("{:?}", c.stage), "scroll": c.scroll.borrow().offset()
    })
}
