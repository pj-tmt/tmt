//! Explicit binds complete only their own verifiable provider session.
use super::*;
use std::time::{Duration, Instant};
use tmt_adapters::runtime::RuntimeRegistry;
use tmt_core::binding::session::HarnessId;

pub(super) struct Warning {
    pub reason: String,
    pub fix: String,
}
pub(super) struct Completion {
    pub unbound: Option<bool>,
    pub warning: Option<Warning>,
}

pub(super) fn complete(result: &BoundIdentity) -> Completion {
    let mut completion = Completion {
        unbound: None,
        warning: None,
    };
    let Some(binding) = result.presence.binding.as_ref() else {
        return completion;
    };
    // The exact-caller path fences this binding before it can write. `add` from
    // elsewhere cannot lend this pane the invoker's provider coordinates.
    let attempted = crate::caller_session_command::complete_binding(binding);
    let Ok(paths) = ConfigPaths::discover() else {
        return completion;
    };
    let Ok(Some(stored)) = Storage::context_by_identity(
        &paths.database,
        &binding.identity_id,
        tmt_adapters::request_runtime::wall_time_ms(),
    ) else {
        return completion;
    };
    let Some(current) = stored
        .entry
        .binding
        .as_ref()
        .filter(|value| value.id == binding.id)
    else {
        return completion;
    };
    completion.unbound = current.session.unbound(&stored.preferences);
    let registry = RuntimeRegistry::first_party();
    let deadline = Instant::now() + Duration::from_millis(500);
    let processes: Vec<_> = registry
        .harnesses()
        .filter_map(|name| {
            let id = HarnessId::new(name).ok()?;
            let lifecycle = registry.lifecycle(&id)?;
            let process = lifecycle.observe_replacement(current.pane_pid, deadline)?;
            Some((id, process))
        })
        .collect();
    let same_admitted = completion.unbound == Some(false)
        && matches!(processes.as_slice(), [(driver, process)] if current.session.key.as_ref().is_some_and(|key| key.incarnation == *process)
            && stored.preferences.remembered.as_ref().is_some_and(|r| r.harness == *driver));
    if same_admitted {
        return completion;
    }
    if current.session.start_refusal.is_none()
        && processes.is_empty()
        && !matches!(attempted, Err("budget" | "storage" | "stale-commit"))
    {
        return completion;
    }
    let reason = current.session.start_refusal.as_ref().map_or(
        match attempted {
            Err("budget") => "the bounded session-verification allowance was exhausted",
            Err("binding") => "the verified caller is not this target binding",
            Err("storage" | "stale-commit") => "the verified session could not be committed",
            _ => "TMT could not verify the provider's session ID in this pane",
        },
        |refusal| refusal.reason.explanation(),
    );
    completion.warning = Some(Warning {
        reason: format!("The pane is bound, but the provider session was not recorded: {reason}"),
        fix: if stored.preferences.remembered.is_some() {
            format!(
                "Exit the agent in that pane, then run: tmt resume -- {}",
                crate::output::shell_word(&stored.entry.identity.name)
            )
        } else {
            "In that pane, send the agent a new prompt after installing provider hooks with: tmt setup"
                .into()
        },
    });
    completion
}

#[cfg(test)]
pub(crate) const PRINTED_HINTS: &[crate::cli_style_tests::HintSpec] = &[
    crate::cli_style_tests::HintSpec::core(
        "Exit the agent in that pane, then run: tmt resume -- {}",
        &[""],
        &[("{}", "worker")],
    ),
    crate::cli_style_tests::HintSpec::core(
        "In that pane, send the agent a new prompt after installing provider hooks with: tmt setup",
        &[""],
        &[],
    ),
];
