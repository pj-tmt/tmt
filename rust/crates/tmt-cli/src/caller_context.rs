//! Compose runtime evidence before allowing implicit host attribution.

use crate::output::Failure;
use tmt_adapters::drivers::Registry;
use tmt_core::driver::{ActionResult, caller::RuntimeCaller};

/// Explicit identity and endpoint selectors do not use implicit host evidence.
/// A shared runtime's inherited pane cannot authorize either a sender or a
/// binding mutation. Session correlation is not replaced by remembered history.
pub fn require_independent_host() -> Result<(), Failure> {
    allow_host(Registry::builtin().identify_caller())
}

fn allow_host<E>(result: ActionResult<RuntimeCaller, E>) -> Result<(), Failure> {
    match result {
        ActionResult::Unsupported => Ok(()),
        ActionResult::Completed(caller) if caller.permits_host_fallback() => Ok(()),
        ActionResult::Completed(_) | ActionResult::Failed(_) => Err(Failure::new(
            "CALLER_IDENTITY_AMBIGUOUS",
            "The runtime host does not establish which agent invoked this command.",
            1,
        ).suggestion(
            "Use --identity <name> where supported, or select the intended pane explicitly for binding operations. The inherited host pane was not used.".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_core::{
        binding::session::{HarnessId, ProviderSessionId},
        driver::caller::HostAttribution,
    };

    #[test]
    fn uncorrelated_shared_session_and_probe_failure_cannot_authorize_host_selection() {
        let caller = RuntimeCaller {
            harness: HarnessId::new("fixture").unwrap(),
            session: Some(ProviderSessionId::new("known-conversation").unwrap()),
            host: HostAttribution::Ambiguous,
        };
        assert!(allow_host::<()>(ActionResult::Completed(caller)).is_err());
        assert!(allow_host(ActionResult::Failed("unavailable")).is_err());
        assert!(allow_host::<()>(ActionResult::Unsupported).is_ok());
    }
}
