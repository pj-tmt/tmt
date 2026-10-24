//! Literal switch stages through the production painter, never an alternate UI.
use super::*;
pub(in crate::board) const SCENES: &[&str] = &[
    "switch-tool-loading",
    "switch-tool-form",
    "switch-tool-model",
    "switch-tool-review",
    "switch-tool-copy-only",
    "switch-tool-pending",
    "switch-tool-success",
    "switch-tool-refused",
    "switch-tool-unknown",
    "switch-tool-stale",
    "switch-tool-load-failed",
];
pub(in crate::board) fn case(scene: &str, look: crate::look::Look) -> App {
    let mut app = if scene == "switch-tool-loading" {
        let mut app = app();
        app.open_switch_tool();
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
    if scene == "switch-tool-copy-only" {
        app.references.agents.get_mut(MEMBER).unwrap().parameters["location"]["host"] =
            json!("herdr");
    }
    let c = app.switch_tool.as_mut().unwrap();
    c.tools = vec!["codex".into(), "claude".into()];
    c.models
        .insert("claude".into(), vec!["opus".into(), "sonnet".into()]);
    if scene == "switch-tool-model" {
        c.fields[1] = Editor::from("opus");
        c.focus = 1;
    }
    if scene == "switch-tool-copy-only" {
        c.source.as_mut().unwrap().observation["location"]["host"] = json!("herdr");
    }
    if matches!(
        scene,
        "switch-tool-review" | "switch-tool-copy-only" | "switch-tool-pending"
    ) {
        c.review = Some(
            Review::new(
                c.source.as_ref().unwrap(),
                c.fields[0].as_str(),
                c.fields[1].as_str(),
                c.loaded.as_ref().unwrap(),
            )
            .unwrap(),
        );
        c.stage = if scene == "switch-tool-pending" {
            Stage::Pending
        } else {
            Stage::Review
        };
    }
    match scene {
        "switch-tool-success"=>c.stage=Stage::Result("worker is running claude in tmux / work.\nObserved model: opus\nName, UUID, notes and open requests stay.".into()),
        "switch-tool-refused"=>c.stage=Stage::Result("Host launch changed; review again.".into()),
        "switch-tool-unknown"=>c.stage=Stage::Result("Switch outcome unknown. No same-UUID running-driver observation arrived within the observation window.\nCheck the old agent and destination before retrying. Nothing was automatically relaunched or closed.".into()),
        "switch-tool-stale"=>c.stage=Stage::Result("Selected member or session observation changed. Close and review again; nothing launched.".into()),
        "switch-tool-load-failed"=>c.stage=Stage::Result("Host templates unavailable. Close and reopen to retry.".into()),
        _=>{},
    }
    app
}
pub(in crate::board) fn metadata(app: &App) -> serde_json::Value {
    let c = app.switch_tool.as_ref().unwrap();
    let rect = |r: Rect| [r.x, r.y, r.width, r.height];
    json!({"stage":format!("{:?}",c.stage),"focus":c.focus,"readable":c.readable.get(),
        "controls":c.controls.get().map(|r|r.map(rect)),"fields":c.fields_hit.get().map(|r|r.map(rect)),
        "choices":c.choices.borrow().iter().map(|(field,value,r)|json!({"field":field,"value":value,"rect":rect(*r)})).collect::<Vec<_>>(),
        "scroll":c.scroll.borrow().offset()})
}
#[test]
fn stages_fit_at_80_and_150_and_short_frames_never_admit_launch() {
    let _ui = crate::runner::forbid_commands();
    for scene in SCENES {
        for width in [80, 150] {
            for height in [43, 8] {
                let app = case(scene, crate::look::Look::default());
                let text = paint(&app, width, height);
                assert!(text.contains("Switch AI tool"), "{scene} {width}x{height}");
                if height == 8 {
                    assert!(!app.switch_tool.as_ref().unwrap().readable.get());
                }
                if height == 43
                    && matches!(
                        *scene,
                        "switch-tool-form" | "switch-tool-review" | "switch-tool-copy-only"
                    )
                {
                    assert!(
                        app.switch_tool.as_ref().unwrap().controls.get()[1].is_some(),
                        "{scene} {width}x{height}: {text}"
                    );
                }
            }
        }
    }
}
