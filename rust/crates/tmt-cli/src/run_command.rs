//! Foreground command composition. Arguments belong to the child; only the
//! driver's harness ID and process ownership enter TMT storage.

use crate::{binding_error::endpoint_failure, invocation::OutputMode, output::Failure};
use std::{
    ffi::OsString,
    io,
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::{ConfigFiles, ConfigPaths},
    host::{CallerEnvironment, Host},
    process::{
        UnixCommandRunner,
        runtime::{ProcessObservation, observe_runtime_process},
    },
    runtime::RuntimeCommand,
    storage::{Storage, StorageError},
};
use tmt_core::{
    binding::session::{ObservedSessionKey, RememberedSession, SessionTransition},
    driver::{HookEvent, HookObserver, observe_driver_hook},
    endpoint::ProcessIncarnation,
};

mod channel;
mod resume;
mod run;

use run::run_bound;

#[cfg(test)]
use resume::{failure_is_trustworthy, select_command};
#[cfg(test)]
use tmt_adapters::runtime::RuntimeRegistry;
#[cfg(test)]
use tmt_core::binding::session::SessionPreferences;

struct Launch {
    command: RuntimeCommand,
    /// The exact remembered session this launch resumes, if any.
    resumed: Option<RememberedSession>,
}

/// `tmt resume <name>` and its `tmt run --resume <name>` alias.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Resume {
    /// Try a session marked stale once more.
    pub retry: bool,
}

// The extension envelope (#324/#328) will supply registered observers here.
// Foreground launch has no subscription discovery or executable event bus.
struct EmptyObserver;

impl HookObserver for EmptyObserver {
    type Error = std::convert::Infallible;
    fn observe(&mut self, _: &HookEvent<'_>) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn observe(observer: &mut impl HookObserver, event: &HookEvent<'_>) {
    if observe_driver_hook(observer, event).is_some() {
        diagnostic("a lifecycle observer failed; the recorded command outcome is unchanged.");
    }
}

fn observe_admission(
    observer: &mut impl HookObserver,
    binding_id: &str,
    key: Option<&ObservedSessionKey>,
    resumed: bool,
    already_exited: bool,
) {
    let Some(key) = key else {
        return;
    };
    observe(
        observer,
        &HookEvent::SessionStarted {
            binding_id,
            key,
            transition: if resumed {
                SessionTransition::Resumed
            } else {
                SessionTransition::Started
            },
        },
    );
    if already_exited {
        observe(observer, &HookEvent::SessionEnded { binding_id, key });
    }
}

/// A non-fatal problem while the command keeps running or has run.
fn diagnostic(message: &str) {
    warn(message, None);
}

/// The child shares stderr with `tmt run`, so each warning names `tmt` as
/// its source.
fn warn(message: &str, hint: Option<&str>) {
    let mut stderr = tmt_cli_style::stream::stderr();
    let terminal = stderr.terminal();
    // A closed diagnostic stream must not unwind and drop a live owned child.
    let _ =
        tmt_cli_style::message::warning(&mut stderr, terminal, &format!("tmt: {message}"), hint);
}

#[derive(Clone, Copy)]
struct RunRequest<'a> {
    name: &'a str,
    command: &'a [OsString],
    resume: Option<Resume>,
    save: bool,
    /// Enroll the provider's message channel for this launch.
    channel: bool,
}

fn storage_failure(error: StorageError) -> Failure {
    Failure::new(
        "IDENTITY_ERROR",
        "Could not access launch identity state.",
        1,
    )
    .caused_by(error)
}

fn incarnation(pid: u32) -> Option<ProcessIncarnation> {
    match observe_runtime_process(
        &UnixCommandRunner,
        u64::from(pid),
        Instant::now() + Duration::from_secs(3),
    ) {
        Ok(ProcessObservation::Live(value)) => Some(value),
        _ => None,
    }
}

pub fn execute(
    name: &str,
    command: &[OsString],
    resume: Option<Resume>,
    save: bool,
    channel: bool,
) -> io::Result<u8> {
    match run(RunRequest {
        name,
        command,
        resume,
        save,
        channel,
    }) {
        Ok(status) => Ok(status),
        Err(error) => error.publish(OutputMode::default()),
    }
}

