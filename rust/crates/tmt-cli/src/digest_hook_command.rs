//! Synchronous provider continuation. A worker prepares a sealed checklist; only
//! its supervisor can publish feedback and then settle that exact attempt.
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths,
    digest,
    process::{
        CommandFailure, CommandRequest, CommandRunner, SupervisedProbeRunner, UnixCommandRunner,
        handoff,
    },
    request_runtime::wall_time_ms,
    response_input::read_stdin_bounded,
    runtime::{
        RuntimeRegistry,
        hook_protocol::{DigestHandoff, HOOK_INPUT_LIMIT, HookLaunch},
    },
    storage::Storage,
};
use tmt_core::{
    binding::session::{HarnessId, launch::SessionLearningDigestStop as DigestStop},
    request::{
        RequestService,
        digest::{DigestOpportunity, DigestState},
    },
};

const BUDGET: Duration = Duration::from_millis(crate::invocation::MAXIMUM_HOOK_WORK_BUDGET_MS);
const PUBLICATION_RESERVE: Duration = Duration::from_millis(600);

pub fn execute(
    provider: &str,
    scope: Option<&str>,
    worker: bool,
    work_budget_ms: Option<u64>,
    legacy_name: bool,
) -> io::Result<u8> {
    let deadline = Instant::now() + crate::invocation::hook_worker_duration(work_budget_ms);
    let mut empty_hook_output = "";
    let mut wrote_stdout = false;
    let mut delivered = false;
    // Why this Stop ends with no output, when a check refused. A Stop with no Digest due
    // is not a refusal and names nothing.
    let mut refusal: Option<DigestStop> = None;
    let mut refuse = |stop: DigestStop| {
        refusal.get_or_insert(stop);
    };
    let result = (|| -> Result<(), ()> {
        let registry = RuntimeRegistry::first_party();
        let harness = HarnessId::new(provider).map_err(|_| ())?;
        let lifecycle = registry.lifecycle(&harness).ok_or(())?;
        empty_hook_output = lifecycle.empty_hook_output();
        let deadline =
            deadline.min(Instant::now() + lifecycle.hook_work_duration().unwrap_or(BUDGET));
        let input = read_stdin_bounded(
            deadline.saturating_duration_since(Instant::now()),
            HOOK_INPUT_LIMIT,
        )
        .map_err(|_| ())?;
        let Some(session) = lifecycle.decode_digest_turn(input.as_bytes()) else {
            return Ok(());
        };
        if worker {
            crate::provider_hook_command::capture_before_digest(
                provider,
                input.as_bytes(),
                deadline,
            )
            .map_err(&mut refuse)?;
            let paths = ConfigPaths::discover().map_err(|_| ())?;
            let mut storage = Storage::open_hook(&paths.database, deadline).map_err(|_| ())?;
            let supplied = scope
                .map(serde_json::from_str::<HookLaunch>)
                .transpose()
                .map_err(|_| refuse(DigestStop::LaunchArgument))?;
            let (launch, _, stored) = crate::provider_hook_command::verified_digest_launch(
                provider,
                lifecycle,
                supplied.as_ref(),
                &session,
                deadline,
                SupervisedProbeRunner,
            )
            .map_err(&mut refuse)?;
            storage
                .ingest_pending_hook_captures_for_identity(&launch.identity_id)
                .map_err(|_| ())?;
            let Some(through) = digest::fitting_through(
                &mut storage,
                &launch.identity_id,
                DigestOpportunity::TurnBoundary,
                false,
            )
            .map_err(|_| ())?
            else {
                storage.close().map_err(|_| ())?;
                return Ok(());
            };
            let claimed = storage
                .claim_digest_for_launch(
                    &stored,
                    tmt_core::operation::new_operation_id(),
                    tmt_core::operation::new_operation_id(),
                    Some(through),
                )
                .map_err(|_| ())?;
            let batch = match claim_refusal(claimed) {
                Ok(Some(batch)) => batch,
                // Nothing to claim is not a refusal and names nothing.
                Ok(None) => {
                    storage.close().map_err(|_| ())?;
                    return Ok(());
                }
                Err(stop) => {
                    refuse(stop);
                    let _ = storage.close();
                    return Err(());
                }
            };
            let text = match digest::text(&mut storage, &batch) {
                Ok(text) => text,
                Err(_) => {
                    let _ = digest::release_unsent(&mut storage, &batch);
                    return Err(());
                }
            };
            let prepared = DigestHandoff {
                launch: launch.clone(),
                checklist_id: batch.id.clone(),
                attempt_token: batch.attempt_token.clone(),
                digest: text,
            };
            storage.close().map_err(|_| ())?;
            let bytes = serde_json::to_vec(&prepared).map_err(|_| ())?;
            // This is private worker output, never delivered evidence. A lost
            // response leaves the claim visible and grants no automatic retry.
            tmt_cli_style::stream::stdout(true)
                .write_all(&bytes)
                .map_err(|_| ())?;
            return Ok(());
        }
        let worker_deadline = deadline
            .checked_sub(PUBLICATION_RESERVE)
            .ok_or_else(|| refuse(DigestStop::WorkerTimeout))?;
        let remaining = worker_deadline
            .saturating_duration_since(Instant::now())
            .as_millis();
        if remaining == 0 {
            refuse(DigestStop::WorkerTimeout);
            return Err(());
        }
        let mut args = vec![
            "__digest-hook".into(),
            provider.into(),
            "--worker".into(),
            "--work-budget-ms".into(),
            remaining.to_string().into(),
        ];
        match scope {
            Some(scope) => args.extend(["--launch".into(), scope.into()]),
            None => args.push("--discover-launch".into()),
        }
        let executable = std::env::current_exe().map_err(|_| refuse(DigestStop::WorkerSpawn))?;
        let output = UnixCommandRunner
            .execute(CommandRequest {
                program: executable.as_os_str(),
                args: &args,
                input: input.as_bytes(),
                deadline: worker_deadline,
                max_output_bytes: digest::HANDOFF_OUTPUT_BYTES,
            })
            .map_err(|error| {
                let stderr = error.output.as_ref().map_or(&[][..], |o| &o.stderr[..]);
                refuse(worker_refusal(Some(error.kind), stderr).unwrap_or(DigestStop::WorkerError))
            })?;
        if let Some(stop) = worker_refusal(None, &output.stderr) {
            refuse(stop);
            return Err(());
        }
        if output.stdout.is_empty() {
            if legacy_name {
                let supplied = scope
                    .map(serde_json::from_str::<HookLaunch>)
                    .transpose()
                    .map_err(|_| refuse(DigestStop::LaunchArgument))?;
                let (_, paths, _) = crate::provider_hook_command::verified_digest_launch(
                    provider,
                    lifecycle,
                    supplied.as_ref(),
                    &session,
                    deadline,
                    UnixCommandRunner,
                )
                .map_err(&mut refuse)?;
                match restart_notice(lifecycle, &paths, &harness, &session, None, deadline) {
                    Ok(Some(notice)) => {
                        wrote_stdout = true;
                        let _ = handoff::write_before(
                            &tmt_cli_style::stream::stdout(true).into_inner(),
                            notice.as_bytes(),
                            deadline,
                        );
                    }
                    Ok(None) => {}
                    Err(stop) => refuse(stop),
                }
            }
            return Ok(());
        }
        let prepared: DigestHandoff = serde_json::from_slice(&output.stdout)
            .map_err(|_| refuse(DigestStop::HandoffInvalid))?;
        let launch = &prepared.launch;
        if !launch.valid()
            || scope.is_some_and(|scope| {
                serde_json::from_str::<HookLaunch>(scope).ok().as_ref() != Some(launch)
            })
        {
            refuse(DigestStop::HandoffInvalid);
            return Err(());
        }
        if !tmt_core::dispatch::canonical_id(&prepared.checklist_id)
            || !tmt_core::dispatch::canonical_id(&prepared.attempt_token)
        {
            refuse(DigestStop::HandoffInvalid);
            return Err(());
        }
        let payload = lifecycle
            .encode_digest_turn(&prepared.digest)
            .ok_or_else(|| refuse(DigestStop::HandoffInvalid))?;
        let paths = tmt_adapters::config::ConfigPaths::discover().map_err(|_| ())?;
        let state = match crate::provider_hook_command::verified_digest_launch(
            provider,
            lifecycle,
            Some(launch),
            &session,
            deadline,
            UnixCommandRunner,
        ) {
            Err(stop) => {
                refuse(stop);
                DigestState::Unsent
            }
            Ok(_) => {
                let mut storage = Storage::open_hook(&paths.database, deadline).map_err(|_| ())?;
                let active = RequestService::new(&mut storage, wall_time_ms)
                    .digest_policies(std::slice::from_ref(&launch.identity_id))
                    .map_err(|_| ())?
                    .into_iter()
                    .next()
                    .and_then(|policy| policy.active_checklist);
                storage.close().map_err(|_| ())?;
                if active.is_none_or(|batch| {
                    batch.id != prepared.checklist_id
                        || batch.attempt_token != prepared.attempt_token
                        || batch.state != DigestState::Claimed
                }) {
                    refuse(DigestStop::ChecklistChanged);
                    return Ok(());
                }
                // A notice that cannot be shown never holds back the Digest itself.
                let notice = legacy_name
                    .then(|| {
                        restart_notice(
                            lifecycle,
                            &paths,
                            &harness,
                            &session,
                            Some(&payload),
                            deadline,
                        )
                        .ok()
                        .flatten()
                    })
                    .flatten();
                let payload = notice.as_deref().unwrap_or(&payload);
                wrote_stdout = true;
                let state = publication_state(handoff::write_before(
                    &tmt_cli_style::stream::stdout(true).into_inner(),
                    payload.as_bytes(),
                    deadline,
                ));
                delivered = state == DigestState::Delivered;
                state
            }
        };
        let mut storage = Storage::open_hook(&paths.database, deadline).map_err(|_| ())?;
        RequestService::new(&mut storage, wall_time_ms)
            .settle_digest_checklist(
                &launch.identity_id,
                &prepared.checklist_id,
                &prepared.attempt_token,
                state,
            )
            .map_err(|_| ())?;
        storage.close().map_err(|_| ())?;
        Ok(())
    })();
    if result.is_err() && worker {
        // Private worker output: the supervisor reads this one name, never a person.
        if let Some(stop) = refusal {
            let _ = tmt_cli_style::stream::stderr().write_all(stop.name().as_bytes());
        }
        let _ = SupervisedProbeRunner::abort_worker_group();
    } else if !worker && !wrote_stdout && !empty_hook_output.is_empty() {
        let _ = tmt_cli_style::stream::stdout(true).write_all(empty_hook_output.as_bytes());
    }
    if !worker {
        record_outcome(
            provider,
            scope,
            (!wrote_stdout).then_some(refusal).flatten(),
            delivered,
            deadline,
        );
    }
    // Unsupported, stale or unavailable Digest never vetoes an ordinary stop.
    Ok(u8::from(worker && result.is_err()))
}

