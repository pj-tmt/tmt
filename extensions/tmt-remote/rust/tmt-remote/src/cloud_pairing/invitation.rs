//! The shared invitation port over the existing Firestore/Auth/runtime provider.
//! Route publication delegates to the existing desired-route worker and journal.
use super::{
    confirmation::{FirestoreConfirmation, RoutePublication},
    wire,
};
use crate::{
    cloud_inbox::Provider,
    error::RemoteError,
    invitations::{BackendBinding, Context, port::*},
    native_sync::complete,
    objects::IoBudget,
    store::Store,
};
use std::{
    future::Future,
    sync::{Arc, Condvar, Mutex},
};

#[derive(Clone)]
pub struct Firestore {
    provider: Arc<Provider>,
    context: Context,
    confirmation: FirestoreConfirmation,
    store: Arc<Mutex<Store>>,
    wake: Arc<(Mutex<bool>, Condvar)>,
}
fn run<T>(
    future: impl Future<Output = Result<T, RemoteError>>,
    io: &IoBudget<'_>,
) -> Result<T, BackendFailure> {
    complete(future, io)
        .map_err(|_| BackendFailure::Unavailable)?
        .map_err(BackendFailure::Provider)
}
impl Firestore {
    /// Composition has checked the registered owner's native pin and uid against
    /// this exact context. This adapter supplies neither that pin nor consent.
    pub fn new(
        provider: Arc<Provider>,
        context: Context,
        confirmation: FirestoreConfirmation,
        store: Arc<Mutex<Store>>,
        wake: Arc<(Mutex<bool>, Condvar)>,
    ) -> Result<Self, RemoteError> {
        wire::v2::validate_context(&context)?;
        if context.owner != provider.machine()
            || context.binding.reference != provider.identity().project.as_bytes()
        {
            return Err(wire::invalid());
        }
        Ok(Self {
            provider,
            context,
            confirmation,
            store,
            wake,
        })
    }
    fn exchange(&self, exchange: &InvitationExchange) -> Result<(), BackendFailure> {
        if exchange.context != self.context {
            return Err(BackendFailure::InvalidEvidence);
        }
        Ok(())
    }
    fn owner(&self) -> Result<(), BackendFailure> {
        if self.provider.uid().as_bytes() != self.context.binding.owner_principal {
            return Err(BackendFailure::Refused);
        }
        Ok(())
    }
    fn original(&self) -> Result<Committed, BackendFailure> {
        self.confirmation
            .captured(&self.context)
            .map_err(BackendFailure::Provider)?
            .ok_or(BackendFailure::InvalidEvidence)
    }
    fn routes(&self, original: &Committed) -> Result<RoutePublication, BackendFailure> {
        let routes = original
            .routes
            .as_deref()
            .ok_or(BackendFailure::InvalidEvidence)?;
        RoutePublication::read(routes, &self.context, &original.snapshot.digest)
            .map_err(BackendFailure::Provider)
    }
}
impl Backend for Firestore {
    fn binding(&self) -> BackendBinding {
        self.context.binding.clone()
    }
    fn publish(
        &self,
        publication: &Publication,
        io: &IoBudget<'_>,
    ) -> Result<WriteOutcome, BackendFailure> {
        self.exchange(&publication.exchange)?;
        io.check().map_err(|_| BackendFailure::Deadline)?;
        match (&publication.exchange.part, &publication.payload) {
            (Part::Offer, Payload::Offer { gate_hash }) => {
                self.owner()?;
                run(
                    self.provider.publish_invite_gate(
                        &self.context.offer,
                        gate_hash,
                        self.context.expires_at_ms,
                        io,
                    ),
                    io,
                )?;
            }
            (Part::Candidate, Payload::Candidate(submitted)) => {
                if submitted.candidate.principal != self.provider.uid().as_bytes() {
                    return Err(BackendFailure::Refused);
                }
                let body = wire::v2::submission(&self.context, submitted)
                    .map_err(BackendFailure::Provider)?;
                run(
                    self.provider.enrollment_create(
                        &self.context.offer,
                        self.provider.uid(),
                        "join",
                        &body,
                        Some(super::EnrollmentProof::Invite(&submitted.gate)),
                        io,
                    ),
                    io,
                )?;
            }
            (Part::Routes, Payload::Routes { snapshot, bytes }) => {
                self.owner()?;
                let original = self.original()?;
                if original.snapshot.digest != *snapshot
                    || original.routes.as_deref() != Some(bytes.as_slice())
                {
                    return Err(BackendFailure::InvalidEvidence);
                }
                self.routes(&original)?;
                // The worker consumes this signal immediately, including when
                // a wake races its current reconcile. This writes no provider row.
                let mut pending = self
                    .wake
                    .0
                    .lock()
                    .map_err(|_| BackendFailure::Unavailable)?;
                *pending = true;
                self.wake.1.notify_all();
                return Ok(WriteOutcome::Unconfirmed);
            }
            (
                Part::Receipt,
                Payload::Receipt {
                    snapshot,
                    bytes,
                    proof,
                },
            ) => {
                self.owner()?;
                let original = self.original()?;
                if original.snapshot.digest != *snapshot
                    || original.receipt != *bytes
                    || original.proof != *proof
                {
                    return Err(BackendFailure::InvalidEvidence);
                }
                let uid = std::str::from_utf8(&original.snapshot.candidate.principal)
                    .map_err(|_| BackendFailure::InvalidEvidence)?;
                let body =
                    wire::v2::receipt(snapshot, bytes, proof).map_err(BackendFailure::Provider)?;
                run(
                    self.provider.enrollment_create(
                        &format!("{}-response", self.context.offer),
                        uid,
                        "receipt",
                        &body,
                        None,
                        io,
                    ),
                    io,
                )?;
            }
            _ => return Err(BackendFailure::InvalidEvidence),
        }
        Ok(WriteOutcome::Acknowledged)
    }
    fn observe(
        &self,
        exchange: &InvitationExchange,
        io: &IoBudget<'_>,
    ) -> Result<Option<Observation>, BackendFailure> {
        self.exchange(exchange)?;
        io.check().map_err(|_| BackendFailure::Deadline)?;
        let (principal, payload) = match exchange.part {
            Part::Offer => {
                self.owner()?;
                let Some(gate) = run(self.provider.invite_gate(&self.context.offer, io), io)?
                else {
                    return Ok(None);
                };
                if gate.expires_at_ms != self.context.expires_at_ms {
                    return Err(BackendFailure::InvalidEvidence);
                }
                (
                    self.context.binding.owner_principal.clone(),
                    Payload::Offer {
                        gate_hash: gate.token_hash,
                    },
                )
            }
            Part::Candidate => {
                let Some(doc) = run(self.provider.enrollment_get(&self.context.offer, io), io)?
                else {
                    return Ok(None);
                };
                if doc.kind != "join"
                    || doc.created_ms >= self.context.expires_at_ms
                    || self.context.issued_at_ms.saturating_sub(doc.created_ms)
                        > crate::session::CLOCK_SKEW.as_millis() as u64
                {
                    return Err(BackendFailure::InvalidEvidence);
                }
                let token = doc.token.ok_or(BackendFailure::InvalidEvidence)?;
                let submitted =
                    wire::v2::read_submission(&doc.body, &self.context, &doc.uid, token)
                        .map_err(BackendFailure::Provider)?;
                (doc.uid.into_bytes(), Payload::Candidate(submitted))
            }
            Part::Routes => {
                self.owner()?;
                let original = self.original()?;
                let capture = self.routes(&original)?;
                let acknowledged = self
                    .store
                    .lock()
                    .map_err(|_| BackendFailure::Unavailable)?
                    .cloud_routes_acknowledged(&capture.rows)
                    .map_err(BackendFailure::Provider)?;
                if !acknowledged {
                    return Ok(None);
                }
                (
                    self.context.binding.owner_principal.clone(),
                    Payload::Routes {
                        snapshot: original.snapshot.digest,
                        bytes: original.routes.ok_or(BackendFailure::InvalidEvidence)?,
                    },
                )
            }
            Part::Receipt => {
                let Some(doc) = run(
                    self.provider
                        .enrollment_get(&format!("{}-response", self.context.offer), io),
                    io,
                )?
                else {
                    return Ok(None);
                };
                if doc.kind != "receipt"
                    || (self.provider.uid().as_bytes() != self.context.binding.owner_principal
                        && doc.uid != self.provider.uid())
                    || doc.created_ms >= self.context.expires_at_ms
                {
                    return Err(BackendFailure::InvalidEvidence);
                }
                let payload =
                    wire::v2::read_receipt(&doc.body).map_err(BackendFailure::Provider)?;
                (self.context.binding.owner_principal.clone(), payload)
            }
        };
        Ok(Some(Observation {
            principal,
            publication: Publication {
                exchange: exchange.clone(),
                payload,
            },
        }))
    }
    fn withdraw(
        &self,
        context: &Context,
        io: &IoBudget<'_>,
    ) -> Result<WriteOutcome, BackendFailure> {
        self.exchange(&InvitationExchange {
            context: context.clone(),
            part: Part::Offer,
        })?;
        self.owner()?;
        run(self.provider.delete_invite(&context.offer, io), io)?;
        Ok(WriteOutcome::Acknowledged)
    }
}
