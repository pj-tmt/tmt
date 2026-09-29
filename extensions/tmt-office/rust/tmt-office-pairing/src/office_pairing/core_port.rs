//! The core reads and registration pairing needs, supplied by the companion
//! over public core commands and `tmt api`. Pairing never opens core storage.

use super::OfficeError;

/// An active core identity, as pairing records and approvals name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingIdentity {
    pub id: String,
    pub name: String,
}

pub trait PairingCore {
    /// The active identity with exactly this UUID; `None` when it is missing
    /// or retired.
    fn active_identity(&self, identity_id: &str) -> Result<Option<PairingIdentity>, OfficeError>;
    /// Registers this pairing scope for retirement delivery. Registering after
    /// retirement queues the delivery at once.
    fn register_retirement_hook(
        &self,
        identity_id: &str,
        reference: &str,
    ) -> Result<(), OfficeError>;
}
