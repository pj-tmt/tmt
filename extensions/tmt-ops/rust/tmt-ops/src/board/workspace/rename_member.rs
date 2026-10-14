//! Member rename consumes Core's UUID/name CAS; it owns no identity or timer store.
use crate::core::{Core, SquadError};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    pub identity_id: String,
    pub expected_name: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    Applied { name: String, changed: bool },
    Conflict { name: String },
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Completed {
    pub request: Request,
    pub outcome: Result<Outcome, SquadError>,
}

fn invalid() -> SquadError {
    SquadError::new(
        "SQUAD_IDENTITY_RENAME_INVALID",
        "Core did not return a matching rename result; the outcome is unknown.",
    )
}

impl Request {
    pub fn apply(&self, core: &Core) -> Result<Outcome, SquadError> {
        let document = core.api(
            "identity.rename",
            json!({
                "identityId": self.identity_id,
                "expectedName": self.expected_name,
                "name": self.name,
            }),
        )?;
        self.decode(&document)
    }

    fn decode(&self, document: &Value) -> Result<Outcome, SquadError> {
        if document["identityId"].as_str() != Some(self.identity_id.as_str()) {
            return Err(invalid());
        }
        let name = || {
            document["name"]
                .as_str()
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .ok_or_else(invalid)
        };
        match document["outcome"].as_str() {
            Some("applied") => Ok(Outcome::Applied {
                name: name()?,
                changed: document["changed"].as_bool().ok_or_else(invalid)?,
            }),
            Some("conflict") => Ok(Outcome::Conflict { name: name()? }),
            Some("not-found") => Ok(Outcome::NotFound),
            _ => Err(invalid()),
        }
    }
}

impl crate::board::app::App {
    pub(in crate::board) fn finished_member_rename(&mut self, completed: Completed) {
        use super::manage::{Change, Kind, Stage};
        let Some(draft) = self.workspace_form.as_mut().filter(|draft| {
            draft.kind == Kind::RenameMember
                && draft.stage == Stage::Pending
                && draft.operation.as_ref()
                    == Some(&Change::RenameMember(completed.request.clone()))
        }) else {
            return;
        };
        let id = &completed.request.identity_id;
        let message = match completed.outcome {
            Ok(Outcome::Applied { name, changed }) => format!(
                "{}: {name:?}. Identity UUID: {id}. The public workspace will refresh; bindings, memberships, history, drafts and timers stay with this identity.",
                if changed { "Renamed" } else { "Name unchanged" },
            ),
            Ok(Outcome::Conflict { name }) => format!(
                "Not applied: the current name is {name:?}. Identity UUID: {id}. Close and refresh, then review a new rename explicitly. The old review cannot be submitted again.",
            ),
            Ok(Outcome::NotFound) => format!(
                "Not applied: this identity is missing or retired. Identity UUID: {id}. No same-name successor was selected.",
            ),
            Err(error) => format!(
                "{}: {}. Identity UUID: {id}. Close and refresh before a new explicit review. Nothing is retried automatically.",
                if matches!(
                    error.code.as_str(),
                    "INVALID_NAME"
                        | "NAME_ALREADY_ACTIVE"
                        | "API_INPUT_INVALID"
                        | "STORAGE_NOT_WRITABLE"
                ) {
                    "Not applied"
                } else {
                    "Unknown"
                },
                error.message,
            ),
        };
        let message = super::render::safe(&message);
        draft.stage = Stage::Result(message.clone());
        draft.invalidate();
        *draft.scroll.borrow_mut() = Default::default();
        self.notice = Some(message);
    }
}

#[cfg(test)]
mod tests;
