//! `tmt __allowance <provider>`: the intake a provider's first-party allowance mod calls (#5187).
//! One bounded JSON document on stdin, nothing printed, always exit 0. It stores nothing unless
//! the user's usage consent for the provider is on, and never opens the SQL store.
use std::{
    io,
    time::{Duration, Instant},
};
use tmt_adapters::{
    config::ConfigPaths, drivers::Registry, response_input::read_stdin_bounded,
    runtime::RuntimeRegistry, setup::usage_consented, skill_installation::ProviderEnvironment,
};
use tmt_core::binding::session::HarnessId;

const BUDGET: Duration = Duration::from_secs(2);
const MAX_INPUT: usize = 4096;

pub fn execute(provider: &str) -> io::Result<u8> {
    // An advisory observation: every failure is silent, because the mod must never be disturbed.
    let _ = intake(provider, Instant::now() + BUDGET);
    Ok(0)
}

fn intake(provider: &str, deadline: Instant) -> Option<bool> {
    let driver = Registry::builtin().find(provider)?;
    if !usage_consented(driver) {
        return None;
    }
    let registry = RuntimeRegistry::first_party();
    let lifecycle = registry.lifecycle(&HarnessId::new(provider).ok()?)?;
    let input = read_stdin_bounded(
        deadline.saturating_duration_since(Instant::now()),
        MAX_INPUT,
    )
    .ok()?;
    let paths = ConfigPaths::discover().ok()?;
    let environment = ProviderEnvironment::capture().ok()?;
    lifecycle
        .accept_allowance(
            &environment,
            &paths.global_dir,
            input.as_bytes(),
            tmt_adapters::request_runtime::wall_time_ms(),
        )
        .ok()
}
