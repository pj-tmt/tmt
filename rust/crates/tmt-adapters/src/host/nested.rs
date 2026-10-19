//! A nested caller's host (#3583): tmux and exactly one approved external
//! host name the caller, in either order. One pinned ancestry orders both
//! hosts' panes, and the nearest located pane owns the caller. Doubt on
//! either side refuses; it never falls back to the other host.

use super::{CallerEnvironment, Host, HostError};
use crate::process::{CommandError, CommandRunner, ancestry, runtime::observe_starts};
use std::time::Instant;
use tmt_core::host::HostServerIds;

impl<R: CommandRunner> Host<R> {
    /// The caller's pane shell pid on the pinned ancestry that chose this
    /// nested caller's host; `None` for every other handle. A pane pid that
    /// a later snapshot reports must equal it.
    pub fn pinned_caller_pane(&self) -> Option<u64> {
        self.pinned_pane
    }
}

impl<R: CommandRunner + Clone> Host<R> {
    /// The owner of a nested caller. One pinned ancestry (each entry's
    /// parent and start from one read) locates both pane shells: the
    /// external one from its driver's conclusive answer, the tmux one from
    /// one server-checked `display-message`. A host whose conclusive answer
    /// is not on the chain is absent; the nearer located pane owns the
    /// caller. Every entry from the caller up to the owner pane must still
    /// be the process that was pinned. A partial tmux environment, doubt on
    /// either side, both hosts naming one pane process, no located pane, or a
    /// changed entry refuses; a failed cleanup is its own error.
    pub(super) fn for_nested_caller<I: HostServerIds>(
        environment: &CallerEnvironment,
        builtin: Self,
        runner: R,
        records: Vec<crate::driver_protocol::registry::DriverRecord>,
        ids: &mut I,
        deadline: Instant,
    ) -> Result<Self, HostError> {
        let unverified =
            |reason: &str| HostError::Unavailable(format!("caller host unverified: {reason}"));
        // Only a failed cleanup is an error of its own; anything else is doubt.
        let cleanup = |error: CommandError, reason: &str| {
            if error.cleanup_failed() {
                HostError::Process(error)
            } else {
                unverified(reason)
            }
        };
        let set =
            |value: &Option<std::ffi::OsString>| value.as_ref().is_some_and(|v| !v.is_empty());
        if !(set(&environment.tmux) && set(&environment.pane)) {
            return Err(unverified("the caller's tmux variables are partial"));
        }
        let pinning = "the caller's ancestry could not be pinned";
        let chain =
            ancestry::pinned_chain(&runner, environment.process_id, deadline).map_err(|error| {
                match error {
                    ancestry::AncestryError::Command(error) => cleanup(error, pinning),
                    ancestry::AncestryError::Unavailable => unverified(pinning),
                }
            })?;
        let callers = builtin
            .external
            .callers(environment, Some(&chain), deadline)?;
        if !callers.conclusive {
            return Err(unverified("an approved driver did not answer"));
        }
        let tmux = builtin
            .tmux
            .proven_caller_pane(environment, deadline)
            .map_err(|error| {
                if error.cleanup_failed() {
                    HostError::Tmux(error)
                } else {
                    unverified("the caller's tmux pane could not be proven")
                }
            })?;
        let tmux = chain.position(tmux);
        let external = callers.found.into_iter().next();
        // One pane process belongs to one host: both naming it is doubt.
        if external
            .as_ref()
            .zip(tmux)
            .is_some_and(|(external, tmux)| external.depth == tmux)
        {
            return Err(unverified("both hosts name the same pane process"));
        }
        let external = external.filter(|external| tmux.is_none_or(|tmux| external.depth < tmux));
        let owner = match (&external, tmux) {
            (Some(external), _) => external.depth,
            (None, Some(tmux)) => tmux,
            (None, None) => return Err(unverified("no host's pane is an ancestor of the caller")),
        };
        let segment = &chain.links()[..=owner];
        let pids: Vec<u64> = segment.iter().map(|link| link.process.pid()).collect();
        let now = observe_starts(&runner, &pids, deadline)
            .map_err(|error| cleanup(error, "the caller's ancestry could not be observed again"))?;
        if Instant::now() >= deadline
            || !segment
                .iter()
                .all(|link| now.get(&link.process.pid()) == Some(&link.process))
        {
            return Err(unverified(
                "a process between the caller and its pane changed",
            ));
        }
        let host = match external {
            Some(external) => Self::for_external_caller(external, runner, records, ids)?,
            None => builtin,
        };
        Ok(Self {
            pinned_pane: Some(pids[owner]),
            ..host
        })
    }
}
