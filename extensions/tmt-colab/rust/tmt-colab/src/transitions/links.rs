//! Root-local link seeds are borrowed for derivation, never persisted or returned.
use super::{Engine, TransitionError, membership::Action};
use crate::{
    keyring::Keyring,
    store::{Store, owner::Recipient},
};
use tmt_colab_model::link;

pub struct LinkSpec<'a> {
    pub id: &'a str,
    pub seed: &'a [u8; 32],
    pub role: &'a str,
    pub pages: Vec<String>,
}
pub enum LinkAction<'a> {
    Add(LinkSpec<'a>),
    Remove {
        link_id: &'a str,
    },
    Reset {
        link_id: &'a str,
        replacement: Option<LinkSpec<'a>>,
    },
}
pub struct LinkRequest<'a> {
    pub operation_id: &'a str,
    pub expected_revision: u64,
    pub action: LinkAction<'a>,
}
impl Engine {
    pub fn link(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        request: LinkRequest<'_>,
        now: u64,
    ) -> std::result::Result<Vec<u8>, TransitionError> {
        let result = (|| {
            let (remove, add, seed) = match request.action {
                LinkAction::Add(spec) => (None, Some(recipient(key, &spec)?), None),
                LinkAction::Remove { link_id } => (Some(link_id), None, None),
                LinkAction::Reset {
                    link_id,
                    replacement,
                } => {
                    let add = replacement
                        .as_ref()
                        .map(|spec| recipient(key, spec))
                        .transpose()?;
                    (Some(link_id), add, replacement.map(|spec| spec.seed))
                }
            };
            self.change(
                store,
                key,
                request.operation_id,
                request.expected_revision,
                Action::Link { remove, add, seed },
                now,
            )
            .map(|(outcome, _)| outcome)
        })();
        result.map_err(TransitionError::from_error)
    }
}
fn recipient(key: &Keyring, spec: &LinkSpec<'_>) -> crate::Result<Recipient> {
    let keys = link::Keys::derive(spec.seed, &key.space_id, spec.id)?;
    Ok(Recipient {
        kind: "link".into(),
        id: spec.id.into(),
        role: Some(spec.role.into()),
        signing_key: keys.signing_public(),
        encryption_key: keys.recipient().public_key(),
        pages: spec.pages.clone(),
        revoked: false,
    })
}
