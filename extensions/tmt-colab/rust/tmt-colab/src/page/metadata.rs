//! Closed owner-browser metadata commands. They use the existing structured publisher and
//! save operation/status lifetime; they grant no link editor or commenter content authority.
use super::{
    StructuredAction,
    save::{Prepared, SourceOpener},
};
use crate::decoder::StructuredKind;
use serde_json::Value;

pub struct MetadataEdit {
    pub page: String,
    pub operation_id: String,
    pub expected_revision: String,
    pub action: String,
    pub body: Value,
}

/// Reject unknown actions and excess fields before opening a native source.
pub fn validate(action: &str, body: &Value) -> crate::Result<StructuredKind> {
    let invalid = || super::Fault::Invalid;
    match action {
        "page.state" => {
            crate::page_state::validate_action(action, body).map_err(|_| invalid())?;
            Ok(StructuredKind::PageState)
        }
        "creator.set" => {
            crate::question_registry::validate_creator_selection(body).map_err(|_| invalid())?;
            // A browser never chooses the editor's machine attribution.
            if body.get("machineId").is_some() {
                return Err(invalid().into());
            }
            Ok(StructuredKind::Creator)
        }
        "question.handle" | "question.reopen" => {
            crate::closed::keys(body, &["id"], &[]).map_err(|_| invalid())?;
            if !body["id"]
                .as_str()
                .is_some_and(crate::page_check::valid_question_id)
            {
                return Err(invalid().into());
            }
            Ok(StructuredKind::Questions)
        }
        _ => Err(invalid().into()),
    }
}

pub fn prepare(
    open: &SourceOpener,
    edit: &MetadataEdit,
    clock: &dyn Fn() -> crate::Result<u64>,
) -> crate::Result<Prepared> {
    let kind = validate(&edit.action, &edit.body)?;
    let mut source = open()?;
    let prepared = super::prepare_structured_publication(
        &source.store,
        &source.keyring,
        &edit.page,
        StructuredAction {
            kind,
            action: &edit.action,
            body: &edit.body,
            expected_revision: &edit.expected_revision,
            display_agent: None,
            operation_id: Some(&edit.operation_id),
        },
        &mut source.decoder,
        clock,
    )?;
    super::save::structured(&source, &edit.page, &edit.operation_id, prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn browser_metadata_is_closed_and_cannot_register_or_forge_attribution() {
        let id = "10000000-0000-4000-8000-000000000001";
        for action in ["question.handle", "question.reopen"] {
            assert!(validate(action, &json!({"id":id})).is_ok());
            for body in [
                json!({"id":id,"task":"T-1"}),
                json!({"id":"bad"}),
                json!({"id":id,"attribution":{}}),
            ] {
                assert!(validate(action, &body).is_err());
            }
        }
        let body = json!({"agent":{"id":id,"name":"A creator"}});
        assert!(validate("creator.set", &body).is_ok());
        assert!(
            validate(
                "creator.set",
                &json!({"agent":body["agent"],"machineId":id})
            )
            .is_err()
        );
        for action in [
            "question.add",
            "task.add",
            "creator.clear",
            "question.answered",
        ] {
            assert!(validate(action, &body).is_err());
        }
    }
}
