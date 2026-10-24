//! Acquired event fixtures through the real conversation painter; no live Core or tmux access.
use super::*;
use crate::checklist::events::{Gap, Logged, Page};

pub(in crate::board) const SCENES: &[&str] = &[
    "checklist-activity",
    "checklist-activity-lifecycle",
    "checklist-activity-legacy",
    "checklist-activity-pruned",
    "checklist-activity-failed",
    "checklist-activity-unresolved",
];

pub(in crate::board) fn capture_case(
    scene: &str,
    look: crate::look::Look,
) -> crate::board::app::App {
    let mut app = super::capture_case("notes-counts", look);
    app.focus = 1;
    let key = app.conversation_checklist_key().unwrap();
    let now = Instant::now() + FRESH;
    let task = app.notes.offer(Some(&key), now).unwrap();
    let mut preview = app.notes.preview(&key).unwrap().clone();
    preview.current.items[0].item.content.title = "Fix the startup test".into();
    preview.current.items[1].item.content.title = "Review the lifecycle cleanup".into();
    let actions: &[&str] = if scene == "checklist-activity-lifecycle" {
        &[
            "reopen", "assign", "unassign", "archive", "restore", "delete",
        ]
    } else {
        &["add", "complete", "edit", "reorder"]
    };
    let events = actions
        .iter()
        .enumerate()
        .map(|(index, action)| Logged {
            id: index as u64 + 1,
            room_id: key.room.clone(),
            member_id: key.member().map(str::to_owned),
            checklist_id: preview
                .current
                .checklist_id
                .as_ref()
                .unwrap()
                .as_str()
                .into(),
            item_id: if *action == "reorder" {
                None
            } else if *action == "delete" {
                Some("88888888-8888-4888-8888-888888888888".into())
            } else {
                Some(preview.current.items[index % 2].item.id.as_str().into())
            },
            actor_id: if *action == "delete" {
                STRANGER.into()
            } else {
                ALICE.into()
            },
            at_ms: crate::status::now_ms() - 180_000 + index as u64 * 30_000,
            action: (*action).into(),
            revision: 2,
        })
        .collect();
    let page = Page {
        events,
        next_cursor: actions.len() as u64,
        gap: match scene {
            "checklist-activity-legacy" => Some(Gap::Legacy),
            "checklist-activity-pruned" => Some(Gap::Pruned),
            _ => None,
        },
        more: scene == "checklist-activity-pruned",
    };
    app.notes.finished(
        Completed {
            key: key.clone(),
            epoch: task.epoch,
            outcome: Outcome::Read {
                preview: Ok(preview.clone()),
                activity: activity::Read {
                    observed: true,
                    user: (scene != "checklist-activity-unresolved").then(|| ACTOR.into()),
                    pages: Ok(vec![page]),
                },
            },
        },
        now,
    );
    if scene == "checklist-activity-failed" {
        let later = now + FRESH;
        let task = app.notes.offer(Some(&key), later).unwrap();
        app.notes.finished(
            Completed {
                key,
                epoch: task.epoch,
                outcome: Outcome::Read {
                    preview: Ok(preview),
                    activity: activity::Read {
                        observed: true,
                        user: Some(ACTOR.into()),
                        pages: Err("read failed".into()),
                    },
                },
            },
            later,
        );
    }
    app
}
