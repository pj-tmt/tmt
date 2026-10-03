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
        self.apply(
            store,
            key,
            super::OwnerRequest {
                operation_id: request.operation_id,
                expected_revision: request.expected_revision,
                action: super::OwnerAction::Link(request.action),
                transport_digest: None,
                scope: None,
            },
            now,
        )
        .map(|applied| applied.outcome)
    }
}
pub(super) fn action<'a>(key: &Keyring, action: LinkAction<'a>) -> crate::Result<Action<'a>> {
    let (remove, add, seed) = match action {
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
    Ok(Action::Link { remove, add, seed })
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