fn run(request: RunRequest<'_>) -> Result<u8, Failure> {
    crate::caller_context::require_independent_host()?;
    let environment = CallerEnvironment::current();
    let host = Host::for_caller(&environment);
    let pane = host
        .caller_pane(&environment)
        .map_err(endpoint_failure)?
        .ok_or_else(|| {
            Failure::new(
                "PANE_NOT_FOUND",
                "Not running inside a resolvable tmux pane.",
                3,
            )
        })?;
    let paths = ConfigPaths::discover().map_err(Failure::from)?;
    let badge = ConfigFiles {
        paths: paths.clone(),
    }
    .load()
    .map_err(Failure::from)?
    .settings
    .pane_badge;
    let mut storage = Storage::open(&paths.database).map_err(storage_failure)?;
    let pending = run_bound(
        &mut storage,
        &paths,
        &host,
        &pane,
        request,
        badge,
        &mut EmptyObserver,
    );
    // The child's status remains authoritative after it has actually run.
    // Closing local state cannot turn a successful child into a different exit.
    if storage.close().is_err() {
        diagnostic(
            "could not close launch state cleanly; inspect identity status before retrying.",
        );
    }
    pending
}

#[cfg(test)]
mod tests {
    use super::*;
    use tmt_adapters::drivers::{
        claude::MODE_DEFAULT as CLAUDE_MODE_DEFAULT, codex::MODE_EMBEDDED as CODEX_MODE_EMBEDDED,
    };
    use tmt_core::binding::session::{
        HarnessId, ProviderSessionId, RememberedSession, RuntimeMode,
    };

    #[derive(Default)]
    struct RecordingObserver {
        events: Vec<(String, ObservedSessionKey, SessionTransition)>,
        fail: bool,
    }

