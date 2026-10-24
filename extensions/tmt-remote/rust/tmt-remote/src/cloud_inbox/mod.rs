//! Firestore agent-carrier binding. Transport evidence grants no operation authority.
use crate::{error::RemoteError, limits};
mod document;
mod driver;
mod ingress;
mod provider;
mod sender;
mod store;
pub use driver::{InboxDriver, poll_delay};
pub use provider::Provider;
pub use sender::Sender;
pub mod rules;
pub use ingress::Receiver;
pub(crate) use ingress::Verified;

/// Refuse an oversized signed envelope before encryption or provider work.
pub fn check_envelope_size(bytes: usize) -> Result<(), RemoteError> {
    if bytes > limits::CLOUD_ENVELOPE_BYTES {
        return Err(RemoteError::new(
            "REMOTE_CLOUD_MESSAGE_TOO_LARGE",
            "The message exceeds the cloud transport limit.",
        ));
    }
    Ok(())
}

/// One stable native peer's provider create-time checkpoint. An overlap is observed again;
/// only the existing journal decides whether a logical operation has already been adopted.
#[derive(Debug, Clone, Copy)]
pub struct Checkpoint(u64);
impl Checkpoint {
    pub fn new(created_at_ms: u64) -> Result<Self, RemoteError> {
        if created_at_ms > crate::cloud_hpke::INTEGER_MAX {
            return Err(RemoteError::new(
                "REMOTE_CLOUD_CHECKPOINT_INVALID",
                "Invalid cloud checkpoint.",
            ));
        }
        Ok(Self(created_at_ms))
    }
    pub fn created_at_ms(self) -> u64 {
        self.0
    }
    pub fn query_from_ms(self) -> u64 {
        self.0.saturating_sub(limits::CLOUD_CHECKPOINT_OVERLAP_MS)
    }
    /// Call only after the full bounded provider page has a durable adopted outcome.
    pub fn completed(&mut self, created_at_ms: u64) -> Result<(), RemoteError> {
        self.0 = self.0.max(Self::new(created_at_ms)?.0);
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Replay {
    Fresh,
    Pending,
    Frozen(Vec<u8>),
    Refused(Option<Vec<u8>>),
}
#[cfg(test)]
mod tests;

/// Cloud freshness is measured at the provider commit, never at owner wake-up.
/// Caller obtains createTime only from an admitted ordinary-user provider document.
pub fn check_freshness(envelope_ms: u64, created_ms: u64, now: u64) -> Result<(), RemoteError> {
    Checkpoint::new(created_ms)?;
    if now.saturating_sub(created_ms) >= crate::journal::RECOVERY {
        return Err(RemoteError::new(
            "REMOTE_CLOUD_MESSAGE_EXPIRED",
            "The cloud message has expired; it will not be sent again.",
        ));
    }
    if envelope_ms.abs_diff(created_ms) > crate::session::CLOCK_SKEW.as_millis() as u64 {
        return Err(RemoteError::new(
            "REMOTE_CLOUD_TIMESTAMP_INVALID",
            "The message timestamp does not match its provider creation time.",
        ));
    }
    Ok(())
}

/// Core's immutable request creation time survives every transport context/restart.
pub fn check_request_age(created_at_ms: u64, now: u64) -> Result<(), RemoteError> {
    Checkpoint::new(created_at_ms)?;
    if now.saturating_sub(created_at_ms) >= crate::journal::RECOVERY {
        return Err(RemoteError::new(
            "REMOTE_CLOUD_MESSAGE_EXPIRED",
            "The cloud message has expired; it will not be sent again.",
        ));
    }
    if created_at_ms.saturating_sub(now) > crate::session::CLOCK_SKEW.as_millis() as u64 {
        return Err(RemoteError::new(
            "REMOTE_CLOUD_TIMESTAMP_INVALID",
            "The request creation time is outside the clock-skew bound.",
        ));
    }
    Ok(())
}

/// Production sender entropy belongs to the driver, not the pure HPKE library.
pub fn seal_native(
    exact: &[u8],
    descriptor: &crate::cloud_hpke::VerifiedDescriptor,
    now: u64,
    peer: crate::cloud_hpke::Sender<'_>,
    sign: impl FnOnce(&[u8]) -> [u8; 64],
) -> Result<(Vec<u8>, crate::cloud_hpke::ResponseOpener), RemoteError> {
    seal_with_entropy(exact, descriptor, now, peer, sign, |seed| {
        getrandom::fill(seed).map_err(|_| ())
    })
}
fn seal_with_entropy(
    exact: &[u8],
    descriptor: &crate::cloud_hpke::VerifiedDescriptor,
    now: u64,
    peer: crate::cloud_hpke::Sender<'_>,
    sign: impl FnOnce(&[u8]) -> [u8; 64],
    entropy: impl FnOnce(&mut [u8; 32]) -> Result<(), ()>,
) -> Result<(Vec<u8>, crate::cloud_hpke::ResponseOpener), RemoteError> {
    use rand::SeedableRng;
    check_envelope_size(exact.len())?;
    let mut seed = [0; 32];
    if entropy(&mut seed).is_err() {
        seed.fill(0);
        return Err(RemoteError::new(
            "REMOTE_ENTROPY",
            "Could not obtain key entropy.",
        ));
    }
    let mut rng = rand::rngs::StdRng::from_seed(seed);
    // Best effort: the compiler may retain copies; no long-lived/thread-local RNG exists.
    seed.fill(0);
    crate::cloud_hpke::seal(
        exact,
        limits::CLOUD_ENVELOPE_BYTES,
        descriptor,
        now,
        peer,
        sign,
        &mut rng,
    )
    .map_err(|error| RemoteError::new(error.code(), &error.to_string()))
}
