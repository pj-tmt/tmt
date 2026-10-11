//! Root-local namespace management. A deleted page's signed policy survives its
//! payloads; only that committed tombstone admits removing its object namespace.
use super::{CallbackOwner, ChannelOwner, Client};
use crate::{Result, attachments, fold, management::Storage, page, page::save::SourceOpener};
use std::{sync::Arc, time::Instant};
use tmt_colab_model::values;
use tmt_extension_objects::{
    Admit, AdmitInput, Bytes32, Call, Context, Decision, Disclosure, Method, NamespaceInput,
    Origin, Outcome, Success, Usage, UsageScope,
};

/// Verify the retained membership log rather than opening deleted payloads.
fn policy(source: &SourceOpener, page: &str) -> Result<(String, bool)> {
    values::generated_id(page)?;
    let view = source()?;
    policy_view(&view, page)
}
fn policy_view(view: &page::save::Source, page: &str) -> Result<(String, bool)> {
    let deleted =
        view.store
            .owner_read(&view.keyring.space_id, &view.keyring.owner_public(), |tx| {
                let (states, _) = fold::verify_log(&tx.log()?, &view.keyring, page)?;
                let authority = states.last().ok_or(page::Fault::Invalid)?;
                if tx.head() != Some(&authority.head) || !tx.pages()?.iter().any(|id| id == page) {
                    return Err(page::Fault::Invalid.into());
                }
                Ok(authority.policy.deleted)
            })?;
    Ok((view.keyring.space_id.clone(), deleted))
}

struct Admission {
    client: Client,
    source: SourceOpener,
    page: String,
    namespace: Bytes32,
    removing: bool,
}
impl CallbackOwner for Admission {
    fn decide(&self, admit: &Admit, deadline: Instant) -> Decision {
        let input = match &admit.operation.input {
            AdmitInput::RemoveNamespace(value) => {
                self.removing && value.namespace == self.namespace
            }
            AdmitInput::Usage(UsageScope::Namespace(value)) => {
                !self.removing && *value == self.namespace
            }
            _ => false,
        };
        let disclosure = match admit.operation.disclosure {
            None => true,
            Some(Disclosure::Removed) => self.removing,
            Some(Disclosure::Usage(_)) => !self.removing,
            _ => false,
        };
        let current = policy(&self.source, &self.page).is_ok_and(|(space, deleted)| {
            (!self.removing || deleted)
                && attachments::namespace(&space, &self.page)
                    .is_ok_and(|value| Bytes32::from_bytes(value) == self.namespace)
        });
        if input
            && disclosure
            && current
            && Instant::now() < deadline
            && admit.generation == self.client.generation()
            && admit.context == Context::LocalExtension
            && self.client.standing(Origin::LocalExtension)
        {
            Decision::Allow
        } else {
            Decision::Deny
        }
    }
}

impl ChannelOwner {
    /// One bounded, rotating pass over retained committed tombstones. The
    /// maintenance worker resets `after` when a fresh channel is established.
    pub(crate) fn sweep_deletions(
        &self,
        source: &SourceOpener,
        view: &page::save::Source,
        after: &mut Option<String>,
        deadline: Instant,
    ) -> Result<bool> {
        let pages =
            view.store
                .owner_read(&view.keyring.space_id, &view.keyring.owner_public(), |tx| {
                    tx.pages()
                })?;
        let mut unfinished = false;
        for (checked, page) in pages
            .iter()
            .filter(|page| after.as_ref().is_none_or(|last| *page > last))
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .enumerate()
        {
            if checked == 4 || Instant::now() >= deadline {
                return Ok(true);
            }
            *after = Some(page.clone());
            match policy_view(view, &page) {
                Ok((_, true)) => {
                    unfinished |=
                        self.remove_page_namespace(&page, source, deadline) != Storage::Freed;
                }
                Ok((_, false)) => {}
                Err(_) => unfinished = true,
            }
        }
        *after = None;
        Ok(unfinished)
    }

    pub(crate) fn remove_page_namespace(
        &self,
        page: &str,
        source: &SourceOpener,
        deadline: Instant,
    ) -> Storage {
        let Ok((space, true)) = policy(source, page) else {
            return Storage::Pending;
        };
        let Some(client) = self.client() else {
            return Storage::Pending;
        };
        match client.supports(Method::RemoveNamespace) {
            Ok(true) => {}
            Ok(false) => return Storage::Unsupported,
            Err(_) => return Storage::Pending,
        }
        let Ok(namespace) = attachments::namespace(&space, page) else {
            return Storage::Pending;
        };
        let namespace = Bytes32::from_bytes(namespace);
        let owner = Arc::new(Admission {
            client: client.clone(),
            source: source.clone(),
            page: page.into(),
            namespace,
            removing: true,
        });
        match client.request(
            Origin::LocalExtension,
            Call::RemoveNamespace(NamespaceInput { namespace }),
            owner,
            deadline,
        ) {
            Ok(Outcome::Success(Success::Removed)) => Storage::Freed,
            Ok(Outcome::Failure(tmt_extension_objects::ErrorCode::Unknown)) | Err(_) => {
                Storage::Unknown
            }
            _ => Storage::Pending,
        }
    }
}
impl Client {
    pub(crate) fn usage_details(
        &self,
        page: &str,
        source: &SourceOpener,
        deadline: Instant,
    ) -> attachments::UsageDetails {
        match self.namespace_usage(page, source, deadline) {
            Ok(Some(value)) => value.into(),
            Ok(None) => attachments::UsageDetails::Unsupported {
                hint: tmt_extension_objects::unsupported_hint().into(),
            },
            Err(_) => attachments::UsageDetails::Unknown,
        }
    }
    fn namespace_usage(
        &self,
        page: &str,
        source: &SourceOpener,
        deadline: Instant,
    ) -> Result<Option<Usage>> {
        let (space, _) = policy(source, page)?;
        let client = self;
        if !client.supports(Method::Usage)? {
            return Ok(None);
        }
        let namespace = Bytes32::from_bytes(attachments::namespace(&space, page)?);
        let owner = Arc::new(Admission {
            client: client.clone(),
            source: source.clone(),
            page: page.into(),
            namespace,
            removing: false,
        });
        match client.request(
            Origin::LocalExtension,
            Call::Usage(UsageScope::Namespace(namespace)),
            owner,
            deadline,
        ) {
            Ok(Outcome::Success(Success::Usage(value))) => Ok(Some(value)),
            _ => Err(page::Fault::Unavailable.into()),
        }
    }
}

impl From<tmt_extension_objects::Usage> for attachments::UsageDetails {
    fn from(value: tmt_extension_objects::Usage) -> Self {
        Self::Known {
            charged_bytes: value.charged_bytes,
            entries: value.entries,
            active_uploads: value.active_uploads,
            retained_identities: value.retained_identities,
        }
    }
}
