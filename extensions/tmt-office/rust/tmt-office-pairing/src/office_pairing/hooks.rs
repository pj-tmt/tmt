//! Office consumes committed lifecycle notifications; it never owns retirement.
//!
//! The companion's consumer reads pending deliveries and records attempts
//! through `tmt api`; this owner settles one pairing scope.

use super::{
    OfficeError, OfficeInstallation, PairingRecord, ProtectedEntry, RetirementFence,
    fence::settle_retirement,
};
use std::time::Instant;

/// Settles one retirement delivery under the scope lock: mark the identity,
/// revoke the scope, then `acknowledge`. A revoked record stays as the
/// secret-free receipt, so a retry after an interrupted acknowledgment repeats
/// no remote work.
pub fn settle_scope<T>(
    installation: &OfficeInstallation,
    identity_id: &str,
    reference: &str,
    fence: &dyn RetirementFence,
    deadline: Instant,
    acknowledge: impl FnOnce() -> Result<T, OfficeError>,
) -> Result<T, OfficeError> {
    installation.with_key(reference, || {
        settle_retirement(
            fence,
            identity_id,
            || revoke(installation, identity_id, reference, deadline),
            acknowledge,
        )
    })
}

fn revoke(
    installation: &OfficeInstallation,
    identity_id: &str,
    reference: &str,
    deadline: Instant,
) -> Result<(), OfficeError> {
    let entry = ProtectedEntry::open(reference)?;
    let bytes = entry.read()?.ok_or(OfficeError::CredentialsUnavailable)?;
    let target = PairingRecord::hook_target(&bytes)?;
    if installation.scope_key(&target, identity_id)? != reference {
        return Err(OfficeError::CredentialsInvalid);
    }
    let mut record = PairingRecord::decode(&bytes, &target, installation.id(), identity_id)?;
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
