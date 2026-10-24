//! Global todo observations through the real board painter.
pub(in crate::board) use super::rooms::{ROOM_SCENES, capture_case as room_case};
use super::*;
use crate::board::{app::App, workspace::OpsTarget};

pub(in crate::board) const SCENES: &[&str] = &[
    "todo-pending",
    "todo-known",
    "todo-zero",
    "todo-partial",
    "todo-unavailable",
    "todo-done",
];

pub(in crate::board) fn case(scene: &str, look: crate::look::Look) -> App {
    let mut app = crate::board::app::workspace::tests::populated_board(1, 2);
    app.local_zone = Some(jiff::tz::TimeZone::get("Asia/Tokyo").unwrap());
    let configuration = app.view.as_mut().unwrap();
    configuration.look = look;
    configuration
        .home
        .as_mut()
        .unwrap()
        .workspace
        .squad_looks
        .clear();
    let keys: Vec<_> = app
        .workspace_snapshot()
        .unwrap()
        .rows
        .keys()
        .filter_map(Key::list)
        .collect();
    let index = app
        .workspace_rows()
        .iter()
        .position(|row| matches!(row.target, OpsTarget::Member { .. }))
        .unwrap();
    app.select(index);
    // A pending request must not replace the actual activity observation.
    for member in app
        .view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .roster
        .members
        .values_mut()
    {
        member.waiting = true;
        member.activity.phase = crate::board::activity::Phase::Working;
    }
    app.workspace_reconcile();
    app.notes = State::reading();
    app.notes.reconcile(keys.clone().into_iter());
    let now = Instant::now();
    if scene != "todo-pending" {
        let mut member_ordinal = 0;
        for _ in 0..keys.len() {
            let task = app.notes.offer_background(None, now).unwrap();
            let ordinal = member_ordinal;
            if task.key.member().is_some() {
                member_ordinal += 1;
            }
            let result = match scene {
                "todo-unavailable" => Err("Conversation Notes unavailable.".into()),
                _ if task.key.member().is_none() => Ok(Counts::default()),
                "todo-partial" if ordinal == 1 => Err("Conversation Notes unavailable.".into()),
                "todo-zero" => Ok(Counts::default()),
                _ => Ok(Counts {
                    todo: ordinal + 1,
                    ..Counts::default()
                }),
            };
            app.notes.finished(
                Completed {
                    key: task.key,
                    epoch: task.epoch,
                    outcome: Outcome::Counts(result),
                },
                now,
            );
        }
    }
    if scene == "todo-done" {
        let selected = app.conversation_checklist_key().unwrap();
        let task = app.notes.offer(Some(&selected), now).unwrap();
        app.notes.finished(
            reading(
                &task,
                Ok(Counts {
                    todo: 1,
                    ..Counts::default()
                }),
            ),
            now,
        );
        let item = view(1, None, Completion::Open, false).item.id;
        let task = app
            .notes
            .write(
                &selected,
                Write::Toggle {
                    item: item.clone(),
                    revision: 1,
                    complete: true,
                },
            )
            .unwrap();
        app.notes.finished(
            Completed {
                key: selected,
                epoch: task.epoch,
                outcome: Outcome::Write(Ok(crate::checklist::Applied {
                    current: preview(vec![view(1, None, Completion::Complete, false)]).current,
                    item_id: Some(item),
                    item_revision: Some(2),
                    changed: true,
                })),
            },
            now,
        );
    }
    app
}