/// A claim that found the launch's authority changed refuses with the launch check it
/// failed, which the worker reports and the supervisor stores (#5459).
fn claim_refusal(
    claim: tmt_adapters::storage::LaunchClaim,
) -> Result<Option<tmt_core::request::digest::DigestChecklist>, DigestStop> {
    use tmt_adapters::storage::LaunchClaim;
    match claim {
        LaunchClaim::Claimed(batch) => Ok(Some(batch)),
        LaunchClaim::Nothing => Ok(None),
        LaunchClaim::BindingChanged => Err(DigestStop::LaunchBinding),
        LaunchClaim::RememberedChanged => Err(DigestStop::LaunchRemembered),
    }
}

/// The supervisor's reading of its worker: `failure` is `None` for a worker that finished.
/// A worker that ran and failed names its own refusal on stderr; a finished worker writes
/// none, so any stderr byte from it is a refusal of its own.
fn worker_refusal(failure: Option<CommandFailure>, stderr: &[u8]) -> Option<DigestStop> {
    Some(match failure {
        None if stderr.is_empty() => return None,
        None => DigestStop::WorkerStderr,
        Some(CommandFailure::Spawn) => DigestStop::WorkerSpawn,
        Some(CommandFailure::Timeout) => DigestStop::WorkerTimeout,
        Some(CommandFailure::Exit { .. }) => std::str::from_utf8(stderr)
            .ok()
            .and_then(DigestStop::from_worker)
            .unwrap_or(DigestStop::WorkerError),
        Some(CommandFailure::OutputLimit | CommandFailure::Io) => DigestStop::WorkerError,
    })
}

