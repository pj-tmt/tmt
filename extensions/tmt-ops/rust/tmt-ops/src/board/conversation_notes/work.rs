//! What the lane runs for a conversation's list: one read, or one write at the revisions the
//! rows were read at.
use super::{Completed, Key, Outcome, Task, Work};
use crate::{
    checklist::{
        Applied, Code, Error, Preview, Service,
        model::{
            Action, Confirmation, Edit, Id, InventoryExpectation, ItemContent, Mutation, Request,
        },
    },
    config::Config,
    core::Core,
};

/// One change to the list. Each carries the revisions the board last read, so Core refuses it
/// as a conflict when someone else changed the item or the list since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::board) enum Write {
    Add {
        body: Option<String>,
        title: String,
        reference: Option<String>,
        inventory: Option<u64>,
    },
    Edit {
        item: Id,
        revision: u64,
        title: String,
        reference: Option<String>,
    },
    Toggle {
        item: Id,
        revision: u64,
        complete: bool,
    },
    Delete {
        item: Id,
        revision: u64,
        inventory: u64,
    },
    Reorder {
        inventory: u64,
        order: Vec<Id>,
    },
}

fn invalid(message: impl ToString) -> Error {
    Error {
        code: Code::InputInvalid,
        message: message.to_string(),
        current: None,
    }
}

/// Runs a task on the lane.
pub(in crate::board) fn complete(core: &Core, task: Task) -> Completed {
    let outcome = match task.work {
        Work::Read => Outcome::Read(read(core, &task.key).map_err(|error| error.message)),
        Work::Counts => Outcome::Counts(
            read(core, &task.key)
                .map(|preview| super::Counts::of(&preview.current.items))
                .map_err(|error| error.message),
        ),
        Work::Write(write) => Outcome::Write(apply(core, &task.key, write)),
    };
    Completed {
        key: task.key,
        epoch: task.epoch,
        outcome,
    }
}

fn service(core: &Core, key: &Key) -> Result<Service, Error> {
    let config = Config::load(core).map_err(|error| invalid(error.message))?;
    let room = Id::parse(&key.room)?;
    match key.member() {
        Some(member) => Service::for_conversation(core, &config, room, Id::parse(member)?),
        None => Service::new(core, &config, room),
    }
}

fn read(core: &Core, key: &Key) -> Result<Preview, Error> {
    service(core, key)?.preview()
}

fn apply(core: &Core, key: &Key, write: Write) -> Result<Applied, Error> {
    let room = Id::parse(&key.room)?;
    let member = Id::parse(
        key.member()
            .ok_or_else(|| invalid("Room writes belong to Checklist."))?,
    )?;
    let service = service(core, key)?;
    let checklist_id = Id::conversation_checklist(&room, &member);
    let action = match write {
        Write::Add {
            body,
            title,
            reference,
            inventory,
        } => Action::Create {
            item_id: Id::parse(&crate::id::new_v4().map_err(invalid)?)?,
            inventory: inventory
                .map_or(InventoryExpectation::Absent, InventoryExpectation::Revision),
            content: ItemContent::new(title, body, reference)?,
            assignee: None,
        },
        Write::Edit {
            item,
            revision,
            title,
            reference,
        } => Action::Item {
            item_id: item,
            revision,
            mutation: Mutation::Edit(Edit {
                title: Some(title),
                body: None,
                reference: Some(reference),
            }),
        },
        Write::Toggle {
            item,
            revision,
            complete,
        } => Action::Item {
            item_id: item,
            revision,
            mutation: if complete {
                Mutation::Complete
            } else {
                Mutation::Reopen
            },
        },
        Write::Delete {
            item,
            revision,
            inventory,
        } => Action::Item {
            item_id: item.clone(),
            revision,
            mutation: Mutation::Delete {
                inventory_revision: inventory,
                confirmation: Confirmation {
                    item_id: item,
                    item_revision: revision,
                },
            },
        },
        Write::Reorder { inventory, order } => Action::Reorder {
            inventory_revision: inventory,
            order,
        },
    };
    service.apply(&Request {
        room_id: room,
        checklist_id,
        action,
    })
}
