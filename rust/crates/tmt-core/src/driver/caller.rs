//! Provider-neutral caller evidence. A session hint is not binding authority.

use crate::binding::session::{HarnessId, ProviderSessionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostAttribution {
    /// The runtime driver permits independent host-driver verification.
    Independent,
    /// A shared or unverified runtime host cannot identify its conversation by pane.
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCaller {
    pub harness: HarnessId,
    pub session: Option<ProviderSessionId>,
    pub host: HostAttribution,
}

impl RuntimeCaller {
    /// Missing correlation never turns a shared-host address into an identity.
    /// Even permitted fallback still requires the host driver's own verification.
    pub fn permits_host_fallback(&self) -> bool {
        self.host == HostAttribution::Independent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_hint_does_not_authorize_shared_host_fallback() {
        for session in [None, Some(ProviderSessionId::new("conversation").unwrap())] {
            let mut caller = RuntimeCaller {
                harness: HarnessId::new("fixture").unwrap(),
                session,
                host: HostAttribution::Ambiguous,
            };
            assert!(!caller.permits_host_fallback());
            caller.host = HostAttribution::Independent;
            assert!(caller.permits_host_fallback());
        }
    }
}