/// Stores why this Stop refused, or that it delivered, in the launch's session-learning
/// row. The launch coordinates select that row only; a hook supplied with other coordinates
/// stores nothing. Best effort within the hook's own deadline: a Stop never fails on it.
fn record_outcome(
    provider: &str,
    scope: Option<&str>,
    refusal: Option<DigestStop>,
    delivered: bool,
    deadline: Instant,
) {
    if refusal.is_none() && !delivered {
        return;
    }
    let Some(learning) = crate::session_learning::Learning::capture(provider) else {
        return;
    };
    if scope.is_some_and(|scope| {
        serde_json::from_str::<HookLaunch>(scope).ok().as_ref() != Some(&learning.launch)
    }) {
        return;
    }
    let _ = match refusal {
        Some(stop) => learning.digest_stop(stop, deadline),
        None => learning.digest_delivered(deadline),
    };
}

const RESTART_NOTICE: &str =
    "tmt: This session was started by an older tmt. Restart it to use the current hooks.";

/// The session's one restart notice, `None` when an earlier Stop already showed it.
fn restart_notice(
    lifecycle: &dyn tmt_adapters::runtime::lifecycle::RuntimeLifecycle,
    paths: &tmt_adapters::config::ConfigPaths,
    provider: &HarnessId,
    session: &tmt_core::binding::session::ProviderSessionId,
    decision: Option<&str>,
    deadline: Instant,
) -> Result<Option<String>, DigestStop> {
    use tmt_adapters::runtime::hook_notices::{NoticeClaim, claim};
    let output = lifecycle
        .encode_hook_notice(RESTART_NOTICE, decision)
        .ok_or(DigestStop::NoticeUnavailable)?;
    match claim(&paths.hook_notice_directory(), provider, session, deadline) {
        NoticeClaim::Claimed => Ok(Some(output)),
        NoticeClaim::Taken => Ok(None),
        NoticeClaim::Unavailable => Err(DigestStop::NoticeUnavailable),
    }
}

