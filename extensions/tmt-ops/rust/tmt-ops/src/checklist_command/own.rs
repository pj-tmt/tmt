//! The agent verbs: act on the calling agent's own conversation list, with no UUIDs to supply.
//! They only choose the target and the expectations; the admitted service stays the one owner of
//! permission, revisions and attribution.
use super::*;
use crate::{me, squad::Squad};

fn refused(code: Code, message: String) -> Error {
    Error {
        code,
        message,
        current: None,
    }
}

/// The calling agent's room and identity. A given room is used as is (the service refuses a
/// caller it does not admit); otherwise the room is the one squad whose roster lists the caller.
/// None or several squads refuse and say which, rather than guess.
pub(super) fn target(core: &Core, room: Option<Id>) -> Result<(Id, Id), Error> {
    let caller = me::caller(core)
        .map_err(|e| refused(Code::Forbidden, e.to_string()))?
        .ok_or_else(|| {
            refused(
                Code::Forbidden,
                "Run this from an agent's pane: no caller is bound here.".into(),
            )
        })?;
    let member = Id::parse(&caller.me.id)?;
    if let Some(room) = room {
        return Ok((room, member));
    }
    let squads = Squad::list(core).map_err(|e| refused(Code::RoomUnavailable, e.to_string()))?;
    let mut rooms = Vec::new();
    for squad in squads {
        let roster = squad
            .roster(core)
            .map_err(|e| refused(Code::RoomUnavailable, e.to_string()))?;
        if roster.iter().any(|listed| listed.id == caller.me.id) {
            rooms.push(squad);
        }
    }
    match rooms.as_slice() {
        [squad] => Ok((Id::parse(&squad.room_id)?, member)),
        [] => Err(refused(
            Code::RoomUnavailable,
            format!("{} is on no squad roster; pass --room.", caller.me.name),
        )),
        many => Err(refused(
            Code::InputInvalid,
            format!(
                "{} is on several squads ({}); choose one with --room.",
                caller.me.name,
                many.iter()
                    .map(|squad| format!("{} {}", squad.name, squad.room_id))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}

/// `#Title (3f2a)`: how an item is named in chat, with the short id unique in its list.
fn reference(current: &Current, item: &Id) -> Option<String> {
    let title = &current
        .items
        .iter()
        .find(|view| &view.item.id == item)?
        .item
        .content
        .title;
    let short = item.short_in(current.items.iter().map(|view| &view.item.id));
    Some(format!("#{title} ({short})"))
}

/// A conflict means another writer got in between the read and the write; the list is read again
/// once. A second conflict is reported as it is.
pub(super) fn twice<T>(mut attempt: impl FnMut() -> Result<T, Error>) -> Result<T, Error> {
    match attempt() {
        Err(Error {
            code: Code::Conflict,
            ..
        }) => attempt(),
        other => other,
    }
}

fn applied(action: &str, applied: checklist::Applied, item: &Id) -> Value {
    let mut result = json!({"action":action,"current":current(&applied.current),"changed":applied.changed,
        "itemId":item.as_str(),"reference":reference(&applied.current, item)});
    if let Some(revision) = applied.item_revision {
        result["itemRevision"] = json!(revision);
    }
    result
}

pub(super) fn add(
    service: &Service,
    checklist_id: Id,
    content: ItemContent,
) -> Result<Value, Error> {
    let item = Id::parse(&crate::id::new_v4().map_err(|e| invalid(&e.to_string()))?)?;
    twice(|| {
        let listed = service.list(&Filter {
            include_archived: true,
            ..Filter::default()
        })?;
        let inventory = match listed.current.inventory_revision {
            Some(revision) => InventoryExpectation::Revision(revision),
            None => InventoryExpectation::Absent,
        };
        let done = service.apply(&Request {
            room_id: service.room_id().clone(),
            checklist_id: checklist_id.clone(),
            action: Action::Create {
                item_id: item.clone(),
                inventory,
                content: content.clone(),
                assignee: None,
            },
        })?;
        Ok(applied("add", done, &item))
    })
}

pub(super) fn done(service: &Service, checklist_id: Id, item: ItemRef) -> Result<Value, Error> {
    let item = item.resolve(service)?;
    twice(|| {
        let shown = service.show(&checklist_id, &item)?;
        let revision = shown
            .items
            .first()
            .map(|view| view.item.revision)
            .ok_or_else(|| refused(Code::NotFound, "The item was not found.".into()))?;
        let done = service.apply(&Request {
            room_id: service.room_id().clone(),
            checklist_id: checklist_id.clone(),
            action: Action::Item {
                item_id: item.clone(),
                revision,
                mutation: Mutation::Complete,
            },
        })?;
        Ok(applied("done", done, &item))
    })
}
