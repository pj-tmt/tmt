//! Literal reviewed form states through the production overlay and frame painter.
use super::*;
pub(in crate::board) const SCENES: &[&str] = &[
    "edit-member-rename",
    "edit-squad-rename",
    "edit-member-rename-review",
    "edit-member-form",
    "edit-member-changes",
    "edit-member-list",
    "edit-member-kept",
    "edit-member-pending",
    "edit-member-partial",
    "edit-member-stale",
    "edit-member-copy-only",
    "edit-squad-form",
    "edit-squad-changes",
    "edit-squad-invalid",
    "edit-squad-after-list",
    "new-member-loading",
    "new-member-form",
    "new-member-reopened",
    "new-member-temporary",
    "new-member-list",
    "new-member-list-filtered",
    "new-member-built-in",
    "new-member-copy-only",
    "new-member-copy-temporary",
    "new-member-template",
    "new-member-template-reset",
    "new-member-template-error",
    "new-member-settings",
    "new-member-pending",
    "new-member-success",
    "new-member-partial",
    "new-member-refused",
    "new-member-load-failed",
    "new-member-tool-list",
    "new-member-plain",
    "new-member-plain-command",
];
pub(in crate::board) fn case(scene: &str, look: crate::look::Look) -> App {
    if scene.starts_with("edit-") {
        return super::super::edit::captures::case(scene, look);
    }
    let mut app = if scene == "new-member-loading" {
        let mut app = board();
        app.open_new_member();
        app
    } else {
        form()
    };
    app.view.as_mut().unwrap().look = look;
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .squad_looks
        .clear();
    let c = app.new_member.as_mut().unwrap();
    c.draft.fields[2] = Editor::from("gpt-6.1-sol");
    // The plain-shell scenes show the login shell; fix it so the capture is the same on every host.
    c.draft.shell = "/bin/zsh".into();
    match scene {
        "new-member-loading"|"new-member-form"|"new-member-reopened"=>{},
        "new-member-temporary"=>{c.draft.saved=false;c.at=Focus::Slot(model::Slot::Keep);},
        "new-member-list"=>{c.at=Focus::Slot(model::Slot::Model);c.open_list(model::Slot::Model);},
        "new-member-list-filtered"=>{c.open_list(model::Slot::Model);c.list.as_mut().unwrap().filter=Editor::from("terra");},
        "new-member-built-in"=>{c.draft.set(model::Slot::Host,model::BUILT_IN_HOST);c.draft.set(model::Slot::Effort,"high");c.at=Focus::Create;},
        "new-member-copy-only"|"new-member-copy-temporary"=>{
            c.draft.fields[4]=Editor::from("herdr");c.draft.saved=scene!="new-member-copy-temporary";c.at=Focus::Create;
        },
        "new-member-template"|"new-member-template-error"=>{
            c.begin_template();
            if scene=="new-member-template-error" {c.notice=Some("The file changed since this form opened. Draft kept; no override saved.".into());}
        },
        "new-member-template-reset"=>{c.notice=Some("Launch template reset to Core default. Member draft kept.".into());},
        "new-member-settings"=>{c.destination=Destination::Settings;c.stage=Stage::Hosts;},
        "new-member-pending"=>{c.review=Some(c.draft.review(c.loaded.as_ref().unwrap()).unwrap());c.stage=Stage::Launching;},
        "new-member-success"=>c.stage=Stage::Result("Created new-worker in product.".into()),
        "new-member-partial"=>c.stage=Stage::Result("Created new-worker in Common pool; squad add failed: The displayed squad changed.\nRepair after checking: tmt ops sq add --squad product -- 33333333-3333-4333-8333-333333333333".into()),
        "new-member-refused"=>c.notice=Some("That identity already exists; choose a new name.".into()),
        "new-member-tool-list"=>{c.at=Focus::Slot(model::Slot::Tool);c.open_list(model::Slot::Tool);},
        "new-member-plain"|"new-member-plain-command"=>{
            c.draft.set(model::Slot::Tool,model::PLAIN_TOOL);
            if scene=="new-member-plain-command" {c.draft.set(model::Slot::Command,"htop -d 10 'my dir'");}
            c.at=Focus::Create;
        },
        "new-member-load-failed"=>{c.loaded=None;c.notice=Some("Public host templates are unavailable. No launch attempted.".into());},
        _=>panic!("unknown new-member scene {scene}"),
    }
    // A kept draft opened again: Cancel · Discard · Create.
    if scene == "new-member-reopened" {
        key(&mut app, KeyCode::Esc);
        app.open_new_member();
    }
    app
}
pub(in crate::board) fn metadata(app: &App) -> serde_json::Value {
    let c = app.new_member.as_ref().unwrap();
    if c.edit.is_some() {
        return super::super::edit::captures::metadata(app);
    }
    let rect = |r: Rect| [r.x, r.y, r.width, r.height];
    json!({"fields":c.fields.borrow().iter().map(|(index,area)|json!({"field":format!("{index:?}"),"rect":rect(*area)})).collect::<Vec<_>>(),"controls":c.controls.get().map(|r|r.map(rect)),"focus":c.focus,"at":format!("{:?}",c.at),"list":c.list.as_ref().map(|l|json!({"slot":l.field.debug_name(),"filter":l.filter.as_str(),"row":l.row})),"readable":c.readable.get(),"stage":format!("{:?}",c.stage),"scroll":c.scroll.borrow().offset()})
}
#[test]
fn literal_scenes_paint_without_commands_at_the_admitted_and_short_sizes() {
    let _ui = crate::runner::forbid_commands();
    for scene in SCENES {
        for width in [60, 80, 150, 160] {
            for height in [43, 8] {
                let app = case(scene, crate::look::Look::default());
                let text = paint(&app, width, height);
                assert!(!text.is_empty());
                if height == 8 && !scene.starts_with("edit-") {
                    assert!(!app.new_member.as_ref().unwrap().readable.get(), "{scene}");
                }
            }
        }
    }
}
