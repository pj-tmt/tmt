//! Compose runtime evidence before allowing implicit host attribution.

use crate::output::Failure;
use tmt_adapters::drivers::Registry;
use tmt_core::driver::{ActionResult, caller::RuntimeCaller};

/// Automatic install offers require human terminal evidence. A marker, runtime
/// ancestor or incomplete observation refuses; this does not attribute identity.
pub fn may_offer_install(interaction: tmt_cli_style::Interaction) -> bool {
    offer_eligible(interaction, std::env::var_os("TMT_AGENT").is_some(), || {
        Registry::builtin().caller_is_human()
    })
}

fn offer_eligible(
    interaction: tmt_cli_style::Interaction,
    marked: bool,
    human: impl FnOnce() -> bool,
) -> bool {
    !interaction.json && interaction.stdin && interaction.stdout && !marked && human()
}

/// Explicit identity and endpoint selectors do not use implicit host evidence.
/// A shared runtime's inherited pane cannot authorize either a sender or a
/// binding mutation. Session correlation is not replaced by remembered history.
pub fn require_independent_host() -> Result<(), Failure> {
    require_independent_host_for(None)
}

pub fn require_independent_host_for(name: Option<&str>) -> Result<(), Failure> {
    allow_host_for(Registry::builtin().identify_caller(), name)
}

#[cfg(test)]
fn allow_host<E>(result: ActionResult<RuntimeCaller, E>) -> Result<(), Failure> {
    allow_host_for(result, None)
}

fn allow_host_for<E>(
    result: ActionResult<RuntimeCaller, E>,
    name: Option<&str>,
) -> Result<(), Failure> {
    match result {
        ActionResult::Unsupported => Ok(()),
        ActionResult::Completed(caller) if caller.permits_host_fallback() => Ok(()),
        ActionResult::Completed(caller) => {
            let error = ambiguous();
            let name = name.map(crate::output::shell_word);
            Err(
                match Registry::builtin().caller_refusal(&caller, name.as_deref()) {
                    Some(refusal) => error.with_runtime_refusal(refusal),
                    None => error,
                },
            )
        }
        ActionResult::Failed(_) => Err(ambiguous()),
    }
}

fn ambiguous() -> Failure {
    Failure::new(
            "CALLER_IDENTITY_AMBIGUOUS",
            "The runtime host does not establish which agent invoked this command.",
            1,
        ).suggestion(
            "Use --identity <name> where supported, or select the intended pane explicitly for binding operations. The inherited host pane was not used.".into(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_core::{
        binding::session::{HarnessId, ProviderSessionId},
        driver::caller::HostAttribution,
    };

    #[test]
    fn install_offer_requires_positive_human_evidence_and_both_terminal_streams() {
        let person = tmt_cli_style::Interaction {
            json: false,
            stdin: true,
            stdout: true,
            stderr: false,
            dumb: false,
        };
        assert!(offer_eligible(person, false, || true));
        assert!(!offer_eligible(person, false, || false));
        for (facts, marked) in [
            (person, true),
            (
                tmt_cli_style::Interaction {
                    json: true,
                    ..person
                },
                false,
            ),
            (
                tmt_cli_style::Interaction {
                    stdin: false,
                    ..person
                },
                false,
            ),
            (
                tmt_cli_style::Interaction {
                    stdout: false,
                    ..person
                },
                false,
            ),
        ] {
            assert!(!offer_eligible(facts, marked, || panic!(
                "automation must not scan callers"
            )));
        }
    }

    #[test]
    fn a_shared_provider_carries_its_own_reason_and_copyable_fix() {
        let caller = RuntimeCaller {
            harness: HarnessId::new(tmt_core::driver::descriptor::CODEX.name).unwrap(),
            session: None,
            host: HostAttribution::Ambiguous,
        };
        let failure = allow_host::<()>(ActionResult::Completed(caller)).unwrap_err();
        let document = failure.document();
        assert_eq!(document["error"]["code"], "CALLER_IDENTITY_AMBIGUOUS");
        assert_eq!(document["error"]["reason"], "codex-shared-server");
        assert_eq!(document["error"]["fix"], "tmt run <name> codex");
        assert!(
            failure
                .message
                .contains("cannot name itself, send as itself, or reply")
        );
        assert!(document["error"].get("suggestion").is_none());
    }

    #[test]
    fn a_requested_caller_name_is_quoted_but_a_recipient_is_not_used_as_the_fix() {
        let caller = || RuntimeCaller {
            harness: HarnessId::new(tmt_core::driver::descriptor::CODEX.name).unwrap(),
            session: None,
            host: HostAttribution::Ambiguous,
        };
        let failure = allow_host_for::<()>(ActionResult::Completed(caller()), Some("My 'Agent'"))
            .unwrap_err();
        assert_eq!(
            failure.document()["error"]["fix"],
            "tmt run 'My '\\''Agent'\\''' codex"
        );
        let independent = RuntimeCaller {
            host: HostAttribution::Independent,
            ..caller()
        };
        assert!(allow_host::<()>(ActionResult::Completed(independent)).is_ok());
    }

    #[test]
    fn uncorrelated_shared_session_and_probe_failure_cannot_authorize_host_selection() {
        let caller = RuntimeCaller {
            harness: HarnessId::new("fixture").unwrap(),
            session: Some(ProviderSessionId::new("known-conversation").unwrap()),
            host: HostAttribution::Ambiguous,
        };
        let other = allow_host::<()>(ActionResult::Completed(caller)).unwrap_err();
        assert!(!other.has_runtime_refusal());
        assert!(
            other.document()["error"]["suggestion"]
                .as_str()
                .unwrap()
                .contains("--identity")
        );
        assert!(allow_host(ActionResult::Failed("unavailable")).is_err());
        assert!(allow_host::<()>(ActionResult::Unsupported).is_ok());
    }
}
