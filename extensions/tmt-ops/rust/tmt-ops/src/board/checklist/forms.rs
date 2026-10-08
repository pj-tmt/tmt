//! Retained authored fields and exact preview expectations, separate from live reads.
use crate::checklist::model::{
    Action, Edit, Id, InventoryExpectation, ItemContent, Mutation, Request,
};
use crate::checklist::{Code, Error, Preview};

#[derive(Clone)]
pub(super) struct Draft {
    pub actor: Id,
    pub request: Request,
    pub title: String,
    pub body: String,
    pub reference: String,
    pub fresh: bool,
}
fn invalid(message: &str) -> Error {
    Error {
        code: Code::InputInvalid,
        message: message.into(),
        current: None,
    }
}
impl Draft {
    pub fn create(preview: &Preview, ids: (Id, Id)) -> Self {
        let current = &preview.current;
        Self {
            actor: preview.actor_id.clone(),
            request: Request {
                room_id: current.room.id.clone(),
                checklist_id: current.checklist_id.clone().unwrap_or(ids.0),
                action: Action::Create {
                    item_id: ids.1,
                    inventory: current
                        .inventory_revision
                        .map_or(InventoryExpectation::Absent, InventoryExpectation::Revision),
                    content: ItemContent {
                        title: String::new(),
                        body: None,
                        reference: None,
                    },
                    assignee: None,
                },
            },
            title: String::new(),
            body: String::new(),
            reference: String::new(),
            fresh: true,
        }
    }
    pub fn edit(preview: &Preview, id: &Id) -> Option<Self> {
        let current = &preview.current;
        let item = &current.items.iter().find(|v| &v.item.id == id)?.item;
        Some(Self {
            actor: preview.actor_id.clone(),
            request: Request {
                room_id: current.room.id.clone(),
                checklist_id: current.checklist_id.clone()?,
                action: Action::Item {
                    item_id: id.clone(),
                    revision: item.revision,
                    mutation: Mutation::Edit(Edit {
                        title: None,
                        body: None,
                        reference: None,
                    }),
                },
            },
            title: item.content.title.clone(),
            body: item.content.body.clone().unwrap_or_default(),
            reference: item.content.reference.clone().unwrap_or_default(),
            fresh: true,
        })
    }
    pub fn submit(&self) -> Result<Request, Error> {
        if !self.fresh {
            return Err(invalid(
                "Review current state before making a fresh preview.",
            ));
        }
        let content = ItemContent::new(
            self.title.clone(),
            Some(self.body.clone()),
            (!self.reference.is_empty()).then(|| self.reference.clone()),
        )?;
        let mut request = self.request.clone();
        match &mut request.action {
            Action::Create {
                content: fields, ..
            } => *fields = content,
            Action::Item {
                mutation: Mutation::Edit(edit),
                ..
            } => {
                *edit = Edit {
                    title: Some(content.title),
                    body: Some(content.body),
                    reference: Some(content.reference),
                }
            }
            _ => return Err(invalid("This draft is not an authored content operation.")),
        }
        request.validate()?;
        Ok(request)
    }
    /// Explicitly adopt fresh expectations without overwriting authored fields.
    pub fn review(&mut self, preview: &Preview) -> Result<(), Error> {
        if self.actor != preview.actor_id || self.request.room_id != preview.current.room.id {
            return Err(invalid(
                "Current actor or exact room differs from the retained draft.",
            ));
        }
        review(&mut self.request, preview)?;
        self.fresh = true;
        Ok(())
    }
    pub fn text_mut(&mut self, id: &str) -> Option<&mut String> {
        match id {
            "title" => Some(&mut self.title),
            "body" => Some(&mut self.body),
            "reference" => Some(&mut self.reference),
            _ => None,
        }
    }
}
pub(super) fn review(request: &mut Request, preview: &Preview) -> Result<(), Error> {
    let current = &preview.current;
    if request.room_id != current.room.id {
        return Err(invalid("The exact room changed."));
    }
    match &mut request.action {
        Action::Create { inventory, .. } => {
            if let Some(id) = &current.checklist_id {
                request.checklist_id = id.clone();
            }
            *inventory = current
                .inventory_revision
                .map_or(InventoryExpectation::Absent, InventoryExpectation::Revision);
        }
        Action::Item {
            item_id,
            revision,
            mutation,
        } => {
            if current.checklist_id.as_ref() != Some(&request.checklist_id) {
                return Err(invalid("The exact checklist is no longer available."));
            }
            *revision = current
                .items
                .iter()
                .find(|v| &v.item.id == item_id)
                .ok_or_else(|| invalid("The exact item is no longer available."))?
                .item
                .revision;
            if let Mutation::Delete {
                inventory_revision,
                confirmation,
            } = mutation
            {
                *inventory_revision = current
                    .inventory_revision
                    .ok_or_else(|| invalid("No current inventory revision."))?;
                confirmation.item_revision = *revision;
            }
        }
        Action::Reorder {
            inventory_revision,
            order,
        } => {
            let current_order: Vec<_> = current.items.iter().map(|v| v.item.id.clone()).collect();
            if current_order.len() != order.len()
                || !current_order.iter().all(|id| order.contains(id))
            {
                return Err(invalid(
                    "The full inventory changed. Cancel and start a new reorder.",
                ));
            }
            *inventory_revision = current
                .inventory_revision
                .ok_or_else(|| invalid("No current inventory revision."))?;
        }
    }
    Ok(())
}
