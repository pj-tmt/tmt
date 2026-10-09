//! Fresh Colab admission for one actual sync peer and one captured request.
//! Policy bytes and Remote context never substitute for this peer's native owner.
use super::{CallbackOwner, Client};
use crate::{
    attachments,
    fold::Snapshot,
    page,
    registration::OwnerAdmission,
    sync::{Access, Admission, SyncScope},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tmt_colab_model::attachment::Descriptor;
use tmt_extension_objects::{
    Admit, AdmitInput, BeginInput, Context, Decision, Disclosure, Origin, Uuid4,
};

pub(crate) struct PeerIdentity {
    pub client: Client,
    pub origin: Origin,
    pub principal: String,
    pub forwarded: Option<String>,
    pub reader: bool,
    pub closed: Arc<AtomicBool>,
    pub admission: OwnerAdmission,
}
impl PeerIdentity {
    fn context_matches(&self, context: Context) -> bool {
        match (self.origin, context) {
            (
                Origin::Mounted(expected),
                Context::OwnerSession {
                    origin_id,
                    device_id,
                    grant_revision,
                },
            ) => {
                expected == origin_id
                    && self
                        .forwarded
                        .as_deref()
                        .and_then(|raw| OwnerAdmission::object_remote_binding(raw).ok())
                        .is_some_and(|(device, revision)| {
                            Uuid4::parse(&device).ok() == Some(device_id)
                                && revision == grant_revision
                        })
            }
            (Origin::Mounted(expected), Context::Mounted { origin_id }) => {
                expected == origin_id && self.reader && self.forwarded.is_none()
            }
            _ => false,
        }
    }
    pub(crate) fn standing(&self) -> bool {
        !self.closed.load(Ordering::Acquire) && self.client.standing(self.origin)
    }
    pub(crate) fn capture_scope(&self, scope: &SyncScope, write: bool) -> crate::Result<[u8; 32]> {
        if !self.standing() || (write && self.reader) {
            return Err(page::Fault::Denied.into());
        }
        if !self.reader {
            self.admission
                .recheck_object_remote(self.forwarded.as_deref().ok_or(page::Fault::Denied)?)?;
        }
        self.admission.authorize(
            &self.principal,
            scope,
            if write { Access::Publish } else { Access::Read },
        )?;
        self.admission.attachment_context(
            &self.principal,
            scope,
            tmt_colab_model::values::decimal(&scope.epoch, false)?,
        )
    }
}
pub(crate) enum CapturedTarget {
    Scope {
        base: String,
    },
    Upload {
        descriptor: Box<Descriptor>,
        base: String,
    },
    Read(Arc<attachments::AdmittedAttachmentRead>),
}
pub(crate) struct RequestCapture {
    pub peer: Arc<PeerIdentity>,
    pub scope: SyncScope,
    pub context: [u8; 32],
    pub target: CapturedTarget,
    pub source: page::save::SourceOpener,
    pub write: bool,
    pub mutation: bool,
    pub deadline: Instant,
}
impl RequestCapture {
    pub(crate) fn current(&self) -> crate::Result<()> {
        if self.peer.capture_scope(&self.scope, self.write)? != self.context {
            return Err(page::Fault::Denied.into());
        }
        match &self.target {
            CapturedTarget::Scope { base } => {
                let source = (self.source)()?;
                if page::revision(&source.store, &source.keyring, &self.scope.page)? != *base {
                    return Err(page::Fault::StaleBase.into());
                }
            }
            CapturedTarget::Upload { descriptor, base } => {
                // Initial target decoding happened outside all live locks. A
                // changed cut/head/base cannot reuse that admitted target.
                let source = (self.source)()?;
                if page::revision(&source.store, &source.keyring, &self.scope.page)? != *base {
                    return Err(page::Fault::StaleBase.into());
                }
                let snapshot = Snapshot::capture(&source.store, &source.keyring, &self.scope.page)?;
                snapshot.asset_author(&source.keyring, descriptor)?;
            }
            CapturedTarget::Read(capture) => {
                let source = (self.source)()?;
                capture.recheck(&source.store, &source.keyring)?;
            }
        }
        if Instant::now() >= self.deadline {
            return Err(page::Fault::Unavailable.into());
        }
        if self.peer.capture_scope(&self.scope, self.write)? != self.context {
            return Err(page::Fault::Denied.into());
        }
        Ok(())
    }
}
pub(crate) struct RequestAdmission {
    pub capture: Arc<RequestCapture>,
    pub input: AdmitInput,
    pub original: Option<BeginInput>,
}
impl RequestAdmission {
    fn disclosure_matches(&self, disclosure: Option<Disclosure>) -> bool {
        match disclosure {
            Some(Disclosure::Receipt {
                opaque_key,
                payload_sha256,
                payload_bytes,
            }) => {
                let frozen = match &self.input {
                    AdmitInput::Begin(input) => {
                        return input.opaque_key == opaque_key
                            && input.payload_sha256 == payload_sha256
                            && input.payload_bytes == payload_bytes;
                    }
                    AdmitInput::Commit(input) => &input.retained,
                    AdmitInput::Status(_) => {
                        return self.original.as_ref().is_some_and(|input| {
                            input.opaque_key == opaque_key
                                && input.payload_sha256 == payload_sha256
                                && input.payload_bytes == payload_bytes
                        });
                    }
                    _ => return false,
                };
                frozen.opaque_key == opaque_key
                    && frozen.payload_sha256 == payload_sha256
                    && frozen.payload_bytes == payload_bytes
            }
            Some(Disclosure::Bytes { offset, length }) => {
                matches!(&self.input, AdmitInput::Read(input) if input.offset == offset && input.count == length)
            }
            _ => true, // The neutral ledger admits the method's other exact classes.
        }
    }
}
impl CallbackOwner for RequestAdmission {
    fn decide(&self, admit: &Admit, deadline: Instant) -> Decision {
        let deadline = deadline.min(self.capture.deadline);
        if !self.capture.peer.context_matches(admit.context)
            || admit.generation != self.capture.peer.client.generation()
            || admit.operation.input != self.input
            || !self.disclosure_matches(admit.operation.disclosure)
        {
            return Decision::Deny;
        }
        if Instant::now() >= deadline {
            return Decision::Unavailable;
        }
        if self.capture.current().is_err() {
            return Decision::Deny;
        }
        if Instant::now() >= deadline {
            return Decision::Unavailable;
        }
        if !self.capture.peer.standing() {
            return Decision::Deny;
        }
        Decision::Allow
    }
}

/// Actual root-local capture, never a Remote owner/reader or a policy credential.
/// Only read disclosures can pass; native writes cannot borrow extension scope.
pub(super) struct RootReadAdmission {
    pub capture: Arc<attachments::AdmittedAttachmentRead>,
    pub source: page::save::SourceOpener,
    pub client: Client,
    pub deadline: Instant,
}
impl CallbackOwner for RootReadAdmission {
    fn decide(&self, admit: &Admit, deadline: Instant) -> Decision {
        if admit.context != Context::LocalExtension
            || admit.generation != self.client.generation()
            || !matches!(admit.operation.input, AdmitInput::Read(_))
        {
            return Decision::Deny;
        }
        let deadline = deadline.min(self.deadline);
        if Instant::now() >= deadline {
            return Decision::Unavailable;
        }
        if !self.client.standing(Origin::LocalExtension) {
            return Decision::Deny;
        }
        let checked =
            (self.source)().and_then(|source| self.capture.recheck(&source.store, &source.keyring));
        if checked.is_err() {
            return Decision::Deny;
        }
        if Instant::now() >= deadline {
            return Decision::Unavailable;
        }
        if !self.client.standing(Origin::LocalExtension) {
            return Decision::Deny;
        }
        Decision::Allow
    }
}
