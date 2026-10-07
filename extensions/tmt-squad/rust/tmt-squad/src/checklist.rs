//! Admitted room-owned checklist service. Commands and board actions are not exposed yet.

pub mod model;
mod store;

use crate::{
    config::Config,
    core::{Core, SquadError},
    me,
    squad::{Member, Squad},
};
use model::{Assignee, Completion, Document, Id, Item, Request, Tombstone};
use serde_json::json;
use std::{fmt, path::PathBuf};
use store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    InputInvalid,
    Forbidden,
    RoomUnavailable,
    AssigneeUnavailable,
    NotFound,
    Deleted,
    StateInvalid,
    Conflict,
    StorageError,
    OutcomeUnknown,
}
impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InputInvalid => "CHECKLIST_INPUT_INVALID",
            Self::Forbidden => "CHECKLIST_FORBIDDEN",
            Self::RoomUnavailable => "CHECKLIST_ROOM_UNAVAILABLE",
            Self::AssigneeUnavailable => "CHECKLIST_ASSIGNEE_UNAVAILABLE",
            Self::NotFound => "CHECKLIST_NOT_FOUND",
            Self::Deleted => "CHECKLIST_DELETED",
            Self::StateInvalid => "CHECKLIST_STATE_INVALID",
            Self::Conflict => "CHECKLIST_CONFLICT",
            Self::StorageError => "CHECKLIST_STORAGE_ERROR",
            Self::OutcomeUnknown => "CHECKLIST_OUTCOME_UNKNOWN",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub code: Code,
    pub message: String,
    /// Present only after successful current access and operation admission.
    pub current: Option<Box<Current>>,
}
impl Error {
    fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            current: None,
        }
    }
    fn storage(error: impl fmt::Display) -> Self {
        Self::new(Code::StorageError, error.to_string())
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code.as_str())
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomView {
    pub id: Id,
    pub name: Option<String>,
    pub available: bool,
    pub manager: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemView {
    pub item: Item,
    pub assignee_label: Option<String>,
    pub assignee_available: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Current {
    pub room: RoomView,
    pub checklist_id: Option<Id>,
    pub inventory_revision: Option<u64>,
    pub items: Vec<ItemView>,
    pub deletion: Option<Tombstone>,
}
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub include_archived: bool,
    pub completion: Option<Completion>,
    pub assignee: Option<Id>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct List {
    pub current: Current,
    /// All nondeleted items, before archive/completion/member filtering.
    pub total_count: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub current: Current,
    pub item_id: Option<Id>,
    pub item_revision: Option<u64>,
    pub changed: bool,
}

struct Admission {
    room: RoomView,
    roster: Vec<Member>,
}

/// Only this local invocation constructor captures the public caller. A request
/// has no actor field; arbitrary client-supplied identity strings grant nothing.
pub struct Service {
    core: Core,
    config_path: PathBuf,
    actor_id: Id,
    store: Store,
}
impl Service {
    pub fn new(core: &Core, config: &Config, room_id: Id) -> Result<Self, Error> {
        let actor = match me::caller(core).map_err(|e| port(Code::Forbidden, e))? {
            Some(caller) => caller.me,
            None => recorded(core, config)?.ok_or_else(|| {
                Error::new(
                    Code::Forbidden,
                    "No admitted caller or recorded active user.",
                )
            })?,
        };
        let root = core
            .api("storage.root", json!({}))
            .map_err(Error::storage)?;
        let root = root["dataRoot"]
            .as_str()
            .map(PathBuf::from)
            .ok_or_else(|| Error::storage("storage.root returned no dataRoot."))?;
        Ok(Self {
            core: core.clone(),
            config_path: config.path().into(),
            actor_id: Id::parse(&actor.id)?,
            store: Store::new(&root, room_id)?,
        })
    }

    fn admit(
        &self,
        write: bool,
        manager: bool,
        assignment: Option<&Id>,
    ) -> Result<(Admission, Option<Assignee>), Error> {
        let config =
            Config::read(self.config_path.clone()).map_err(|e| port(Code::Forbidden, e))?;
        active(&self.core, &self.actor_id, Code::Forbidden)?;
        let user =
            recorded(&self.core, &config)?.is_some_and(|user| user.id == self.actor_id.as_str());
        let refs = self
            .core
            .api(
                "references.resolve",
                json!({"roomIds":[self.store.room_id().as_str()]}),
            )
            .map_err(|e| port(Code::RoomUnavailable, e))?;
        let room = &refs["rooms"][0];
        if room["id"] != self.store.room_id().as_str()
            || !room["found"].is_boolean()
            || (room["found"] == true && !room["retired"].is_boolean())
        {
            return Err(Error::new(
                Code::RoomUnavailable,
                "Core returned an incomplete room reference.",
            ));
        }
        let available = room["found"] == true && room["retired"] == false;
        if !available {
            if write {
                return Err(Error::new(
                    Code::RoomUnavailable,
                    "The exact room is absent or retired.",
                ));
            }
            if !user {
                return Err(Error::new(
                    Code::Forbidden,
                    "Only the recorded active user may inspect an unavailable room by UUID.",
                ));
            }
            return Ok((
                Admission {
                    room: RoomView {
                        id: self.store.room_id().clone(),
                        name: None,
                        available: false,
                        manager: true,
                    },
                    roster: Vec::new(),
                },
                None,
            ));
        }
        let shown = self
            .core
            .json(&["room", "show", self.store.room_id().as_str()])
            .map_err(|e| port(Code::RoomUnavailable, e))?;
        let name = shown["room"]["name"]
            .as_str()
            .filter(|_| shown["room"]["id"] == self.store.room_id().as_str())
            .and_then(|name| name.strip_prefix("squad-"))
            .filter(|name| crate::squad::valid_name(name))
            .ok_or_else(|| {
                Error::new(
                    Code::RoomUnavailable,
                    "The exact room is not an active Squad room.",
                )
            })?;
        let squad = Squad {
            name: name.into(),
            room_id: self.store.room_id().as_str().into(),
        };
        let roster = squad
            .roster(&self.core)
            .map_err(|e| port(Code::RoomUnavailable, e))?;
        let member = roster
            .iter()
            .find(|member| member.id == self.actor_id.as_str());
        let is_manager = user || member.is_some_and(Member::is_lead);
        if (!user && member.is_none()) || (manager && !is_manager) {
            return Err(Error::new(
                Code::Forbidden,
                "The actor lacks current room or operation permission.",
            ));
        }
        let assignee = if let Some(id) = assignment {
            let label = active(&self.core, id, Code::AssigneeUnavailable)?;
            if !roster.iter().any(|member| member.id == id.as_str()) {
                return Err(Error::new(
                    Code::AssigneeUnavailable,
                    "The chosen assignee is not an active member of this room.",
                ));
            }
            Some(Assignee {
                id: id.clone(),
                label,
            })
        } else {
            None
        };
        Ok((
            Admission {
                room: RoomView {
                    id: self.store.room_id().clone(),
                    name: Some(name.into()),
                    available: true,
                    manager: is_manager,
                },
                roster,
            },
            assignee,
        ))
    }

    pub fn list(&self, filter: &Filter) -> Result<List, Error> {
        let (admission, _) = self.admit(false, false, None)?;
        let document = self.store.read()?;
        let mut current = project(&admission, document.as_ref(), None);
        let total_count = current.items.len();
        current.items.retain(|view| {
            (filter.include_archived || !view.item.archived)
                && filter
                    .completion
                    .is_none_or(|completion| view.item.completion == completion)
                && filter
                    .assignee
                    .as_ref()
                    .is_none_or(|id| view.item.assignee.as_ref().is_some_and(|a| &a.id == id))
        });
        Ok(List {
            current,
            total_count,
        })
    }

    pub fn show(&self, checklist_id: &Id, item_id: &Id) -> Result<Current, Error> {
        let (admission, _) = self.admit(false, false, None)?;
        let document = self.store.read()?;
        let current = project(&admission, document.as_ref(), Some(item_id));
        let document = document
            .as_ref()
            .filter(|d| &d.checklist_id == checklist_id)
            .ok_or_else(|| Error::new(Code::NotFound, "The exact checklist was not found."))?;
        if document.deleted.iter().any(|d| &d.item_id == item_id) {
            return Err(Error {
                current: Some(Box::new(current)),
                ..Error::new(Code::Deleted, "The exact item is deleted.")
            });
        }
        if !document.items.iter().any(|i| &i.id == item_id) {
            return Err(Error::new(Code::NotFound, "The exact item was not found."));
        }
        Ok(Current {
            items: current
                .items
                .into_iter()
                .filter(|i| &i.item.id == item_id)
                .collect(),
            ..current
        })
    }

    pub fn apply(&self, request: &Request) -> Result<Applied, Error> {
        request.validate()?;
        if &request.room_id != self.store.room_id() {
            return Err(Error::new(
                Code::InputInvalid,
                "Request room differs from the admitted service room.",
            ));
        }
        // Refuse unauthorized operations before creating any storage infrastructure.
        self.admit(true, request.action.manager(), request.action.assignment())?;
        self.store.update(
            |document| {
                let (admission, assignee) =
                    self.admit(true, request.action.manager(), request.action.assignment())?;
                let before = document.clone();
                let mutation = model::apply(document, request, assignee);
                let outcome = match mutation {
                    Ok(outcome) => outcome,
                    Err(mut error) => {
                        if matches!(error.code, Code::Conflict | Code::Deleted) {
                            error.current = Some(Box::new(project(
                                &admission,
                                before.as_ref(),
                                request.action.item_id(),
                            )));
                        }
                        return Err(error);
                    }
                };
                // These checks are repeated after the candidate is complete, immediately
                // before store staging/publication. Core and files are not one transaction.
                let (admission, assignee) =
                    self.admit(true, request.action.manager(), request.action.assignment())?;
                if let Some(assignee) = assignee.filter(|_| outcome.changed) {
                    let item = document
                        .as_mut()
                        .and_then(|d| {
                            d.items
                                .iter_mut()
                                .find(|i| Some(&i.id) == request.action.item_id())
                        })
                        .expect("admitted assignment has a live target");
                    item.assignee = Some(assignee);
                }
                Ok(Applied {
                    current: project(&admission, document.as_ref(), request.action.item_id()),
                    item_id: request.action.item_id().cloned(),
                    item_revision: outcome.item_revision,
                    changed: outcome.changed,
                })
            },
            || {
                self.admit(true, request.action.manager(), request.action.assignment())
                    .map(|_| ())
            },
        )
    }
}

fn port(code: Code, error: SquadError) -> Error {
    Error::new(code, error.to_string())
}
fn recorded(core: &Core, config: &Config) -> Result<Option<me::Me>, Error> {
    // me::current's legacy name fallback must never adopt a same-name successor
    // when a recorded UUID has retired.
    if let Some(id) = config.me_id().map_err(|e| port(Code::Forbidden, e))? {
        let id = Id::parse(id).map_err(|e| Error::new(Code::Forbidden, e.message))?;
        let refs = core
            .api("references.resolve", json!({"identityIds":[id.as_str()]}))
            .map_err(|e| port(Code::Forbidden, e))?;
        let identity = &refs["identities"][0];
        if identity["id"] != id.as_str()
            || !identity["found"].is_boolean()
            || (identity["found"] == true && !identity["retired"].is_boolean())
        {
            return Err(Error::new(
                Code::Forbidden,
                "Core returned an incomplete user reference.",
            ));
        }
        if identity["found"] != true || identity["retired"] != false {
            return Ok(None);
        }
    }
    let current = me::current(core, config).map_err(|e| port(Code::Forbidden, e))?;
    if let Some(expected) = config.me_id().map_err(|e| port(Code::Forbidden, e))?
        && current
            .as_ref()
            .is_some_and(|user| !user.id.eq_ignore_ascii_case(expected))
    {
        return Err(Error::new(
            Code::Forbidden,
            "The recorded user UUID changed during admission.",
        ));
    }
    Ok(current)
}
fn active(core: &Core, id: &Id, code: Code) -> Result<String, Error> {
    let refs = core
        .api("references.resolve", json!({"identityIds":[id.as_str()]}))
        .map_err(|e| port(code, e))?;
    let identity = &refs["identities"][0];
    if identity["id"] != id.as_str() || identity["found"] != true || identity["retired"] != false {
        return Err(Error::new(code, "The exact identity is unavailable."));
    }
    identity["name"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::new(code, "Core returned no identity name."))
}
fn project(admission: &Admission, document: Option<&Document>, item_id: Option<&Id>) -> Current {
    Current {
        room: admission.room.clone(),
        checklist_id: document.map(|d| d.checklist_id.clone()),
        inventory_revision: document.map(|d| d.inventory_revision),
        items: document
            .into_iter()
            .flat_map(|d| &d.items)
            .map(|item| {
                let member = item
                    .assignee
                    .as_ref()
                    .and_then(|a| admission.roster.iter().find(|m| m.id == a.id.as_str()));
                ItemView {
                    item: item.clone(),
                    assignee_label: member
                        .map(|m| m.name.clone())
                        .or_else(|| item.assignee.as_ref().map(|a| a.label.clone())),
                    assignee_available: member.is_some(),
                }
            })
            .collect(),
        deletion: document
            .and_then(|d| d.deleted.iter().find(|d| Some(&d.item_id) == item_id))
            .cloned(),
    }
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
