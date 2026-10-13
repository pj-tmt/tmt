//! Section boundaries on the squad tab: one rule between neighbours, never two.
use super::*;
use crate::board::cronboard::{TEST_NOW, test_cron, test_view};
use crate::config::{Board, Direction, Pane};
use crate::test_support::stacked_rules;
use tmt_ops::cron::ClockStatus;

const WIDTHS: [u16; 3] = [160, 100, 80];
/// Bodies from the smallest that carries every section up to a tall one, odd and even.
const HEIGHTS: [u16; 5] = [12, 13, 16, 24, 31];

fn squad_app(board: Board, notes: Notes, cron: bool) -> App {
    let mut app = paned(board, notes);
    if cron {
        app.view.as_mut().unwrap().document["squad"]["roomId"] = json!("room");
        let mut job = test_view("someone", "job", Some(TEST_NOW + 3_600_000));
        job.job.squad = "product".into();
        job.job.room_id = "room".into();
        app.cron
            .replace(Ok(test_cron(vec![job], ClockStatus::NoClock)));
    }
    app
}

/// What the notes pane can hold: a notebook, or nothing yet.
fn notebooks() -> [Notes; 2] {
    [Notes::Text("a note".into()), Notes::Missing]
}

fn assert_single_rules(app: &mut App, what: &str) {
    for width in WIDTHS {
        app.set_body_width(width);
        for height in HEIGHTS {
            let lines = draw(app, width, height);
            if let Some(at) = stacked_rules(&lines) {
                panic!(
                    "two rules in a row at line {at}: {what} at {width}x{height}\n{}",
                    lines.join("\n")
                );
            }
        }
    }
}

#[test]
fn no_pane_order_stacks_two_rules_at_a_boundary() {
    let others = [Pane::Rows, Pane::Notes, Pane::Detail, Pane::Replies];
    let mut orders: Vec<Vec<Pane>> = Vec::new();
    for first in others {
        for second in others.into_iter().filter(|pane| *pane != first) {
            orders.push(vec![first, second]);
            for third in others
                .into_iter()
                .filter(|pane| *pane != first && *pane != second)
            {
                orders.push(vec![first, second, third]);
            }
        }
    }
    orders.retain(|panes| panes.contains(&Pane::Rows));
    for panes in orders {
        for direction in [Direction::TopBottom, Direction::LeftRight] {
            for members in [true, false] {
                for cron in [true, false] {
                    let notes = if panes.contains(&Pane::Notes) {
                        notebooks().to_vec()
                    } else {
                        vec![Notes::NotShown]
                    };
                    for notes in notes {
                        let sizes = vec![100 / panes.len() as u16; panes.len()];
                        let mut board = split(direction, panes.clone(), sizes);
                        board.members = members;
                        let mut app = squad_app(board, notes, cron);
                        let what = format!(
                            "{panes:?} {direction:?} members={members} cron={cron} notes={:?}",
                            app.view.as_ref().unwrap().notes
                        );
                        assert_single_rules(&mut app, &what);
                    }
                }
            }
        }
    }
}

#[test]
fn no_factory_view_stacks_two_rules_at_a_boundary() {
    let path = std::env::temp_dir().join(format!("squad-rules-{}.toml", std::process::id()));
    for workflow in ["crew", "team"] {
        for name in crate::view::ViewName::ALL {
            std::fs::write(
                &path,
                format!(
                    "[squad.product]\nlayout = '{workflow}'\n[squad.product.board]\nview = '{}'\n",
                    name.name()
                ),
            )
            .unwrap();
            let board = crate::config::Config::read(path.clone())
                .unwrap()
                .board("product")
                .unwrap();
            for cron in [true, false] {
                for notes in notebooks() {
                    let mut app = squad_app(board.clone(), notes, cron);
                    let what = format!(
                        "{} / {workflow} cron={cron} notes={:?}",
                        name.name(),
                        app.view.as_ref().unwrap().notes
                    );
                    assert_single_rules(&mut app, &what);
                }
            }
        }
    }
    std::fs::remove_file(path).unwrap();
}

