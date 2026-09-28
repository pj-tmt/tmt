//! Native approval/proof codec. Secret-bearing values deliberately have no Debug.

mod fence;
mod hooks;
mod invocation;
mod local;
mod record;
mod remote;
mod vault;
mod wire;

pub use fence::RetirementFence;
pub use invocation::execute;
pub use local::OfficeInstallation;
pub use record::PairingRecord;
pub use remote::{AgentCredential, claim_pairing};
use tmt_office_model::office_protocol::OfficeError;
pub use vault::ProtectedEntry;
pub use wire::{Approval, Claim, Proof};
