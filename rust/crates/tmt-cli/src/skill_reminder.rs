//! Best-effort outcome-aware human guidance, never part of command success.

use crate::invocation::{Invocation, OutputMode, Parsed};
use std::io::{self, IsTerminal, Write};
use tmt_adapters::{
    config::ConfigPaths,
    skill_installation::{ProviderEnvironment, inspect_local_drift},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    TemporaryIdentityCreated,
    SavedIdentityCreated,
}

fn optional_hint(outcome: Outcome, mode: OutputMode, enabled: bool) -> Option<&'static str> {
    if mode.json || !enabled {
        return None;
    }
    match outcome {
        Outcome::None => None,
        Outcome::TemporaryIdentityCreated => Some(
            "this temporary identity ends with its pane; keep it with tmt identity create <name>",
        ),
        Outcome::SavedIdentityCreated => {
            Some("receive work for this saved identity with tmt x listen --identity <name>")
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
                | Invocation::Help(_)
                | Invocation::Version
                | Invocation::Completion(_)
                | Invocation::Complete(_)
                | Invocation::Init
                | Invocation::Run { .. }
                | Invocation::Learn { .. }
                | Invocation::Install { .. }
                | Invocation::Setup { .. }
                | Invocation::ProviderHook { .. }
                | Invocation::Upgrade { .. }
                | Invocation::NativeInstall { .. }
                | Invocation::NativeRefreshSkills
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

/// Exactly one line follows a successful human result. Outcome transitions
/// take precedence; passive drift is inspected only on a terminal and only
/// when no outcome hint was selected. Inspection never mutates guidance.
pub fn present(outcome: Outcome, mode: OutputMode, inspect_drift: bool) {
    if mode.json {
        return;
    }
    let mut stderr = tmt_cli_style::stream::stderr();
    let terminal = stderr.terminal();
    if let Some(hint) = optional_hint(outcome, mode, hints_enabled()) {
        write_hint(&mut stderr, terminal, hint);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_require_real_transitions_and_human_output() {
        let human = OutputMode::default();
        assert!(optional_hint(Outcome::None, human, true).is_none());
        for transition in [
            Outcome::TemporaryIdentityCreated,
            Outcome::SavedIdentityCreated,
        ] {
            assert!(optional_hint(transition, human, true).is_some());
            assert!(optional_hint(transition, human, false).is_none());
            assert!(optional_hint(transition, OutputMode { json: true }, true).is_none());
        }
    }

    #[test]
    fn machine_setup_and_transition_owners_skip_passive_inspection() {
        let mut parsed = Parsed {
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
            Invocation::Completion(None),
            Invocation::Init,
            Invocation::Run {
                name: "runner".into(),
                command: vec!["claude".into()],
                resume: false,
                save: false,
            },
            Invocation::Learn {
                skill: Some("tmux-team".into()),
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
            },
            Invocation::NativeRefreshSkills,
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
