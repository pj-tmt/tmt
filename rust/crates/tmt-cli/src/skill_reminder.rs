//! Best-effort outcome-aware human guidance, never part of command success.

use crate::invocation::{Invocation, OutputMode, Parsed};
use std::io::{self, IsTerminal, Write};
use tmt_adapters::{
    config::ConfigPaths,
    skill_installation::{ProviderEnvironment, inspect_local_drift},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    TemporaryIdentityCreated { name: String },
    SavedIdentityCreated { name: String },
}

fn optional_hint(outcome: &Outcome, mode: OutputMode, enabled: bool) -> Option<String> {
    if mode.json || !enabled {
        return None;
    }
    match outcome {
        Outcome::None => None,
        Outcome::TemporaryIdentityCreated { name } => {
            let name = crate::output::shell_word(name);
            Some(format!(
                "this temporary identity ends with its pane; keep it with tmt identity create -- {name}; use -s when binding"
            ))
        }
        Outcome::SavedIdentityCreated { name } => {
            let name = crate::output::shell_word(name);
            Some(format!(
                "receive work for this saved identity with tmt x listen --identity={name}"
            ))
        }
    }
}

fn hints_enabled() -> bool {
    !std::env::var("TMT_HINTS").is_ok_and(|value| value.eq_ignore_ascii_case("off"))
}

fn write_hint(output: &mut impl Write, terminal: tmt_cli_style::Terminal, hint: &str) {
    // Discovery is optional; a broken stderr must not change a successful
    // command result or cause a second output attempt.
    let _ = tmt_cli_style::message::hint(output, terminal, hint);
}

pub fn eligible_for_drift(parsed: &Parsed) -> bool {
    !parsed.mode.json
        && !matches!(
            parsed.invocation,
            Invocation::Extension { .. }
                | Invocation::Api
                | Invocation::Mcp { .. }
                | Invocation::Help(_)
                | Invocation::Version
                | Invocation::Completion { .. }
                | Invocation::CompletionScript(_)
                | Invocation::Complete(_)
                | Invocation::Init
                | Invocation::Run { .. }
                | Invocation::Learn { .. }
                | Invocation::Install { .. }
                | Invocation::Setup { .. }
                | Invocation::ProviderHook { .. }
                | Invocation::DigestHook { .. }
                | Invocation::Upgrade { .. }
                | Invocation::NativeInstall { .. }
                | Invocation::NativeInstallHandoff { .. }
                | Invocation::NativeSchema { .. }
                | Invocation::NativeRefreshSkills { .. }
                | Invocation::NativeUpgradeExtensions { .. }
                | Invocation::Office { .. }
                | Invocation::Identity(_)
                | Invocation::Bind { .. }
                | Invocation::BindMarked { .. }
                | Invocation::Whoami
                | Invocation::WhoamiContext
                | Invocation::Unbind
                | Invocation::Remove { .. }
                | Invocation::Rename { .. }
                | Invocation::List { .. }
        )
}

/// Commands the Herdr driver hint follows: any a person or agent runs in a
/// pane, failures included, but not machine output, completion, driver
/// management itself, or installation plumbing.
pub fn eligible_for_driver_hint(parsed: &Parsed) -> bool {
    !parsed.mode.json
        && !matches!(
            parsed.invocation,
            Invocation::Extension { .. }
                | Invocation::Api
                | Invocation::Mcp { .. }
                | Invocation::Help(_)
                | Invocation::Version
                | Invocation::Completion { .. }
                | Invocation::CompletionScript(_)
                | Invocation::Complete(_)
                | Invocation::Driver(_)
                | Invocation::ProviderHook { .. }
                | Invocation::DigestHook { .. }
                | Invocation::NativeInstall { .. }
                | Invocation::NativeInstallHandoff { .. }
                | Invocation::NativeSchema { .. }
                | Invocation::NativeRefreshSkills { .. }
                | Invocation::NativeUpgradeExtensions { .. }
        )
}

/// The hint for a Herdr pane, where no approved driver serves Herdr. Herdr
/// was a built-in host until #1082, so bindings made there wait for its
/// driver; the place is the pane on its server.
fn driver_hint(
    pane: Option<&str>,
    socket: Option<&str>,
    herdr_approved: bool,
) -> Option<(&'static str, String)> {
    let (pane, socket) = (pane?, socket?);
    if pane.is_empty() || socket.is_empty() || herdr_approved {
        return None;
    }
    Some((
        "Herdr panes need the Herdr driver: tmt driver install herdr",
        format!("{socket} {pane}"),
    ))
}

