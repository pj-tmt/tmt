//! Compose runtime evidence before allowing implicit host attribution.

#[cfg(test)]
pub(crate) mod testing;

use crate::output::Failure;
use tmt_adapters::{
    config::ConfigPaths,
    drivers::Registry,
    host::{CallerEnvironment, Host, HostError},
};
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

/// The caller's host for an ordinary command. A selection read that fails to
/// clean up is an error: a process it started may still be running, so naming
/// tmux would be a guess about where the caller is. Doubt keeps tmux.
pub fn host(environment: &CallerEnvironment) -> Result<Host, Failure> {
    Host::for_caller(environment).map_err(selection_failed)
}

/// [`host`] for workspace recovery: an explicit socket selects the built-in
/// host and reads no caller evidence.
pub fn workspace_host(
    environment: &CallerEnvironment,
    explicit_socket: bool,
) -> Result<Host, Failure> {
    Host::for_workspace(environment, explicit_socket).map_err(selection_failed)
}

/// The failure for a host selection whose read could not clean up. Nothing
/// ran, so running the command again is safe.
pub fn selection_failed(error: HostError) -> Failure {
    Failure::new(
        "CLEANUP_ERROR",
        "Could not tell which host this command runs in: a process started to look for the caller's pane could not be stopped. Nothing was changed; run the command again.",
        1,
    )
    .caused_by(error)
}

/// How a command selects the caller's host: [`Host::for_caller`] in a
/// command, a test's own selection in a test, so the test runs no real read.
pub type SelectCaller<'a> = &'a dyn Fn(&CallerEnvironment) -> Result<Host, HostError>;

/// What a command takes from its process: the caller's host selection and
/// its configuration paths. A test passes its own, so it runs no real
/// selection and opens only a private store.
#[derive(Clone, Copy)]
pub struct Composition<'a> {
    pub select: SelectCaller<'a>,
    pub config: &'a dyn Fn() -> Result<ConfigPaths, Failure>,
}

impl Composition<'_> {
    pub fn process() -> Composition<'static> {
        Composition {
            select: &Host::for_caller,
            config: &discovered,
        }
    }

    /// Only a caller-scoped operation selects the caller's host. A selection
    /// read that fails to clean up stops it with `CLEANUP_ERROR` (#4449).
    pub fn caller(&self, environment: &CallerEnvironment) -> Result<Host, Failure> {
        (self.select)(environment).map_err(selection_failed)
    }

    pub fn paths(&self) -> Result<ConfigPaths, Failure> {
        (self.config)()
    }
}

fn discovered() -> Result<ConfigPaths, Failure> {
    ConfigPaths::discover().map_err(Failure::from)
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

    #[test]
    fn a_failed_host_selection_cleanup_is_a_retryable_cleanup_error() {
        let failure = selection_failed(HostError::Unavailable("probe survived".into()));
        assert_eq!(
            (failure.code.as_str(), failure.status),
            ("CLEANUP_ERROR", 1)
        );
        let message = failure.document()["error"]["message"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            message.contains("could not be stopped") && message.contains("run the command again"),
            "{message}"
        );
        assert!(
            !message.contains("probe survived"),
            "the cause is not the message"
        );
    }
}