    impl HookObserver for RecordingObserver {
        type Error = &'static str;
        fn observe(&mut self, event: &HookEvent<'_>) -> Result<(), Self::Error> {
            let (binding_id, key, transition) = match event {
                HookEvent::SessionStarted {
                    binding_id,
                    key,
                    transition,
                } => (*binding_id, *key, *transition),
                HookEvent::SessionEnded { binding_id, key } => {
                    (*binding_id, *key, SessionTransition::Ended)
                }
                _ => panic!("unexpected foreground event"),
            };
            self.events
                .push((binding_id.into(), key.clone(), transition));
            if self.fail {
                Err("fixture observer failure")
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn only_recorded_admission_emits_and_fast_exit_keeps_event_order_on_observer_failure() {
        let key = ObservedSessionKey {
            incarnation: ProcessIncarnation::new(17, "fixture-child-start").unwrap(),
            provider_session: Some(ProviderSessionId::new("provider-key").unwrap()),
        };
        let mut observer = RecordingObserver {
            fail: true,
            ..Default::default()
        };
        observe_admission(&mut observer, "binding", None, false, false);
        assert!(observer.events.is_empty());
        observe_admission(&mut observer, "binding", Some(&key), false, true);
        assert_eq!(
            observer.events,
            vec![
                ("binding".into(), key.clone(), SessionTransition::Started),
                ("binding".into(), key.clone(), SessionTransition::Ended),
            ]
        );
        observer.events.clear();
        observe_admission(&mut observer, "binding", Some(&key), true, false);
        assert_eq!(
            observer.events,
            vec![("binding".into(), key, SessionTransition::Resumed)]
        );
    }

    fn preferences(harness: &str, mode: &str, session: &str) -> SessionPreferences {
        SessionPreferences {
            preferred_harness: Some(HarnessId::new(harness).unwrap()),
            remembered: Some(RememberedSession {
                harness: HarnessId::new(harness).unwrap(),
                mode: RuntimeMode::new(mode).unwrap(),
                provider_session: ProviderSessionId::new(session).unwrap(),
                state: None,
                stale_at_ms: None,
                resume_pending_at_ms: None,
            }),
        }
    }

    const SESSION: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const RESUME: Option<Resume> = Some(Resume { retry: false });

    fn refused(result: Result<Launch, Failure>) -> (String, String) {
        let error = result.err().expect("resume must not launch");
        (error.code.to_string(), error.message.to_string())
    }

    #[test]
    fn launch_selection_has_no_implicit_resume_or_argument_replay() {
        let mut registry = RuntimeRegistry::first_party();
        let preferences = preferences("claude", CLAUDE_MODE_DEFAULT, SESSION);
        let explicit = vec![
            OsString::from("/bin/sh"),
            "-c".into(),
            "printf secret".into(),
        ];
        let launch = select_command(&mut registry, "Ann", &explicit, None, &preferences).unwrap();
        assert_eq!(launch.command, RuntimeCommand::verbatim(&explicit).unwrap());
        assert!(launch.resumed.is_none());
        let launch = select_command(&mut registry, "Ann", &[], None, &preferences).unwrap();
        assert_eq!(launch.command.executable, "claude");
        assert!(launch.command.args.is_empty());
        assert!(launch.resumed.is_none());
        assert!(
            select_command(
                &mut registry,
                "Ann",
                &[],
                None,
                &SessionPreferences::default()
            )
            .is_err()
        );
    }

    #[test]
    fn resume_launches_only_the_exact_session_and_never_starts_fresh() {
        let mut registry = RuntimeRegistry::first_party();
        let mut preferences = preferences("codex", CODEX_MODE_EMBEDDED, SESSION);
        let launch = select_command(&mut registry, "Ann", &[], RESUME, &preferences).unwrap();
        assert_eq!(launch.resumed.as_ref(), preferences.remembered.as_ref());
        assert_eq!(
            launch.command.args,
            ["resume", SESSION, "--no-daemon"].map(OsString::from)
        );
        preferences.remembered.as_mut().unwrap().mode = RuntimeMode::new("unsupported").unwrap();
        let (code, message) = refused(select_command(
            &mut registry,
            "Ann",
            &[],
            RESUME,
            &preferences,
        ));
        assert_eq!(code, "RESUME_UNSUPPORTED");
        assert!(
            message.ends_with("Start fresh with: tmt run Ann"),
            "{message}"
        );
        preferences.remembered.as_mut().unwrap().mode =
            RuntimeMode::new(CODEX_MODE_EMBEDDED).unwrap();
        preferences.remembered.as_mut().unwrap().provider_session =
            ProviderSessionId::new("not-a-uuid").unwrap();
        assert_eq!(
            refused(select_command(
                &mut registry,
                "Ann",
                &[],
                RESUME,
                &preferences
            ))
            .0,
            "RESUME_UNAVAILABLE"
        );
        preferences.remembered = None;
        let (code, message) = refused(select_command(
            &mut registry,
            "Ann",
            &[],
            RESUME,
            &preferences,
        ));
        assert_eq!(code, "RESUME_UNAVAILABLE");
        assert_eq!(
            message,
            "Ann has no remembered session to resume. Start fresh with: tmt run Ann"
        );
    }

    #[test]
    fn a_stale_session_is_refused_until_the_user_retries() {
        let mut registry = RuntimeRegistry::first_party();
        let mut preferences = preferences("claude", CLAUDE_MODE_DEFAULT, SESSION);
        preferences.remembered.as_mut().unwrap().stale_at_ms = Some(1);
        let (code, message) = refused(select_command(
            &mut registry,
            "Ann",
            &[],
            RESUME,
            &preferences,
        ));
        assert_eq!(code, "RESUME_UNAVAILABLE");
        assert!(message.contains("tmt resume --forget Ann"), "{message}");
        assert!(message.contains("tmt resume --retry Ann"), "{message}");
        let launch = select_command(
            &mut registry,
            "Ann",
            &[],
            Some(Resume { retry: true }),
            &preferences,
        )
        .unwrap();
        assert_eq!(
            launch.command.args,
            ["--resume", SESSION].map(OsString::from)
        );
    }

    #[test]
    fn an_unregistered_session_driver_is_reported_not_replaced() {
        let mut registry = RuntimeRegistry::first_party();
        let mut preferences = preferences("missing-driver", "default", "provider-session");
        preferences.preferred_harness = Some(HarnessId::new("claude").unwrap());
        assert_eq!(
            refused(select_command(
                &mut registry,
                "Ann",
                &[],
                RESUME,
                &preferences
            ))
            .0,
            "RESUME_UNSUPPORTED"
        );
    }

    #[test]
    fn only_an_unsignalled_failure_with_hooks_installed_can_mark_a_session_stale() {
        assert!(failure_is_trustworthy(1, true));
        assert!(!failure_is_trustworthy(0, true), "a clean exit");
        assert!(!failure_is_trustworthy(130, true), "Ctrl-C (128 + SIGINT)");
        assert!(!failure_is_trustworthy(143, true), "SIGTERM");
        assert!(!failure_is_trustworthy(1, false), "hooks absent at launch");
    }
}
