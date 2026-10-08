//! Synchronous provider continuation. A worker prepares a sealed checklist; only
//! its supervisor can publish feedback and then settle that exact attempt.
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};
use tmt_adapters::{
    focus,
    process::{CommandRequest, CommandRunner, SupervisedProbeRunner, UnixCommandRunner, handoff},
    request_runtime::wall_time_ms,
    response_input::read_stdin_bounded,
    runtime::{
        RuntimeRegistry,
        hook_protocol::{FocusHandoff, HOOK_INPUT_LIMIT, HookLaunch},
    },
    storage::Storage,
};
use tmt_core::{
    binding::session::HarnessId,
    request::{RequestService, focus::FocusState},
};

const BUDGET: Duration = Duration::from_millis(crate::invocation::MAXIMUM_HOOK_WORK_BUDGET_MS);
const PUBLICATION_RESERVE: Duration = Duration::from_millis(600);

pub fn execute(
    provider: &str,
    scope: &str,
    worker: bool,
    work_budget_ms: Option<u64>,
) -> io::Result<u8> {
    let deadline = Instant::now()
        + work_budget_ms
            .map(Duration::from_millis)
            .unwrap_or(BUDGET)
            .min(BUDGET);
    let result = (|| -> Result<(), ()> {
        let launch: HookLaunch = serde_json::from_str(scope).map_err(|_| ())?;
        if !launch.valid() {
            return Err(());
        }
        let registry = RuntimeRegistry::first_party();
        let harness = HarnessId::new(provider).map_err(|_| ())?;
        let lifecycle = registry.lifecycle(&harness).ok_or(())?;
        let input = read_stdin_bounded(
            deadline.saturating_duration_since(Instant::now()),
            HOOK_INPUT_LIMIT,
        )
        .map_err(|_| ())?;
        let Some(session) = lifecycle.decode_focus_turn(input.as_bytes()) else {
            return Ok(());
        };
        if worker {
            let (paths, stored) = crate::provider_hook_command::verified_focus_launch(
                provider,
                lifecycle,
                &launch,
                &session,
                deadline,
                SupervisedProbeRunner,
            )?;
            let mut storage = Storage::open_hook(&paths.database, deadline).map_err(|_| ())?;
            let Some(batch) = storage
                .claim_focus_for_launch(
                    &stored,
                    tmt_core::operation::new_operation_id(),
                    tmt_core::operation::new_operation_id(),
                )
                .map_err(|_| ())?
            else {
                storage.close().map_err(|_| ())?;
                return Ok(());
            };
            let page = match focus::read(&mut storage, &launch.identity_id, Some(&batch.id), 0, 128)
            {
                Ok(page) => page,
                Err(_) => {
                    let _ = RequestService::new(&mut storage, wall_time_ms).settle_focus_checklist(
                        &launch.identity_id,
                        &batch.id,
                        &batch.attempt_token,
                        FocusState::Unsent,
                    );
                    return Err(());
                }
            };
            let prepared = FocusHandoff {
                checklist_id: batch.id.clone(),
                attempt_token: batch.attempt_token.clone(),
                digest: focus::digest(&page, &batch),
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
        let worker_deadline = deadline.checked_sub(PUBLICATION_RESERVE).ok_or(())?;
        let remaining = worker_deadline
            .saturating_duration_since(Instant::now())
            .as_millis();
        if remaining == 0 {
            return Err(());
        }
        let args = [
            "__focus-hook".into(),
            provider.into(),
            "--launch".into(),
            scope.into(),
            "--worker".into(),
            "--work-budget-ms".into(),
            remaining.to_string().into(),
        ];
        let executable = std::env::current_exe().map_err(|_| ())?;
        let output = UnixCommandRunner
            .execute(CommandRequest {
                program: executable.as_os_str(),
                args: &args,
                input: input.as_bytes(),
                deadline: worker_deadline,
                max_output_bytes: 32 * 1024,
            })
            .map_err(|_| ())?;
        if !output.stderr.is_empty() {
            return Err(());
        }
        if output.stdout.is_empty() {
            return Ok(());
        }
        let prepared: FocusHandoff = serde_json::from_slice(&output.stdout).map_err(|_| ())?;
        if !tmt_core::dispatch::canonical_id(&prepared.checklist_id)
            || !tmt_core::dispatch::canonical_id(&prepared.attempt_token)
        {
            return Err(());
        }
        let payload = lifecycle.encode_focus_turn(&prepared.digest).ok_or(())?;
        let paths = tmt_adapters::config::ConfigPaths::discover().map_err(|_| ())?;
        let state = match crate::provider_hook_command::verified_focus_launch(
            provider,
            lifecycle,
            &launch,
            &session,
            deadline,
            UnixCommandRunner,
        ) {
            Err(()) => FocusState::Unsent,
            Ok(_) => {
                let mut storage = Storage::open_hook(&paths.database, deadline).map_err(|_| ())?;
                let active = RequestService::new(&mut storage, wall_time_ms)
                    .focus_policies(std::slice::from_ref(&launch.identity_id))
                    .map_err(|_| ())?
                    .into_iter()
                    .next()
                    .and_then(|policy| policy.active_checklist);
                storage.close().map_err(|_| ())?;
                if active.is_none_or(|batch| {
                    batch.id != prepared.checklist_id
                        || batch.attempt_token != prepared.attempt_token
                        || batch.state != FocusState::Claimed
                }) {
                    return Ok(());
                }
                publication_state(handoff::write_before(
                    &tmt_cli_style::stream::stdout(true).into_inner(),
                    payload.as_bytes(),
                    deadline,
                ))
            }
        };
        let mut storage = Storage::open_hook(&paths.database, deadline).map_err(|_| ())?;
        RequestService::new(&mut storage, wall_time_ms)
            .settle_focus_checklist(
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
        let _ = SupervisedProbeRunner::abort_worker_group();
    }
    // Unsupported, stale or unavailable Focus never vetoes an ordinary stop.
    Ok(u8::from(worker && result.is_err()))
}

fn publication_state(result: Result<(), handoff::HandoffFailure>) -> FocusState {
    match result {
        Ok(()) => FocusState::Delivered,
        Err(error) if error.written == 0 => FocusState::Unsent,
        Err(_) => FocusState::Uncertain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_complete_publication_is_delivered() {
        assert_eq!(publication_state(Ok(())), FocusState::Delivered);
        assert_eq!(
            publication_state(Err(handoff::HandoffFailure { written: 0 })),
            FocusState::Unsent
        );
        assert_eq!(
            publication_state(Err(handoff::HandoffFailure { written: 1 })),
            FocusState::Uncertain
        );
    }
}