/// The default board and the member list, with the lead's notes present and absent,
/// beside the jobs half.
fn snapshots() -> Value {
    let path = std::env::temp_dir().join(format!(
        "squad-rules-shots-{}-{:?}.toml",
        std::process::id(),
        std::thread::current().id()
    ));
    let mut frames = Vec::new();
    for name in [crate::view::ViewName::Team, crate::view::ViewName::Members] {
        std::fs::write(
            &path,
            format!(
                "[squad.product]\nlayout = 'team'\n[squad.product.board]\nview = '{}'\n",
                name.name()
            ),
        )
        .unwrap();
        let board = crate::config::Config::read(path.clone())
            .unwrap()
            .board("product")
            .unwrap();
        for (notes, present) in [
            (Notes::Text("a note".into()), true),
            (Notes::Missing, false),
        ] {
            for width in WIDTHS {
                let mut app = squad_app(board.clone(), notes.clone(), true);
                app.set_body_width(width);
                frames.push(json!({
                    "view": name.name(), "notes": present, "width": width, "height": 24,
                    "lines": draw(&app, width, 24),
                }));
            }
        }
    }
    std::fs::remove_file(path).unwrap();
    json!(frames)
}

#[test]
fn squad_tab_boundaries_snapshots_with_and_without_notes() {
    assert_eq!(
        snapshots(),
        serde_json::from_str::<Value>(include_str!("rules_snapshots.json")).unwrap()
    );
}

#[test]
#[ignore = "explicit captures of the squad tab's section boundaries"]
fn record_squad_tab_boundaries_snapshots() {
    std::fs::write(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/board/view/tests/rules_snapshots.json"
        ),
        serde_json::to_string_pretty(&snapshots()).unwrap() + "\n",
    )
    .unwrap();
}

/// Task-owned evidence only; frozen fixtures are never written by this capture.
#[test]
#[ignore = "explicit squad-heading cell/style/hit evidence capture"]
fn capture_squad_heading_packet() {
    let output = std::env::var("TMT_SQUAD_HEADING_OUTPUT").expect("task-owned output path");
    let path = std::env::temp_dir().join(format!("squad-heading-{}.toml", std::process::id()));
    let mut frames = Vec::new();
    crate::status::with_now_ms(TEST_NOW as u64, || {
        for name in [crate::view::ViewName::Team, crate::view::ViewName::Members] {
            std::fs::write(
                &path,
                format!(
                    "[squad.product]\nlayout = 'team'\n[squad.product.board]\nview = '{}'\n",
                    name.name()
                ),
            )
            .unwrap();
            let board = crate::config::Config::read(path.clone())
                .unwrap()
                .board("product")
                .unwrap();
            for (notes, present) in [
                (Notes::Text("a note".into()), true),
                (Notes::Missing, false),
            ] {
                for cron in [false, true] {
                    for width in WIDTHS {
                        for (base, depth) in [
                            ("tmt", tmt_cli_style::Depth::TrueColor),
                            ("tmt-light", tmt_cli_style::Depth::TrueColor),
                            ("NO_COLOR", tmt_cli_style::Depth::None),
                        ] {
                            for focus in [0, 1] {
                                for long_lead in [false, true] {
                                    let mut app = squad_app(board.clone(), notes.clone(), cron);
                                    app.focus = focus;
                                    let view = app.view.as_mut().unwrap();
                                    view.look = crate::look::Look {
                                        theme: tmt_cli_style::Theme::new(
                                            tmt_cli_style::Base::parse(if base == "NO_COLOR" {
                                                "tmt"
                                            } else {
                                                base
                                            })
                                            .unwrap(),
                                        ),
                                        depth,
                                    };
                                    if long_lead {
                                        view.document["squad"]["lead"]["name"] = json!(
                                            "a-very-long-lead-name-that-exercises-heading-clipping"
                                        );
                                        view.document["squad"]["notesStaleness"] =
                                            json!({"state":"stale", "ageMs":7_200_000});
                                    }
                                    let frame = super::parity::capture(&mut app, width);
                                    frames.push(json!({"view":name.name(), "notes":present, "cron":cron,
                                        "width":width, "base":base, "focus":focus, "long_lead":long_lead,
                                        "frame":frame, "titles":format!("{:?}",app.title_hits.borrow())}));
                                }
                            }
                        }
                    }
                }
            }
        }
    });
    std::fs::remove_file(path).unwrap();
    std::fs::write(output, serde_json::to_vec(&frames).unwrap()).unwrap();
}

#[test]
#[ignore = "read-only squad-rule snapshot inspection before fixture approval"]
fn dump_squad_rule_snapshots() {
    let path = std::env::var("SQUAD_RULES_SNAPSHOTS_OUT").expect("task-owned output path");
    std::fs::write(
        path,
        serde_json::to_string_pretty(&snapshots()).unwrap() + "\n",
    )
    .unwrap();
}
