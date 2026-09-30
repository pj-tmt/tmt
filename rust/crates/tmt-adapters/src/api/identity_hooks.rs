//! Consumer-scoped retirement hook admission and composition.

use super::{Fault, Request, invalid};
use crate::storage::Storage;
use serde::Deserialize;
use serde_json::json;
use tmt_core::identity_hooks::{IdentityHook, IdentityHookState, valid_hook_consumer};

/// Bound on one pending page; matches the Office consumer's batch.
const HOOK_PAGE_LIMIT: usize = 16;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HookInput {
    consumer: String,
    identity_id: String,
    reference: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HookPendingInput {
    consumer: String,
    limit: usize,
}

pub(super) fn hook(input: &[u8]) -> Result<IdentityHook, Fault> {
    let value: HookInput = serde_json::from_slice(input).map_err(|_| invalid())?;
    IdentityHook::new(&value.consumer, &value.identity_id, &value.reference).map_err(|_| invalid())
}

pub(super) fn decode_pending(input: &[u8]) -> Result<Request, Fault> {
    Ok({
        let value: HookPendingInput = serde_json::from_slice(input).map_err(|_| invalid())?;
        if !valid_hook_consumer(&value.consumer) || !(1..=HOOK_PAGE_LIMIT).contains(&value.limit) {
            return Err(invalid());
        }
        Request::HookPending {
            consumer: value.consumer,
            limit: value.limit,
        }
    })
}

pub(super) fn register(storage: &mut Storage, hook: IdentityHook) -> Result<Vec<u8>, Fault> {
    // Identities are never deleted, so existence checked here holds at
    // registration; a retired identity registers straight to pending.
    if storage
        .find_identity_by_id(hook.identity_id())
        .map_err(|_| Fault::unavailable())?
        .is_none()
    {
        return Err(Fault::new(
            "IDENTITY_NOT_FOUND",
            "The hook identity was not found.",
        ));
    }
    let state = storage
        .register_identity_hook(&hook)
        .map_err(|_| Fault::unavailable())?;
    Ok(serde_json::to_vec(&json!({"state": match state {
        IdentityHookState::Registered => "registered",
        IdentityHookState::Pending => "pending",
        IdentityHookState::Delivered => "delivered",
    }}))
    .expect("hook state"))
}

pub(super) fn pending(storage: &Storage, consumer: String, limit: usize) -> Result<Vec<u8>, Fault> {
    let hooks = storage
        .pending_identity_hooks(&consumer, limit)
        .map_err(|_| Fault::unavailable())?;
    let pending = storage
        .count_pending_identity_hooks(&consumer)
        .map_err(|_| Fault::unavailable())?;
    Ok(serde_json::to_vec(&json!({
        "hooks": hooks.iter().map(|delivery| json!({
            "identityId": delivery.hook.identity_id(),
            "reference": delivery.hook.reference(),
            "attemptCount": delivery.attempt_count,
        })).collect::<Vec<_>>(),
        "pending": pending,
    }))
    .expect("hook page"))
}

pub(super) fn attempt(storage: &mut Storage, hook: IdentityHook) -> Result<Vec<u8>, Fault> {
    require_delivery(storage, &hook)?;
    storage
        .record_identity_hook_attempt(&hook)
        .map(|recorded| serde_json::to_vec(&json!({"recorded": recorded})).expect("attempt"))
        .map_err(|_| Fault::unavailable())
}

pub(super) fn ack(storage: &mut Storage, hook: IdentityHook) -> Result<Vec<u8>, Fault> {
    require_delivery(storage, &hook)?;
    storage
        .acknowledge_identity_hook(&hook)
        .map(|acknowledged| {
            serde_json::to_vec(&json!({"acknowledged": acknowledged})).expect("ack")
        })
        .map_err(|_| Fault::unavailable())
}

/// Attempts and acknowledgments apply only to this consumer's hooks that
/// retirement queued; a delivered hook answers `false` without changing.
fn require_delivery(storage: &Storage, hook: &IdentityHook) -> Result<(), Fault> {
    match storage
        .identity_hook_state(hook)
        .map_err(|_| Fault::unavailable())?
    {
        None => Err(Fault::new(
            "HOOK_NOT_FOUND",
            "This consumer has no such identity hook.",
        )),
        Some(IdentityHookState::Registered) => Err(Fault::new(
            "HOOK_NOT_PENDING",
            "The hook's identity has not retired.",
        )),
        Some(IdentityHookState::Pending | IdentityHookState::Delivered) => Ok(()),
    }
}