/// Shows the Herdr driver hint after a command's own result, once a day for
/// each pane, unless hints are off. Returns whether it was shown.
///
/// Transitional: reading `HERDR_PANE_ID` and `HERDR_SOCKET_PATH` here is
/// the one place core still names Herdr's environment, only to guide users
/// of the former built-in host to its driver; the driver's `callerEnv`
/// owns these variables otherwise.
pub fn present_driver_hint() -> bool {
    if !hints_enabled() {
        return false;
    }
    let pane = std::env::var("HERDR_PANE_ID").ok();
    let socket = std::env::var("HERDR_SOCKET_PATH").ok();
    if pane.is_none() || socket.is_none() {
        return false;
    }
    let herdr_approved = tmt_adapters::host::external::approved()
        .iter()
        .any(|record| record.name == "herdr");
    let Some((hint, place)) = driver_hint(pane.as_deref(), socket.as_deref(), herdr_approved)
    else {
        return false;
    };
    let Ok(paths) = ConfigPaths::discover() else {
        return false;
    };
    let day = tmt_adapters::request_runtime::wall_time_ms() / 86_400_000;
    if !tmt_adapters::hint_cadence::due_today(&paths.global_dir, "herdr-driver", &place, day) {
        return false;
    }
    let mut stderr = tmt_cli_style::stream::stderr();
    let terminal = stderr.terminal();
    write_hint(&mut stderr, terminal, hint);
    true
}

/// Exactly one line follows a successful human result. Outcome transitions
/// take precedence; passive drift is inspected only on a terminal and only
/// when no outcome hint was selected. Inspection never mutates guidance.
pub fn present(outcome: Outcome, mode: OutputMode, inspect_drift: bool) {
    if mode.json {
        return;
    }
    let mut stderr = tmt_cli_style::stream::stderr();
    let terminal = stderr.terminal();
    if let Some(hint) = optional_hint(&outcome, mode, hints_enabled()) {
        write_hint(&mut stderr, terminal, &hint);
        return;
    }
    if !inspect_drift || !io::stdin().is_terminal() || !stderr.is_terminal() {
        return;
    }
    let Ok(env) = ProviderEnvironment::capture() else {
        return;
    };
    let Ok(paths) = ConfigPaths::discover() else {
        return;
    };
    let Ok(drift) = inspect_local_drift(&env, &paths.global_dir) else {
        return;
    };
    if let Some(first) = drift.first() {
        let _ = tmt_cli_style::message::warning(
            &mut stderr,
            terminal,
            &format!(
                "Skill guidance needs inspection at {} ({} location(s)).",
                first.display(),
                drift.len()
            ),
            Some(
                "run tmt install for the intended provider; inspect conflicts before using --force, then reload the agent",
            ),
        );
    }
}