fn publication_state(result: Result<(), handoff::HandoffFailure>) -> DigestState {
    match result {
        Ok(()) => DigestState::Delivered,
        Err(error) if error.written == 0 => DigestState::Unsent,
        Err(_) => DigestState::Uncertain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_claim_refuses_only_on_an_authority_change_and_names_it() {
        use tmt_adapters::storage::LaunchClaim;
        assert!(matches!(claim_refusal(LaunchClaim::Nothing), Ok(None)));
        assert_eq!(
            claim_refusal(LaunchClaim::BindingChanged).unwrap_err(),
            DigestStop::LaunchBinding
        );
        assert_eq!(
            claim_refusal(LaunchClaim::RememberedChanged).unwrap_err(),
            DigestStop::LaunchRemembered
        );
        // The worker reports both by name, and its supervisor reads them back to store.
        for stop in [DigestStop::LaunchBinding, DigestStop::LaunchRemembered] {
            assert_eq!(
                worker_refusal(
                    Some(CommandFailure::Exit {
                        code: Some(1),
                        signal: None
                    }),
                    stop.name().as_bytes()
                ),
                Some(stop)
            );
        }
    }

    #[test]
    fn only_complete_publication_is_delivered() {
        assert_eq!(publication_state(Ok(())), DigestState::Delivered);
        assert_eq!(
            publication_state(Err(handoff::HandoffFailure { written: 0 })),
            DigestState::Unsent
        );
        assert_eq!(
            publication_state(Err(handoff::HandoffFailure { written: 1 })),
            DigestState::Uncertain
        );
    }
    #[test]
    fn the_supervisor_names_each_worker_refusal() {
        let exit = Some(CommandFailure::Exit {
            code: None,
            signal: Some(9),
        });
        assert_eq!(
            worker_refusal(None, b""),
            None,
            "a clean worker refuses nothing"
        );
        assert_eq!(worker_refusal(None, b"x"), Some(DigestStop::WorkerStderr));
        assert_eq!(
            worker_refusal(Some(CommandFailure::Spawn), b""),
            Some(DigestStop::WorkerSpawn)
        );
        assert_eq!(
            worker_refusal(Some(CommandFailure::Timeout), b""),
            Some(DigestStop::WorkerTimeout)
        );
        assert_eq!(
            worker_refusal(Some(CommandFailure::Io), b"launch:owner"),
            Some(DigestStop::WorkerError)
        );
        // The worker's own name for its refusal, and nothing it cannot report.
        assert_eq!(
            worker_refusal(exit, b"launch:owner"),
            Some(DigestStop::LaunchOwner)
        );
        assert_eq!(
            worker_refusal(exit, b"capture_failed"),
            Some(DigestStop::CaptureFailed)
        );
        for other in [
            &b""[..],
            b"worker_spawn",
            b"launch:owner\n",
            b"panicked",
            b"\xff",
        ] {
            assert_eq!(
                worker_refusal(exit, other),
                Some(DigestStop::WorkerError),
                "{other:?}"
            );
        }
    }

    #[test]
    fn an_unusable_notice_directory_is_a_refusal_and_a_shown_notice_is_not() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "tmt-hook-advisory-{}",
            tmt_core::operation::new_operation_id()
        ));
        let paths = tmt_adapters::config::ConfigPaths::resolve(&root, &root, Some(&root), None);
        let registry = RuntimeRegistry::first_party();
        let harness = HarnessId::new("claude").unwrap();
        let lifecycle = registry.lifecycle(&harness).unwrap();
        let session = tmt_core::binding::session::ProviderSessionId::new("marked").unwrap();
        let notice =
            |deadline| restart_notice(lifecycle, &paths, &harness, &session, None, deadline);
        let later = || Instant::now() + Duration::from_secs(1);
        assert!(matches!(notice(later()), Ok(Some(_))));
        // The marker is present: the notice was shown once, and that is not a refusal.
        assert_eq!(notice(later()), Ok(None));
        std::fs::set_permissions(
            paths.hook_notice_directory(),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let other = tmt_core::binding::session::ProviderSessionId::new("other").unwrap();
        assert_eq!(
            restart_notice(lifecycle, &paths, &harness, &other, None, later()),
            Err(DigestStop::NoticeUnavailable)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn provider_notice_election_is_separate_from_handoff_and_lost_output_is_not_retried() {
        use std::fs;
        let root = std::env::temp_dir().join(format!(
            "tmt-hook-advisory-{}",
            tmt_core::operation::new_operation_id()
        ));
        let paths = tmt_adapters::config::ConfigPaths::resolve(&root, &root, Some(&root), None);
        let registry = RuntimeRegistry::first_party();
        for provider in ["claude", "codex"] {
            let harness = HarnessId::new(provider).unwrap();
            let lifecycle = registry.lifecycle(&harness).unwrap();
            let session =
                tmt_core::binding::session::ProviderSessionId::new("verified-session").unwrap();
            let decision = r#"{"decision":"block","reason":"held"}"#;
            let output = restart_notice(
                lifecycle,
                &paths,
                &harness,
                &session,
                Some(decision),
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap()
            .unwrap();
            let value: serde_json::Value = serde_json::from_str(&output).unwrap();
            assert_eq!(value["decision"], "block");
            assert_eq!(value["reason"], "held");
            assert_eq!(value["systemMessage"], RESTART_NOTICE);
            let (_read, write) = nix::unistd::pipe().unwrap();
            assert_eq!(
                publication_state(handoff::write_before(
                    &write,
                    output.as_bytes(),
                    Instant::now()
                )),
                DigestState::Unsent
            );
            assert_eq!(
                restart_notice(
                    lifecycle,
                    &paths,
                    &harness,
                    &session,
                    None,
                    Instant::now() + Duration::from_secs(1)
                ),
                Ok(None)
            );
            let other =
                tmt_core::binding::session::ProviderSessionId::new("quiet-session").unwrap();
            let quiet = restart_notice(
                lifecycle,
                &paths,
                &harness,
                &other,
                None,
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&quiet).unwrap(),
                serde_json::json!({"systemMessage":RESTART_NOTICE})
            );
        }
        assert!(!paths.database.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
