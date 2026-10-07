//! The correlation ledger of one channel generation. Both ends keep the same one:
//! the extension issues requests, Remote answers them and issues callbacks, the
//! extension answers those. Every frame, sent or received, is applied to it before it
//! is sent or queued, so a duplicate, stale, mismatched or out-of-order frame is
//! refused here and can never satisfy a successor.
//!
//! Identifiers are strictly increasing per issuer with a remembered high-water mark
//! (no reuse, no wrap), outstanding entries are bounded, and a request has at most one
//! callback outstanding. A request ends only when it has no callback outstanding.
use super::Role;
use crate::{AdmitInput, Call, Frame, Method, Uuid4};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

/// Why a frame was refused by the ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// This end may not send this kind, or the peer may not have.
    Direction,
    /// An identifier is not above everything its issuer already used.
    Order,
    /// No outstanding entry has this identifier.
    Unknown,
    /// The entry exists but names another operation, transfer or request.
    Mismatch,
    /// A callback is already outstanding for the request, or still is.
    Busy,
    /// A bound on outstanding entries is full.
    Capacity,
}

/// Bounds on outstanding entries of one channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub requests: usize,
    pub callbacks: usize,
}
impl Caps {
    pub const fn contract() -> Self {
        Self {
            requests: 8,
            callbacks: 8,
        }
    }
}

/// Outstanding entries shared by every channel of one installation. A holder returns
/// its entries when it ends, so a closed channel never keeps a slot.
#[derive(Clone, Debug)]
pub struct Budget {
    requests: Arc<(AtomicUsize, usize)>,
    callbacks: Arc<(AtomicUsize, usize)>,
}
impl Budget {
    pub fn new(requests: usize, callbacks: usize) -> Self {
        Self {
            requests: Arc::new((AtomicUsize::new(0), requests)),
            callbacks: Arc::new((AtomicUsize::new(0), callbacks)),
        }
    }
    /// The contract's installation bounds: 32 requests and 32 callbacks.
    pub fn contract() -> Self {
        Self::new(32, 32)
    }
    fn take(slots: &(AtomicUsize, usize)) -> bool {
        slots
            .0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                (used < slots.1).then_some(used + 1)
            })
            .is_ok()
    }
    fn give(slots: &(AtomicUsize, usize)) {
        slots.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Debug)]
struct Open {
    method: Method,
    transfer: Option<Uuid4>,
    /// The one callback outstanding for this request.
    callback: Option<u64>,
}

#[derive(Debug)]
pub(super) struct Ledger {
    role: Role,
    limits: Caps,
    budget: Option<Budget>,
    requests: BTreeMap<u64, Open>,
    /// Outstanding callback id -> its request id.
    callbacks: BTreeMap<u64, u64>,
    request_high: u64,
    callback_high: u64,
}
impl Ledger {
    pub(super) fn new(role: Role, limits: Caps, budget: Option<Budget>) -> Self {
        Self {
            role,
            limits,
            budget,
            requests: BTreeMap::new(),
            callbacks: BTreeMap::new(),
            request_high: 0,
            callback_high: 0,
        }
    }

    /// Apply `frame`, which this end is `sending` or has received. On refusal nothing
    /// changes.
    pub(super) fn apply(&mut self, frame: &Frame, sending: bool) -> Result<(), Reason> {
        // Remote sends results, callbacks and lifecycle; the extension sends requests
        // and admissions. Receiving is the mirror image.
        let from_remote = (self.role == Role::Remote) == sending;
        let allowed = match frame {
            Frame::Result(_) | Frame::Admit(_) | Frame::OriginState(_) => from_remote,
            Frame::Request(_) | Frame::Admission(_) => !from_remote,
        };
        if !allowed {
            return Err(Reason::Direction);
        }
        match frame {
            Frame::Request(request) => {
                let id = request.request_id.get();
                if id <= self.request_high {
                    return Err(Reason::Order);
                }
                if self.requests.len() >= self.limits.requests
                    || !self
                        .budget
                        .as_ref()
                        .is_none_or(|b| Budget::take(&b.requests))
                {
                    return Err(Reason::Capacity);
                }
                self.request_high = id;
                let transfer = match &request.call {
                    Call::Begin(input) => Some(input.transfer_id),
                    Call::Part(input) => Some(input.transfer_id),
                    Call::Commit(input) | Call::Discard(input) => Some(input.transfer_id),
                    Call::Status(input) => Some(input.transfer_id),
                    Call::Config(_) | Call::Read(_) => None,
                };
                self.requests.insert(
                    id,
                    Open {
                        method: request.call.method(),
                        transfer,
                        callback: None,
                    },
                );
            }
            Frame::Admit(admit) => {
                let callback = admit.callback_id.get();
                let id = admit.request_id.get();
                let open = self.requests.get(&id).ok_or(Reason::Unknown)?;
                if open.method != admit.operation.input.method()
                    || admitted_transfer(&admit.operation.input)
                        .is_some_and(|transfer| open.transfer != Some(transfer))
                {
                    return Err(Reason::Mismatch);
                }
                if open.callback.is_some() {
                    return Err(Reason::Busy);
                }
                if callback <= self.callback_high {
                    return Err(Reason::Order);
                }
                if self.callbacks.len() >= self.limits.callbacks
                    || !self
                        .budget
                        .as_ref()
                        .is_none_or(|b| Budget::take(&b.callbacks))
                {
                    return Err(Reason::Capacity);
                }
                self.callback_high = callback;
                self.callbacks.insert(callback, id);
                if let Some(open) = self.requests.get_mut(&id) {
                    open.callback = Some(callback);
                }
            }
            Frame::Admission(admission) => {
                let callback = admission.callback_id.get();
                let owner = *self.callbacks.get(&callback).ok_or(Reason::Unknown)?;
                if owner != admission.request_id.get() {
                    return Err(Reason::Mismatch);
                }
                self.callbacks.remove(&callback);
                if let Some(budget) = &self.budget {
                    Budget::give(&budget.callbacks);
                }
                if let Some(open) = self.requests.get_mut(&owner) {
                    open.callback = None;
                }
            }
            Frame::Result(result) => {
                let id = result.request_id.get();
                let open = self.requests.get(&id).ok_or(Reason::Unknown)?;
                if open.method != result.method || open.transfer != result.transfer_id {
                    return Err(Reason::Mismatch);
                }
                if open.callback.is_some() {
                    return Err(Reason::Busy);
                }
                self.requests.remove(&id);
                if let Some(budget) = &self.budget {
                    Budget::give(&budget.requests);
                }
            }
            Frame::OriginState(_) => {}
        }
        Ok(())
    }
}
impl Drop for Ledger {
    fn drop(&mut self) {
        if let Some(budget) = &self.budget {
            for _ in 0..self.requests.len() {
                Budget::give(&budget.requests);
            }
            for _ in 0..self.callbacks.len() {
                Budget::give(&budget.callbacks);
            }
        }
    }
}

/// The transfer an admit names, when its operation has one.
fn admitted_transfer(input: &AdmitInput) -> Option<Uuid4> {
    match input {
        AdmitInput::Begin(value) => Some(value.transfer_id),
        AdmitInput::Part(value) => Some(value.transfer_id),
        AdmitInput::Commit(value) | AdmitInput::Discard(value) => Some(value.transfer_id),
        AdmitInput::Status(value) => Some(value.transfer_id),
        AdmitInput::Config(_) | AdmitInput::Read(_) => None,
    }
}

#[cfg(test)]
mod tests;
