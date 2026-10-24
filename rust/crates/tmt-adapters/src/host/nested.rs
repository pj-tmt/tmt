//! A nested caller's host (#3583): tmux and an approved external host name
//! the caller, in either order. One pinned ancestry orders both hosts'
//! panes, and the nearest located pane owns the caller. A hook refuses on
//! doubt and never falls back to the other host (#3583); an ordinary
//! command keeps its fallback to tmux (#3920).

use super::{CallerEnvironment, ExternalCaller, Host, HostError, external::Callers};
use crate::process::{CommandError, CommandRunner, ancestry, runtime::observe_starts};
use std::time::Instant;
use tmt_core::host::HostServerIds;

fn unverified(reason: &str) -> HostError {
    HostError::Unavailable(format!("caller host unverified: {reason}"))
}

/// Doubt as `None`; only a failed cleanup stays an error.
fn unless_doubt<T>(result: Result<T, HostError>) -> Result<Option<T>, HostError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.cleanup_failed() => Err(error),
        Err(_) => Ok(None),
    }
}

/// Only a failed cleanup is an error of its own; anything else is doubt.
fn cleanup(error: CommandError, reason: &str) -> HostError {
    if error.cleanup_failed() {
        HostError::Process(error)
    } else {
        unverified(reason)
    }
}

impl<R: CommandRunner> Host<R> {
    /// The caller's pane shell pid on the pinned ancestry that chose this
    /// nested caller's host; `None` for every other handle. A pane pid that
    /// a later snapshot reports must equal it.
    pub fn pinned_caller_pane(&self) -> Option<u64> {
        self.pinned_pane
    }

    /// An ordinary command's nested caller: the nearest external pane that
    /// one pinned ancestry locates, unless the caller's tmux pane is nearer
    /// or both name one pane process. tmux doubt counts as tmux absent, as a
    /// tmux environment left from a closed tmux does. Any other doubt, and a
    /// change from the caller up to the external pane before the answer,
    /// keep tmux. A failed cleanup is an error, never doubt.
    pub(super) fn nearest_nested_external(
        &self,
        environment: &CallerEnvironment,
        deadline: Instant,
    ) -> Result<Option<ExternalCaller>, HostError> {
        let Some((chain, callers)) = unless_doubt(self.pinned_callers(environment, deadline))?
        else {
            return Ok(None);
        };
        let Some(external) = callers.found.into_iter().next() else {
            return Ok(None);
        };
        let tmux = match self.tmux.proven_caller_pane(environment, deadline) {
            Ok(pid) => chain.position(pid),
            Err(error) if error.cleanup_failed() => return Err(HostError::Tmux(error)),
            Err(_) => None,
        };
        if tmux.is_some_and(|tmux| tmux <= external.depth) {
            return Ok(None);
        }
        let unchanged = Self::unchanged(self.external.runner(), &chain, external.depth, deadline);
        Ok(unless_doubt(unchanged)?.map(|_| external))
    }

    /// The caller's ancestry, pinned once, and the external panes the
    /// approved drivers locate on it.
    fn pinned_callers(
        &self,
        environment: &CallerEnvironment,
        deadline: Instant,
    ) -> Result<(ancestry::PinnedChain, Callers), HostError> {
        let pinning = "the caller's ancestry could not be pinned";
        let chain =
            ancestry::pinned_chain(self.external.runner(), environment.process_id, deadline)
                .map_err(|error| match error {
                    ancestry::AncestryError::Command(error) => cleanup(error, pinning),
                    ancestry::AncestryError::Unavailable => unverified(pinning),
                })?;
        let callers = self.external.callers(environment, Some(&chain), deadline)?;
        Ok((chain, callers))
    }

    /// The owner pane's shell pid, when every entry from the caller up to
    /// `owner` is still the process that was pinned, within the deadline.
    fn unchanged(
        runner: &R,
        chain: &ancestry::PinnedChain,
        owner: usize,
        deadline: Instant,
    ) -> Result<u64, HostError> {
        let segment = &chain.links()[..=owner];
        let pids: Vec<u64> = segment.iter().map(|link| link.process.pid()).collect();
        let now = observe_starts(runner, &pids, deadline)
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
        Ok(pids[owner])
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
        let set =
            |value: &Option<std::ffi::OsString>| value.as_ref().is_some_and(|v| !v.is_empty());
        if !(set(&environment.tmux) && set(&environment.pane)) {
            return Err(unverified("the caller's tmux variables are partial"));
        }
        let (chain, callers) = builtin.pinned_callers(environment, deadline)?;
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
        let pane = Self::unchanged(&runner, &chain, owner, deadline)?;
        let host = match external {
            Some(external) => Self::for_external_caller(external, runner, records, ids)?,
            None => builtin,
        };
        Ok(Self {
            pinned_pane: Some(pane),
            ..host
        })
    }
}
