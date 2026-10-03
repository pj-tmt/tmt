//! Stable root-local dispatch seam for admitted device management requests.
use super::{
    Engine, EpochAdvance, LinkAction, MemberAction, TransitionError, links,
    membership::Action,
    sharing::{PageAction, PageRequest},
};
use crate::{
    Result,
    keyring::Keyring,
    store::{Store, owner::OwnerFault},
};
use serde_json::Value;
use tmt_colab_model::{crypto, framing, statement, values};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareMode {
    Private,
    Link,
    Public,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryMode {
    Shared,
    Current,
}
/// Trusted composition selects the backend, never an unsigned browser assertion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Publication {
    Loopback,
    Cloud,
}
pub struct OwnerRequest<'a> {
    pub operation_id: &'a str,
    pub expected_revision: u64,
    pub action: OwnerAction<'a>,
    pub transport_digest: Option<[u8; 32]>,
    pub scope: Option<RequestScope>,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct RequestScope {
    pub initiating_page: String,
    pub affected_pages: Vec<String>,
}
pub enum OwnerAction<'a> {
    Create {
        page: &'a str,
        title: &'a str,
        source: &'a str,
    },
    EpochAdvance {
        page: &'a str,
    },
    Member(MemberAction),
    Link(LinkAction<'a>),
    DeviceRevoke {
        device_id: &'a str,
        grant_revision: u64,
    },
    Share {
        page: &'a str,
        mode: ShareMode,
        publication: Publication,
    },
    History {
        page: &'a str,
        mode: HistoryMode,
    },
    Retention {
        page: &'a str,
        days: Option<u64>,
    },
    Archive {
        page: &'a str,
    },
    Delete {
        page: &'a str,
    },
}
#[derive(Debug)]
pub struct Applied {
    pub outcome: Vec<u8>,
    /// The outcome's committed head, including on replay after later mutations.
    pub head: statement::Head,
    pub replayed: bool,
}
#[derive(Clone, Copy)]
pub(super) struct OwnerContext<'a> {
    pub id: &'a str,
    pub expected: u64,
    pub transport: Option<[u8; 32]>,
    pub scope: Option<&'a RequestScope>,
}
impl OwnerContext<'_> {
    pub fn check_scope(&self, pages: &[String]) -> Result<()> {
        if let Some(scope) = self.scope {
            let ids = scope
                .affected_pages
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            if values::generated_id(&scope.initiating_page).is_err()
                || framing::id_list(&ids, false).is_err()
                || !scope.affected_pages.contains(&scope.initiating_page)
                || scope.affected_pages != pages
            {
                return Err(OwnerFault::StaleHead.into());
            }
        }
        Ok(())
    }
    pub fn digest(&self, key: &Keyring, operation: &str, selected: &Value) -> Result<[u8; 32]> {
        // Preserve root-local operation digests so existing durable receipts replay.
        let base = if operation == "epoch.advance" {
            crypto::digest(&framing::frame(&[
                b"tmt-colab-local-epoch-request-v1",
                key.space_id.as_bytes(),
                self.id.as_bytes(),
                self.expected.to_string().as_bytes(),
                selected["pageId"]
                    .as_str()
                    .ok_or(OwnerFault::Invalid)?
                    .as_bytes(),
            ])?)
        } else {
            crypto::digest(&framing::frame(&[
                b"tmt-colab-local-transition-v1",
                key.space_id.as_bytes(),
                self.id.as_bytes(),
                self.expected.to_string().as_bytes(),
                operation.as_bytes(),
                &serde_json::to_vec(selected)?,
            ])?)
        };
        let base = if let Some(scope) = self.scope {
            crypto::digest(&framing::frame(&[
                b"tmt-colab-scoped-transition-v1",
                &base,
                &serde_json::to_vec(scope)?,
            ])?)
        } else {
            base
        };
        Ok(if let Some(transport) = self.transport {
            crypto::digest(&framing::frame(&[
                b"tmt-colab-transport-transition-v1",
                &base,
                &transport,
            ])?)
        } else {
            base
        })
    }
}
impl Engine {
    /// Caller holds sync before Registration and admits the live management
    /// session/signature/scope before invoking this root-authorized seam.
    pub fn apply(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        request: OwnerRequest<'_>,
        now: u64,
    ) -> std::result::Result<Applied, TransitionError> {
        self.apply_request(store, key, request, now)
            .map_err(TransitionError::from_error)
    }
    fn apply_request(
        &mut self,
        store: &mut Store,
        key: &Keyring,
        request: OwnerRequest<'_>,
        now: u64,
    ) -> Result<Applied> {
        let context = OwnerContext {
            id: request.operation_id,
            expected: request.expected_revision,
            transport: request.transport_digest,
            scope: request.scope.as_ref(),
        };
        let (outcome, changed) = match request.action {
            OwnerAction::Create {
                page,
                title,
                source,
            } => self.create_page(
                store,
                key,
                context,
                super::create::Selection {
                    page,
                    title,
                    source,
                },
                now,
            )?,
            OwnerAction::EpochAdvance { page } => self.advance(
                store,
                key,
                EpochAdvance {
                    operation_id: context.id,
                    expected_revision: context.expected,
                    page,
                },
                now,
                context,
            )?,
            OwnerAction::Member(action) => {
                self.change(store, key, context, Action::Member(action), now)?
            }
            OwnerAction::Link(action) => {
                self.change(store, key, context, links::action(key, action)?, now)?
            }
            OwnerAction::DeviceRevoke {
                device_id,
                grant_revision,
            } => self.change(
                store,
                key,
                context,
                Action::Device {
                    id: device_id,
                    grant: grant_revision,
                },
                now,
            )?,
            action => {
                let (page, action) = match action {
                    OwnerAction::Share {
                        page,
                        mode,
                        publication,
                    } => (page, PageAction::Share { mode, publication }),
                    OwnerAction::History { page, mode } => (page, PageAction::History(mode)),
                    OwnerAction::Retention { page, days } => (page, PageAction::Retention(days)),
                    OwnerAction::Archive { page } => (page, PageAction::Archive),
                    OwnerAction::Delete { page } => (page, PageAction::Delete),
                    _ => unreachable!(),
                };
                self.page_change(
                    store,
                    key,
                    PageRequest {
                        operation_id: context.id,
                        expected_revision: context.expected,
                        page,
                        action,
                    },
                    now,
                    context,
                )?
            }
        };
        let wire: Value = serde_json::from_slice(&outcome)?;
        let head = if let Some(last) = wire["statements"]
            .as_array()
            .ok_or(OwnerFault::Invalid)?
            .last()
        {
            let envelope = statement::Envelope::from_json(&serde_json::to_vec(last)?)?;
            let bytes =
                values::binary(last["statement"].as_str().ok_or(OwnerFault::Invalid)?, 1024)?;
            let header = statement::decode(&bytes)?;
            let revision = values::decimal(header.revision, false)?;
            let previous = statement::Head {
                revision: revision.checked_sub(1).ok_or(OwnerFault::Invalid)?,
                hash: *header.previous_hash,
                owner_member: key.management_member()?,
            };
            envelope
                .verify_next(&key.space_id, &key.owner_public(), Some(&previous))?
                .head
        } else {
            store
                .owner_head(&key.space_id, &key.owner_public())?
                .ok_or(OwnerFault::Invalid)?
        };
        Ok(Applied {
            outcome,
            head,
            replayed: !changed,
        })
    }
}
