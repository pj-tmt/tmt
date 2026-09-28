//! Office consumes committed lifecycle notifications; it never owns retirement.

use super::{OfficeError, OfficeInstallation, PairingRecord, ProtectedEntry};
use crate::{config::ConfigPaths, storage::Storage};
use std::{io, time::Instant};
use tmt_core::identity_hooks::IdentityHook;
use tmt_office_model::office_protocol::OFFICE_HOOK_BATCH_LIMIT;
use tmt_office_model::office_protocol::OfficeSyncReport;

const CONSUMER: &str = "tmt-office";

pub(super) fn register(paths: &ConfigPaths, identity: &str, key: &str) -> Result<(), OfficeError> {
    let hook =
        IdentityHook::new(CONSUMER, identity, key).map_err(|_| OfficeError::CredentialsInvalid)?;
    let mut storage = Storage::open(&paths.database).map_err(unavailable)?;
    storage.register_identity_hook(&hook).map_err(unavailable)?;
    storage.close().map_err(unavailable)
}

pub(super) fn sync(
    paths: &ConfigPaths,
    deadline: Instant,
) -> Result<OfficeSyncReport, OfficeError> {
    // No prior local identity database means there can be no registered hooks.
    match std::fs::symlink_metadata(&paths.database) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(OfficeSyncReport::default());
        }
        Err(_) => return Err(OfficeError::CredentialsUnavailable),
        Ok(_) => {}
    }
    let mut storage = Storage::open(&paths.database).map_err(unavailable)?;
    let pending = storage
        .pending_identity_hooks(CONSUMER, OFFICE_HOOK_BATCH_LIMIT)
        .map_err(unavailable)?;
    let mut report = OfficeSyncReport::default();
    if !pending.is_empty() {
        let installation =
            OfficeInstallation::open(paths, false)?.ok_or(OfficeError::CredentialsUnavailable)?;
        for delivery in pending {
            if Instant::now() >= deadline {
                break;
            }
            let hook = delivery.hook;
            // Record the attempt before external effects. Contended/failed work
            // rotates behind less-attempted items instead of starving the queue.
            if !storage
                .record_identity_hook_attempt(&hook)
                .map_err(unavailable)?
            {
                continue;
            }
            let result = installation.with_key(hook.reference(), || {
                deliver(&installation, &hook, deadline)?;
                storage
                    .acknowledge_identity_hook(&hook)
                    .map_err(unavailable)
            });
            match result {
                Ok(true) => report.completed += 1,
                Ok(false) => {}
                Err(error) => {
                    report.failed += 1;
                    report.failure.get_or_insert(error);
                }
            }
        }
    }
    report.pending = storage
        .count_pending_identity_hooks(CONSUMER)
        .map_err(unavailable)?;
    storage.close().map_err(unavailable)?;
    Ok(report)
}

fn deliver(
    installation: &OfficeInstallation,
    hook: &IdentityHook,
    deadline: Instant,
) -> Result<(), OfficeError> {
    let entry = ProtectedEntry::open(hook.reference())?;
    let bytes = entry.read()?.ok_or(OfficeError::CredentialsUnavailable)?;
    let target = PairingRecord::hook_target(&bytes)?;
    if installation.scope_key(&target, hook.identity_id())? != hook.reference() {
        return Err(OfficeError::CredentialsInvalid);
    }
    let mut record = PairingRecord::decode(&bytes, &target, installation.id(), hook.identity_id())?;
    if record.has_credentials()
        && record.refresh_if_needed(&target, super::invocation::now_ms()?, deadline)?
    {
        // A retired identity can refresh only here for authority reduction. The
        // normal resource path still requires an active UUID before each effect.
        entry.write(&record.encode()?)?;
    }
    record.revoke(&target, deadline)?;
    entry.write(&record.encode()?)?;
    Ok(())
}

fn unavailable(_: impl std::error::Error) -> OfficeError {
    OfficeError::CredentialsUnavailable
}
