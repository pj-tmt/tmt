//! Explicit local evidence: real public Core reads and exact renderer cells.
use super::*;
use crate::board::recent_responses::{Item, Observation};
use crate::board::workspace::OpsTarget;

fn for_identity(identity: &str) -> App {
    let mut app = workspace_fixture();
    let old = crate::board::app::workspace::tests::MEMBER;
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let mut member = snapshot.roster.members.remove(old).unwrap();
    member.id = identity.into();
    snapshot.roster.members.insert(identity.into(), member);
    for squad in &mut snapshot.roster.squads {
        for id in &mut squad.members {
            if id == old {
                *id = identity.into();
            }
        }
        if let Some(state) = squad.member_states.remove(old) {
            squad.member_states.insert(identity.into(), state);
        }
    }
    let old_target = app.workspace.selected.clone().unwrap();
    let OpsTarget::Member { room_id, .. } = old_target.clone() else {
        panic!("member fixture")
    };
    let next = OpsTarget::Member {
        room_id,
        identity_id: identity.into(),
    };
    let mut row = snapshot.rows.remove(&old_target).unwrap();
    row["id"] = json!(identity);
    snapshot.rows.insert(next.clone(), row);
    for row in snapshot.rows.values_mut() {
        if let Some(ids) = row["memberIds"].as_array_mut() {
            for id in ids {
                if id.as_str() == Some(old) {
                    *id = json!(identity);
                }
            }
        }
    }
    app.view.as_mut().unwrap().replies[0]["recipientId"] = json!(identity);
    app.view.as_mut().unwrap().replies[0]["response"] =
        json!("Retained exchange remains reachable.");
    app.view.as_mut().unwrap().replies[0]["submittedAtMs"] = json!(1000);
    app.workspace.selected = Some(next);
    app.workspace_reconcile();
    app
}

fn install(app: &mut App, observation: Observation) {
    let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    let key = app.pane_output_read().expect("visible exact selected UUID");
    app.apply_pane_output(key, observation);
}

#[test]
#[ignore = "requires an explicitly isolated native Core and seeded UUID"]
fn native_recent_output_observation_and_display() {
    let executable =
        std::env::var_os("TMT_RECENT_NATIVE_CORE").expect("fixture-selected native Core");
    assert!(std::path::Path::new(&executable).is_absolute());
    let identity = std::env::var("TMT_RECENT_NATIVE_ID").expect("fixture identity UUID");
    let observation =
        crate::board::recent_responses::read(&crate::core::Core::at(executable.into()), &identity);
    let Observation::Observed(items) = &observation else {
        panic!("native Core observation: {observation:?}")
    };
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].text, "same native final");
    assert_eq!(items[1].text, "same native final");
    let suffix = "\nNATIVE pane final\r\nlast line\n";
    assert_eq!(
        items[2].text,
        format!("{}{suffix}", "x".repeat(8192 - suffix.len()))
    );
    let baseline = std::env::var("TMT_RECENT_NATIVE_TIME")
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert_eq!(
        items.iter().map(|item| item.ended_ms).collect::<Vec<_>>(),
        [baseline, baseline, baseline - 1]
    );
    assert!(items[2].truncated);
    // All source acquisition is done. Drawing/selection cannot invoke commands.
    let mut app = for_identity(&identity);
    let _no_commands = crate::runner::forbid_commands();
    install(&mut app, observation);
    let screen = draw(&app, 100, 36);
    assert!(screen.contains("NATIVE pane final"), "{screen}");
    assert!(screen.contains("… truncated"), "{screen}");
    app.scroll_pane(Pane::Replies, Step::Top);
    let top = draw(&app, 100, 36);
    assert!(top.contains("pane output"), "{top}");
    assert_eq!(
        app.conversation.borrow().reply_hits.len(),
        1,
        "only the real exchange has a source hit"
    );
    assert!(
        app.conversation
            .borrow()
            .reply_hits
            .iter()
            .all(|hit| hit.key.request == "req_44444444-4444-4444-8444-444444444444")
    );
}

#[test]
#[ignore = "explicit recent-output look packet, never writes frozen files"]
fn capture_recent_pane_output_packet() {
    let output = std::env::var_os("TMT_RECENT_OUTPUT_PACKET").expect("task-owned packet path");
    let _no_commands = crate::runner::forbid_commands();
    crate::status::with_now_ms(1_791_600_000_000, || {
        let mut frames = vec![];
        for (base, depth) in [
            ("tmt", tmt_cli_style::Depth::TrueColor),
            ("tmt-light", tmt_cli_style::Depth::TrueColor),
            ("tmt", tmt_cli_style::Depth::None),
        ] {
            for width in [160, 100, 60] {
                for scene in ["mixed", "unavailable", "long-top", "long-bottom"] {
                    let mut app = for_identity(crate::board::app::workspace::tests::MEMBER);
                    app.view.as_mut().unwrap().look = Look {
                        theme: tmt_cli_style::Theme::new(tmt_cli_style::Base::parse(base).unwrap()),
                        depth,
                    };
                    let observation = if scene == "unavailable" {
                        Observation::Unavailable
                    } else {
                        Observation::Observed((0..if scene.starts_with("long") {10}else{3}).map(|index|Item {
                            text:if scene.starts_with("long") {format!("Capture {index}\n{}\nEnd capture {index}","Line in bounded pane capture\n".repeat(100))} else {format!("Pane capture {index}\n**Marked** text remains pane output.\nNot a submitted reply.")},
                            ended_ms:1_791_599_900_000+index,truncated:index==2,
                        }).collect())
                    };
                    install(&mut app, observation);
                    app.key(ratatui::crossterm::event::KeyEvent::new(
                        ratatui::crossterm::event::KeyCode::Enter,
                        ratatui::crossterm::event::KeyModifiers::NONE,
                    ));
                    assert!(app.input.is_some());
                    let mut terminal = Terminal::new(TestBackend::new(width, 36)).unwrap();
                    let mut paint = |app: &mut App| {
                        terminal
                            .draw(|frame| {
                                crate::board::view::prepare(app, frame.area());
                                crate::board::view::render(frame, app);
                            })
                            .unwrap();
                    };
                    paint(&mut app);
                    if scene == "long-top" {
                        app.scroll_pane(Pane::Replies, Step::Top);
                        paint(&mut app);
                    }
                    let buffer = terminal.backend().buffer();
                    frames.push(json!({"scene":scene,"width":width,"height":36,"base":base,"depth":format!("{depth:?}"),
                        "lines":buffer.content.chunks(usize::from(width)).map(|row|row.iter().map(|cell|cell.symbol()).collect::<String>()).collect::<Vec<_>>(),
                        "cells":buffer.content.iter().map(|cell|json!({"symbol":cell.symbol(),"fg":format!("{:?}",cell.fg),"bg":format!("{:?}",cell.bg),"modifier":format!("{:?}",cell.modifier)})).collect::<Vec<_>>(),
                        "inputBand":format!("{:?}",app.input_band.get()),"requestHits":app.conversation.borrow().reply_hits.len()}));
                }
            }
        }
        std::fs::write(output, serde_json::to_vec_pretty(&frames).unwrap()).unwrap();
    });
}
