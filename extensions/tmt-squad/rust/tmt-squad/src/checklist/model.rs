//! Checklist values and transitions; no caller, filesystem or dispatch authority.

use super::{Code, Error};
use crate::effects;
use std::collections::BTreeSet;

/// Canonical UUID text is the sole storage and reference identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Id(String);

impl Id {
    pub fn parse(text: &str) -> Result<Self, Error> {
        if text.len() != 36
            || !text.bytes().enumerate().all(|(i, byte)| {
                if matches!(i, 8 | 13 | 18 | 23) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            })
        {
            return Err(Error::new(
                Code::InputInvalid,
                "Expected a hyphenated UUID.",
            ));
        }
        Ok(Self(text.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemContent {
    pub title: String,
    pub body: Option<String>,
    pub reference: Option<String>,
}

impl ItemContent {
    pub fn new(
        title: String,
        body: Option<String>,
        reference: Option<String>,
    ) -> Result<Self, Error> {
        let content = Self {
            title,
            body: body.filter(|text| !text.is_empty()),
            reference,
        };
        content.validate()?;
        Ok(content)
    }

    pub(super) fn validate(&self) -> Result<(), Error> {
        if self.title.len() > 1024
            || self.title.trim().is_empty()
            || self.title.chars().any(char::is_control)
        {
            return Err(Error::new(
                Code::InputInvalid,
                "Title must be nonblank, without controls, and at most 1024 UTF-8 bytes.",
            ));
        }
        if self.body.as_ref().is_some_and(|body| {
            body.len() > 65536
                || body
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        }) {
            return Err(Error::new(
                Code::InputInvalid,
                "Body must be at most 65536 UTF-8 bytes; only line breaks and tab controls are allowed.",
            ));
        }
        if let Some(reference) = &self.reference {
            if reference.len() > 4096 {
                return Err(Error::new(
                    Code::InputInvalid,
                    "Reference exceeds 4096 UTF-8 bytes.",
                ));
            }
            effects::web_link(reference)
                .map_err(|message| Error::new(Code::InputInvalid, message))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignee {
    pub id: Id,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    Open,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: Id,
    pub revision: u64,
    pub content: ItemContent,
    pub assignee: Option<Assignee>,
    pub completion: Completion,
    pub archived: bool,
}

/// Content-free identity protection, never an operation receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tombstone {
    pub item_id: Id,
    pub deletion_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub room_id: Id,
    pub checklist_id: Id,
    pub inventory_revision: u64,
    /// Full authored order, including archived items.
    pub items: Vec<Item>,
    pub deleted: Vec<Tombstone>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InventoryExpectation {
    Absent,
    Revision(u64),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Edit {
    pub title: Option<String>,
    /// None means unchanged; Some(None) explicitly clears the field.
    pub body: Option<Option<String>>,
    pub reference: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutation {
    Edit(Edit),
    Assign(Id),
    Unassign,
    Complete,
    Reopen,
    Archive,
    Restore,
    Delete {
        inventory_revision: u64,
        confirmation: Confirmation,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmation {
    pub item_id: Id,
    pub item_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Create {
        item_id: Id,
        inventory: InventoryExpectation,
        content: ItemContent,
        assignee: Option<Id>,
    },
    Item {
        item_id: Id,
        revision: u64,
        mutation: Mutation,
    },
    Reorder {
        inventory_revision: u64,
        order: Vec<Id>,
    },
}
impl Action {
    pub(super) fn manager(&self) -> bool {
        match self {
            Self::Create { assignee, .. } => assignee.is_some(),
            Self::Item { mutation, .. } => !matches!(
                mutation,
                Mutation::Edit(_) | Mutation::Complete | Mutation::Reopen
            ),
            Self::Reorder { .. } => true,
        }
    }
    pub(super) fn assignment(&self) -> Option<&Id> {
        match self {
            Self::Create { assignee, .. } => assignee.as_ref(),
            Self::Item {
                mutation: Mutation::Assign(id),
                ..
            } => Some(id),
            _ => None,
        }
    }
    pub(super) fn item_id(&self) -> Option<&Id> {
        match self {
            Self::Create { item_id, .. } | Self::Item { item_id, .. } => Some(item_id),
            Self::Reorder { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub room_id: Id,
    pub checklist_id: Id,
    pub action: Action,
}
impl Request {
    pub fn validate(&self) -> Result<(), Error> {
        match &self.action {
            Action::Create {
                content, inventory, ..
            } => {
                content.validate()?;
                if matches!(inventory, InventoryExpectation::Revision(0)) {
                    return Err(invalid("Inventory expectations must be positive."));
                }
            }
            Action::Item {
                item_id,
                revision,
                mutation,
            } => {
                if *revision == 0 {
                    return Err(invalid("Item expectations must be positive."));
                }
                if let Mutation::Delete {
                    inventory_revision,
                    confirmation,
                } = mutation
                    && (*inventory_revision == 0
                        || &confirmation.item_id != item_id
                        || confirmation.item_revision != *revision)
                {
                    return Err(invalid(
                        "Delete needs confirmation of this exact item UUID and revision.",
                    ));
                }
                if let Mutation::Edit(edit) = mutation {
                    // Validate supplied fields before any storage side effect.
                    ItemContent::new(
                        edit.title.clone().unwrap_or_else(|| "unchanged".into()),
                        edit.body.clone().flatten(),
                        edit.reference.clone().flatten(),
                    )?;
                }
            }
            Action::Reorder {
                inventory_revision,
                order,
            } => {
                if *inventory_revision == 0
                    || order.iter().collect::<BTreeSet<_>>().len() != order.len()
                {
                    return Err(invalid(
                        "Reorder needs a positive revision and unique UUIDs.",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn invalid(message: &str) -> Error {
    Error::new(Code::InputInvalid, message)
}
fn conflict(message: &str) -> Error {
    Error::new(Code::Conflict, message)
}
fn next(revision: u64) -> Result<u64, Error> {
    revision
        .checked_add(1)
        .ok_or_else(|| Error::storage("Checklist revision space exhausted."))
}

pub(super) struct Outcome {
    pub changed: bool,
    pub item_revision: Option<u64>,
}

/// The caller mutates a disposable candidate under the stable room lock.
/// Returning an error never publishes this candidate.
pub(super) fn apply(
    document: &mut Option<Document>,
    request: &Request,
    assignee: Option<Assignee>,
) -> Result<Outcome, Error> {
    request.validate()?;
    if let Action::Create {
        item_id,
        inventory,
        content,
        ..
    } = &request.action
    {
        match (document.as_ref(), inventory) {
            (None, InventoryExpectation::Absent) => {}
            (Some(d), InventoryExpectation::Revision(revision))
                if d.checklist_id == request.checklist_id && d.inventory_revision == *revision => {}
            _ => {
                return Err(conflict(
                    "Checklist identity or inventory expectation changed.",
                ));
            }
        }
        if let Some(d) = document.as_ref() {
            if d.deleted.iter().any(|t| &t.item_id == item_id) {
                return Err(Error::new(
                    Code::Deleted,
                    "A deleted item UUID cannot be reused.",
                ));
            }
            if d.items.iter().any(|i| &i.id == item_id) {
                return Err(conflict("The item UUID already exists."));
            }
        }
        let item = Item {
            id: item_id.clone(),
            revision: 1,
            content: ItemContent::new(
                content.title.clone(),
                content.body.clone(),
                content.reference.clone(),
            )?,
            assignee,
            completion: Completion::Open,
            archived: false,
        };
        if let Some(d) = document {
            d.inventory_revision = next(d.inventory_revision)?;
            d.items.push(item);
        } else {
            *document = Some(Document {
                room_id: request.room_id.clone(),
                checklist_id: request.checklist_id.clone(),
                inventory_revision: 1,
                items: vec![item],
                deleted: Vec::new(),
            });
        }
        return Ok(Outcome {
            changed: true,
            item_revision: Some(1),
        });
    }
    let d = document
        .as_mut()
        .ok_or_else(|| Error::new(Code::NotFound, "The checklist is absent."))?;
    if d.checklist_id != request.checklist_id {
        return Err(conflict("The exact checklist UUID changed."));
    }
    match &request.action {
        Action::Create { .. } => unreachable!(),
        Action::Reorder {
            inventory_revision,
            order,
        } => {
            if d.inventory_revision != *inventory_revision {
                return Err(conflict("The inventory revision changed."));
            }
            if order.len() != d.items.len()
                || order.iter().collect::<BTreeSet<_>>() != d.items.iter().map(|i| &i.id).collect()
            {
                return Err(invalid(
                    "Reorder must include every nondeleted item, including archived items, exactly once.",
                ));
            }
            if d.items.iter().map(|i| &i.id).eq(order) {
                return Ok(Outcome {
                    changed: false,
                    item_revision: None,
                });
            }
            d.inventory_revision = next(d.inventory_revision)?;
            let mut by_id = std::mem::take(&mut d.items)
                .into_iter()
                .map(|i| (i.id.clone(), i))
                .collect::<std::collections::BTreeMap<_, _>>();
            d.items = order
                .iter()
                .map(|id| by_id.remove(id).expect("validated full permutation"))
                .collect();
            Ok(Outcome {
                changed: true,
                item_revision: None,
            })
        }
        Action::Item {
            item_id,
            revision,
            mutation,
        } => {
            if d.deleted.iter().any(|t| &t.item_id == item_id) {
                return Err(Error::new(Code::Deleted, "The exact item is deleted."));
            }
            let index = d
                .items
                .iter()
                .position(|i| &i.id == item_id)
                .ok_or_else(|| Error::new(Code::NotFound, "The exact item was not found."))?;
            let item = &mut d.items[index];
            if item.revision != *revision {
                return Err(conflict("The item revision changed."));
            }
            if let Mutation::Delete {
                inventory_revision, ..
            } = mutation
            {
                if d.inventory_revision != *inventory_revision {
                    return Err(conflict("The inventory revision changed."));
                }
                let deletion_revision = next(item.revision)?;
                d.inventory_revision = next(d.inventory_revision)?;
                d.items.remove(index);
                d.deleted.push(Tombstone {
                    item_id: item_id.clone(),
                    deletion_revision,
                });
                return Ok(Outcome {
                    changed: true,
                    item_revision: Some(deletion_revision),
                });
            }
            if item.archived && !matches!(mutation, Mutation::Archive | Mutation::Restore) {
                return Err(Error::new(
                    Code::StateInvalid,
                    "Restore the archived item before changing it.",
                ));
            }
            let before = item.clone();
            match mutation {
                Mutation::Edit(edit) => {
                    item.content = ItemContent::new(
                        edit.title
                            .clone()
                            .unwrap_or_else(|| item.content.title.clone()),
                        edit.body
                            .clone()
                            .unwrap_or_else(|| item.content.body.clone()),
                        edit.reference
                            .clone()
                            .unwrap_or_else(|| item.content.reference.clone()),
                    )?;
                }
                Mutation::Assign(_) => {
                    let target =
                        assignee.expect("service admits assignment before domain mutation");
                    if item.assignee.as_ref().is_none_or(|old| old.id != target.id) {
                        item.assignee = Some(target);
                    }
                }
                Mutation::Unassign => item.assignee = None,
                Mutation::Complete => item.completion = Completion::Complete,
                Mutation::Reopen => item.completion = Completion::Open,
                Mutation::Archive => item.archived = true,
                Mutation::Restore => item.archived = false,
                Mutation::Delete { .. } => unreachable!(),
            }
            let changed = before != *item;
            if changed {
                item.revision = next(item.revision)?;
            }
            Ok(Outcome {
                changed,
                item_revision: Some(item.revision),
            })
        }
    }
}

impl Document {
    pub(super) fn encode(&self) -> serde_json::Value {
        use serde_json::json;
        json!({"version":1,"roomId":self.room_id.as_str(),"checklistId":self.checklist_id.as_str(),"inventoryRevision":self.inventory_revision,
            "items":self.items.iter().map(|i| json!({"id":i.id.as_str(),"revision":i.revision,"title":i.content.title,"body":i.content.body,"reference":i.content.reference,
                "assignee":i.assignee.as_ref().map(|a| json!({"id":a.id.as_str(),"label":a.label})),"completion":match i.completion { Completion::Open=>"open",Completion::Complete=>"complete" },"archived":i.archived})).collect::<Vec<_>>(),
            "deleted":self.deleted.iter().map(|d| json!({"roomId":self.room_id.as_str(),"checklistId":self.checklist_id.as_str(),"itemId":d.item_id.as_str(),"deletionRevision":d.deletion_revision})).collect::<Vec<_>>()})
    }

    pub(super) fn decode(value: &serde_json::Value) -> Result<Self, Error> {
        keys(
            value,
            &[
                "version",
                "roomId",
                "checklistId",
                "inventoryRevision",
                "items",
                "deleted",
            ],
        )?;
        if value["version"] != 1 {
            return Err(Error::storage("Unsupported checklist storage version."));
        }
        let room_id = id(value, "roomId")?;
        let checklist_id = id(value, "checklistId")?;
        let mut items = Vec::new();
        for i in array(value, "items")? {
            keys(
                i,
                &[
                    "id",
                    "revision",
                    "title",
                    "body",
                    "reference",
                    "assignee",
                    "completion",
                    "archived",
                ],
            )?;
            let assignee = if i["assignee"].is_null() {
                None
            } else {
                keys(&i["assignee"], &["id", "label"])?;
                Some(Assignee {
                    id: id(&i["assignee"], "id")?,
                    label: text(&i["assignee"], "label")?.into(),
                })
            };
            items.push(Item {
                id: id(i, "id")?,
                revision: positive(i, "revision")?,
                content: ItemContent {
                    title: text(i, "title")?.into(),
                    body: optional(i, "body")?,
                    reference: optional(i, "reference")?,
                },
                assignee,
                completion: match text(i, "completion")? {
                    "open" => Completion::Open,
                    "complete" => Completion::Complete,
                    _ => return Err(Error::storage("Invalid completion.")),
                },
                archived: i["archived"]
                    .as_bool()
                    .ok_or_else(|| Error::storage("Invalid archive flag."))?,
            });
        }
        let mut deleted = Vec::new();
        for t in array(value, "deleted")? {
            keys(t, &["roomId", "checklistId", "itemId", "deletionRevision"])?;
            if id(t, "roomId")? != room_id || id(t, "checklistId")? != checklist_id {
                return Err(Error::storage(
                    "Tombstone identity differs from its document.",
                ));
            }
            deleted.push(Tombstone {
                item_id: id(t, "itemId")?,
                deletion_revision: positive(t, "deletionRevision")?,
            });
        }
        let document = Self {
            room_id,
            checklist_id,
            inventory_revision: positive(value, "inventoryRevision")?,
            items,
            deleted,
        };
        document.validate()?;
        Ok(document)
    }

    pub(super) fn validate(&self) -> Result<(), Error> {
        if self.inventory_revision == 0
            || self.items.len().saturating_add(self.deleted.len()) > 100_000
        {
            return Err(Error::storage(
                "Invalid inventory revision or more than 100000 retained item identities.",
            ));
        }
        let mut ids = BTreeSet::new();
        for item in &self.items {
            item.content.validate().map_err(Error::storage)?;
            if item.content.body.as_ref().is_some_and(String::is_empty) {
                return Err(Error::storage(
                    "Stored empty body must be represented as absent.",
                ));
            }
            if item.revision == 0
                || !ids.insert(&item.id)
                || item.assignee.as_ref().is_some_and(|a| {
                    a.label.trim().is_empty()
                        || a.label.len() > 4096
                        || a.label.chars().any(char::is_control)
                })
            {
                return Err(Error::storage(
                    "Invalid item revision, duplicate UUID or assignee label.",
                ));
            }
        }
        for tombstone in &self.deleted {
            if tombstone.deletion_revision < 2 || !ids.insert(&tombstone.item_id) {
                return Err(Error::storage("Invalid tombstone revision or reused UUID."));
            }
        }
        Ok(())
    }
}

fn keys(value: &serde_json::Value, expected: &[&str]) -> Result<(), Error> {
    let object = value
        .as_object()
        .ok_or_else(|| Error::storage("Expected a checklist object."))?;
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(Error::storage(
            "Unexpected or missing checklist storage fields.",
        ));
    }
    Ok(())
}
fn text<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str, Error> {
    value[key]
        .as_str()
        .ok_or_else(|| Error::storage(format!("Invalid {key}.")))
}
fn id(value: &serde_json::Value, key: &str) -> Result<Id, Error> {
    let text = text(value, key)?;
    let id = Id::parse(text).map_err(Error::storage)?;
    if id.as_str() != text {
        return Err(Error::storage("Stored UUID is not canonical."));
    }
    Ok(id)
}
fn positive(value: &serde_json::Value, key: &str) -> Result<u64, Error> {
    value[key]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or_else(|| Error::storage(format!("Invalid {key}.")))
}
fn optional(value: &serde_json::Value, key: &str) -> Result<Option<String>, Error> {
    if value[key].is_null() {
        Ok(None)
    } else {
        Ok(Some(text(value, key)?.into()))
    }
}
fn array<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a [serde_json::Value], Error> {
    value[key]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| Error::storage(format!("Invalid {key}.")))
}

#[cfg(test)]
mod tests;