// Source-checked command samples for the printed-command guard.
#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "Herdr panes need the Herdr driver: tmt driver install herdr",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "receive work for this saved identity with tmt x listen --identity={name}",
        &[""],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "run tmt install for the intended provider; inspect conflicts before using --force, then reload the agent",
        &[" for the intended provider"],
        &[],
    ),
    crate::cli_style_tests::HintSpec::core(
        "this temporary identity ends with its pane; keep it with tmt identity create -- {name}; use -s when binding",
        &["; use -s when binding"],
        &[],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_require_real_transitions_and_human_output() {
        let human = OutputMode::default();
        assert!(optional_hint(&Outcome::None, human, true).is_none());
        for transition in [
            Outcome::TemporaryIdentityCreated {
                name: "worker".into(),
            },
            Outcome::SavedIdentityCreated {
                name: "worker".into(),
            },
        ] {
            assert!(optional_hint(&transition, human, true).is_some());
            assert!(optional_hint(&transition, human, false).is_none());
            assert!(optional_hint(&transition, OutputMode { json: true }, true).is_none());
        }
    }

    #[test]
    fn identity_hints_preserve_names_and_only_temporary_identities_offer_saving() {
        use crate::invocation::IdentityRequest;
        for name in [
            "worker",
            "Team Lead",
            "Team's Lead",
            "-worker",
            "worker; echo hello",
            "作業者",
        ] {
            let temporary = optional_hint(
                &Outcome::TemporaryIdentityCreated { name: name.into() },
                OutputMode::default(),
                true,
            )
            .unwrap();
            let saved = optional_hint(
                &Outcome::SavedIdentityCreated { name: name.into() },
                OutputMode::default(),
                true,
            )
            .unwrap();
            assert!(temporary.ends_with("; use -s when binding"));
            assert!(!saved.contains("use -s"));
            for (hint, temporary) in [(temporary, true), (saved, false)] {
                let command = hint.split_once("tmt ").unwrap().1;
                let command = if temporary {
                    command.strip_suffix("; use -s when binding").unwrap()
                } else {
                    command
                };
                let args = tmt_cli_style::help::ShownExample {
                    note: String::new(),
                    command: format!("tmt {command}"),
                }
                .argv()
                .unwrap()
                .into_iter()
                .skip(1)
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>();
                let parsed = crate::parser::parse_core(&args).unwrap();
                match parsed.invocation {
                    Invocation::Identity(IdentityRequest::Create(actual)) if temporary => {
                        assert_eq!(actual, name)
                    }
                    Invocation::Exchange {
                        identity,
                        operation: crate::invocation::ExchangeOperation::Listen { .. },
                        ..
                    } if !temporary => assert_eq!(identity.as_deref(), Some(name)),
                    invocation => panic!("wrong hint command: {invocation:?}"),
                }
            }
        }
    }

    #[test]
    fn machine_setup_and_transition_owners_skip_passive_inspection() {
        let mut parsed = Parsed {
            legacy_hook: false,
            invocation: Invocation::Whoami,
            mode: OutputMode::default(),
        };
        assert!(!eligible_for_drift(&parsed));
        parsed.invocation = Invocation::Config(crate::invocation::ConfigRequest::Show);
        assert!(eligible_for_drift(&parsed));
        parsed.mode.json = true;
        assert!(!eligible_for_drift(&parsed));
        parsed.mode.json = false;
        for invocation in [
            Invocation::Help(Vec::new()),
            Invocation::Version,
            Invocation::Completion { shell: None },
            Invocation::CompletionScript("bash".into()),
            Invocation::Init,
            Invocation::Run {
                name: "runner".into(),
                command: vec!["claude".into()],
                resume: false,
                save: false,
                channel: crate::invocation::ChannelMode::Default,
            },
            Invocation::Learn {
                skill: Some("tmt".into()),
            },
            Invocation::Learn { skill: None },
            Invocation::Install {
                target: None,
                directory: None,
                force: false,
            },
            Invocation::Upgrade {
                channel: None,
                exact: None,
                unpin: false,
                yes: false,
                allow_schema_ahead: false,
            },
            Invocation::NativeRefreshSkills { managed: false },
            Invocation::NativeUpgradeExtensions { plan: true },
            Invocation::NativeInstall {
                product: tmt_core::native_install::Product::Cli,
                archive: "archive.tar.gz".into(),
                manifest: "manifest.json".into(),
                prefix: "/explicit-prefix".into(),
                channel: tmt_core::native_install::Channel::Alpha,
                pin: tmt_core::native_install::PinAction::Preserve,
            },
        ] {
            parsed.invocation = invocation;
            assert!(!eligible_for_drift(&parsed));
        }
    }

    #[test]
    fn the_driver_hint_needs_a_herdr_pane_without_an_approved_driver() {
        let (hint, place) = driver_hint(Some("w1:p2"), Some("/s.sock"), false).unwrap();
        assert_eq!(
            hint,
            "Herdr panes need the Herdr driver: tmt driver install herdr"
        );
        assert_eq!(place, "/s.sock w1:p2");
        assert!(driver_hint(Some("w1:p2"), Some("/s.sock"), true).is_none());
        assert!(driver_hint(None, Some("/s.sock"), false).is_none());
        assert!(driver_hint(Some("w1:p2"), None, false).is_none());
        assert!(driver_hint(Some(""), Some("/s.sock"), false).is_none());
    }

    #[test]
    fn the_driver_hint_skips_machine_and_driver_commands() {
        let mut parsed = Parsed {
            legacy_hook: false,
            invocation: Invocation::Whoami,
            mode: OutputMode::default(),
        };
        assert!(eligible_for_driver_hint(&parsed));
        parsed.mode.json = true;
        assert!(!eligible_for_driver_hint(&parsed));
        parsed.mode.json = false;
        for invocation in [
            Invocation::Version,
            Invocation::Help(Vec::new()),
            Invocation::Complete(Vec::new()),
            Invocation::NativeRefreshSkills { managed: false },
        ] {
            parsed.invocation = invocation;
            assert!(!eligible_for_driver_hint(&parsed));
        }
    }

    #[test]
    fn broken_hint_stream_is_best_effort() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("closed stderr"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        write_hint(&mut Broken, tmt_cli_style::Terminal::PLAIN, "optional");
    }
}
