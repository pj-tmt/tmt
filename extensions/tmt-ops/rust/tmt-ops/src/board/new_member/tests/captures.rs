//! Literal reviewed form states through the production overlay and frame painter.
use super::*;
pub(in crate::board) const SCENES: &[&str] = &[
    "new-member-loading",
    "new-member-form",
    "new-member-temporary",
    "new-member-review",
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
];
pub(in crate::board) fn case(scene: &str, look: crate::look::Look) -> App {
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
    match scene {
        "new-member-loading"|"new-member-form"=>{},
        "new-member-temporary"=>{c.draft.saved=false;c.focus=6;},
        "new-member-review"=>{c.review=Some(c.draft.review(c.loaded.as_ref().unwrap()).unwrap());c.stage=Stage::Review;},
        "new-member-copy-only"|"new-member-copy-temporary"=>{
            c.draft.fields[4]=Editor::from("herdr");c.draft.saved=scene!="new-member-copy-temporary";
            c.review=Some(c.draft.review(c.loaded.as_ref().unwrap()).unwrap());c.stage=Stage::Review;
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
        "new-member-load-failed"=>{c.loaded=None;c.notice=Some("Public host templates are unavailable. No launch attempted.".into());},
        _=>panic!("unknown new-member scene {scene}"),
    }
    app
}
pub(in crate::board) fn metadata(app: &App) -> serde_json::Value {
    let c = app.new_member.as_ref().unwrap();
    let rect = |r: Rect| [r.x, r.y, r.width, r.height];
    json!({"fields":c.fields.borrow().iter().map(|(index,area)|json!({"field":format!("{index:?}"),"rect":rect(*area)})).collect::<Vec<_>>(),"controls":c.controls.get().map(|r|r.map(rect)),"focus":c.focus,"readable":c.readable.get(),"stage":format!("{:?}",c.stage),"scroll":c.scroll.borrow().offset()})
}
#[test]
fn literal_scenes_paint_without_commands_at_the_admitted_and_short_sizes() {
    let _ui = crate::runner::forbid_commands();
    for scene in SCENES {
        for width in [80, 150] {
            for height in [43, 8] {
                let app = case(scene, crate::look::Look::default());
                let text = paint(&app, width, height);
                assert!(!text.is_empty());
                if height == 8 {
                    assert!(!app.new_member.as_ref().unwrap().readable.get(), "{scene}");
                }
            }
        }
    }
}
