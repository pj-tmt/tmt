//! Literal idle-alert states through the production frame and input controller.
use super::*;
pub(in crate::board) const SCENES: &[&str] = &[
    "idle-editor-list-on",
    "idle-editor-hover",
    "idle-editor-list-off",
    "idle-editor-form",
    "idle-editor-off",
    "idle-editor-impact",
    "idle-editor-invalid",
    "idle-editor-saving",
    "idle-editor-saved",
    "idle-editor-refused",
    "idle-editor-partial",
    "idle-editor-loading",
    "idle-editor-unavailable",
    "idle-editor-pool",
    "idle-editor-chooser",
    "idle-editor-unknown",
];
pub(in crate::board) fn case(scene: &str, look: crate::look::Look) -> App {
    let mut app = if matches!(
        scene,
        "idle-editor-loading"
            | "idle-editor-unavailable"
            | "idle-editor-pool"
            | "idle-editor-chooser"
    ) {
        board()
    } else {
        form()
    };
    app.view.as_mut().unwrap().look = look;
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    snapshot.squad_looks.clear();
    let squad = &mut snapshot.roster.squads[0];
    squad.lead = Some(MEMBER.into());
    squad.idle_alert = Some(crate::board::workspace::IdleAlertSetting {
        enabled: scene != "idle-editor-list-off",
        after_ms: 1_800_000,
    });
    squad.member_states.get_mut(MEMBER).unwrap().idle_over = scene != "idle-editor-list-off";
    let member = snapshot.roster.members.get_mut(MEMBER).unwrap();
    member.activity.phase = crate::board::activity::Phase::Idle;
    member.activity.since_ms = Some(crate::status::now_ms() - 32 * 60_000);
    match scene {
        "idle-editor-list-on" | "idle-editor-list-off" | "idle-editor-hover" => {
            app.idle_alert_editor = None;
            app.workspace.selected = Some(OpsTarget::Squad(ROOM.into()));
            if scene == "idle-editor-hover" {
                app.idle_alert_hover = Some(ROOM.into());
            }
        }
        "idle-editor-loading" | "idle-editor-unavailable" => {
            app.open_idle_alert(None);
            if scene == "idle-editor-unavailable" {
                let c = app.idle_alert_editor.as_mut().unwrap();
                c.pending = false;
                c.notice = Some("✗ Refused · configuration unavailable".into());
            }
        }
        "idle-editor-pool" => {
            app.workspace.selected = Some(OpsTarget::Pool);
            app.open_idle_alert(None);
        }
        "idle-editor-chooser" => {
            let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
            let squad = &snapshot.roster.squads[0];
            snapshot
                .roster
                .squads
                .push(crate::board::workspace::OpsSquad {
                    room_id: "99999999-9999-4999-8999-999999999999".into(),
                    name: "platform".into(),
                    members: squad.members.clone(),
                    lead: None,
                    member_states: squad.member_states.clone(),
                    observation: squad.observation,
                    idle_alert: squad.idle_alert,
                });
            app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
            app.workspace_reconcile();
            app.open_idle_alert(None);
        }
        _ => {
            let c = app.idle_alert_editor.as_mut().unwrap();
            c.config = Some(
                c.config
                    .as_ref()
                    .unwrap()
                    .preview_setting(Some("product"), "idle_alert.enabled", "true")
                    .unwrap(),
            );
            c.enabled = true;
            match scene {
                "idle-editor-form" => {}
                "idle-editor-off" => {
                    c.enabled = false;
                }
                "idle-editor-impact" => {
                    c.after = Editor::from("45m");
                    c.focus = 1;
                }
                "idle-editor-invalid" => {
                    c.after = Editor::from("90x");
                    c.focus = 1;
                }
                "idle-editor-saving" => {
                    c.pending = true;
                }
                "idle-editor-saved" => {
                    c.notice = Some("✓ Saved · squad idle alert".into());
                }
                "idle-editor-refused" => {
                    c.notice = Some("✗ Refused · configuration changed. Draft kept.".into());
                }
                "idle-editor-partial" => {
                    c.notice = Some("✗ Refused · After saved, Alert not changed".into());
                }
                "idle-editor-unknown" => {
                    app.view
                        .as_mut()
                        .unwrap()
                        .home
                        .as_mut()
                        .unwrap()
                        .workspace
                        .roster
                        .squads[0]
                        .observation = crate::board::workspace::OpsObservation::Unavailable;
                }
                _ => panic!("unknown idle-alert scene {scene}"),
            }
        }
    }
    app.overview_rows.borrow_mut().clear();
    app
}
pub(in crate::board) fn metadata(app: &App) -> serde_json::Value {
    app.idle_alert_editor.as_ref().map_or(serde_json::Value::Null,|c|serde_json::json!({"focus":c.focus,"choice":c.choice,"pending":c.pending,"readable":c.readable.get(),"hits":c.hits.borrow().iter().map(|(r,a)|serde_json::json!({"rect":[r.x,r.y,r.width,r.height],"control":format!("{a:?}")})).collect::<Vec<_>>()}))
}
