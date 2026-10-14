//! Gives the agent that `tmt run` or `tmt resume` started the identity's name
//! as its host agent name. The host recognizes the agent only some time after
//! it starts, so the supervisor tries at each wait tick, not at the spawn. It
//! retries only while the host recognizes no agent yet, and stops at the
//! first other answer: a definite refusal is final, and an uncertain one is
//! never repeated in the loop.
//!
//! An attempt holds the storage write lock while it checks that the binding
//! and the identity's name are still the run's and calls the host. A `tmt mv`
//! commits only before or after it, and names the agent after its commit, so
//! a run never sets an old name back over a newer one.

use std::time::{Duration, Instant};
use tmt_adapters::{
    config::ConfigPaths,
    host::{AgentNameRefusal, AgentNaming, Host, HostError},
    storage::{Storage, StorageError},
};
use tmt_core::{
    binding::{Binding, BindingRepository},
    identity::Identity,
};

/// The most attempts, one per wait tick: about 30 s at the 5 s cadence.
pub(super) const ATTEMPTS: u32 = 6;

/// The most one attempt may take. A tick blocks signal forwarding to the
/// child while it runs, so an attempt is short, and a timeout ends the loop.
pub(super) const CALL: Duration = Duration::from_secs(1);

/// An answer's refusal class, which alone decides a retry, and whether the
/// driver process could not be cleaned up.
pub(super) trait Refusal {
    fn class(&self) -> AgentNameRefusal;
    fn leaked(&self) -> bool;
}

impl Refusal for HostError {
    fn class(&self) -> AgentNameRefusal {
        AgentNameRefusal::of(self)
    }

    fn leaked(&self) -> bool {
        self.cleanup_failed()
    }
}

/// How the naming ended, for after the command exits.
pub(super) enum Outcome<E> {
    /// Named, not linked, moved on, or never tried: nothing to say.
    Quiet,
    /// The agent has no confirmed name: one warning.
    Warning(E),
    /// A driver process could not be cleaned up: a failure, not a warning.
    Leaked(E),
}

/// What one tick did.
pub(super) enum Attempt<E> {
    /// The host answered.
    Answered(Result<AgentNaming, E>),
    /// The binding or the identity's name changed: whoever changed it names
    /// the agent, so the run stops without a warning.
    Moved,
    /// Storage could not be read in time; the attempt counts, and the next
    /// tick tries again.
    Skipped,
}

/// One attempt under the storage write lock, all of it within [`CALL`].
pub(super) fn attempt(
    paths: &ConfigPaths,
    host: &Host,
    binding: &Binding,
    identity: &Identity,
) -> Attempt<HostError> {
    let deadline = Instant::now() + CALL;
    let Ok(mut storage) = Storage::open_hook(&paths.database, deadline) else {
        return Attempt::Skipped;
    };
    let answered = storage.with_binding_transaction::<_, StorageError>(|records| {
        let current = records
            .entry_by_id(&binding.identity_id)?
            .is_some_and(|entry| {
                entry.identity.name == identity.name
                    && entry
                        .binding
                        .is_some_and(|current| current.id == binding.id)
            });
        Ok(current.then(|| host.name_agent(binding, identity, Some(deadline))))
    });
    let _ = storage.close();
    match answered {
        Ok(Some(answer)) => Attempt::Answered(answer),
        Ok(None) => Attempt::Moved,
        Err(_) => Attempt::Skipped,
    }
}

pub(super) struct AgentNamer<E = HostError> {
    attempts: u32,
    settled: bool,
    last: Option<E>,
    leaked: bool,
}

impl<E> Default for AgentNamer<E> {
    fn default() -> Self {
        Self {
            attempts: 0,
            settled: false,
            last: None,
            leaked: false,
        }
    }
}

impl<E: Refusal> AgentNamer<E> {
    /// One attempt, while the name is still pending.
    pub(super) fn attempt(&mut self, call: impl FnOnce() -> Attempt<E>) {
        if self.settled {
            return;
        }
        self.attempts += 1;
        match call() {
            Attempt::Answered(Ok(_)) | Attempt::Moved => {
                self.settled = true;
                self.last = None;
            }
            Attempt::Skipped => self.settled = self.attempts >= ATTEMPTS,
            Attempt::Answered(Err(error)) if error.leaked() => {
                self.settled = true;
                self.leaked = true;
                self.last = Some(error);
            }
            Attempt::Answered(Err(error)) => {
                self.settled =
                    error.class() != AgentNameRefusal::NoAgent || self.attempts >= ATTEMPTS;
                self.last = Some(error);
            }
        }
    }

    /// Whether a driver process could not be cleaned up: the run then stops
    /// all optional work and keeps only the command's own supervision.
    pub(super) fn leaked(&self) -> bool {
        self.leaked
    }

    /// How the naming ended. A command that exits before the first attempt
    /// gets nothing to say.
    pub(super) fn into_outcome(self) -> Outcome<E> {
        match self.last {
            None => Outcome::Quiet,
            Some(error) if self.leaked => Outcome::Leaked(error),
            Some(error) => Outcome::Warning(error),
        }
    }

    #[cfg(test)]
    fn into_issue(self) -> Option<E> {
        match self.into_outcome() {
            Outcome::Warning(error) => Some(error),
            Outcome::Quiet | Outcome::Leaked(_) => None,
        }
    }

    #[cfg(test)]
    fn attempts(&self) -> u32 {
        self.attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An answer reduced to its class, as the driver's code maps to it, and
    /// whether its driver process was left behind.
    #[derive(Debug)]
    struct Refused(AgentNameRefusal, bool);

    impl Refusal for Refused {
        fn class(&self) -> AgentNameRefusal {
            self.0
        }

        fn leaked(&self) -> bool {
            self.1
        }
    }

    fn refused(class: AgentNameRefusal) -> Refused {
        Refused(class, false)
    }

    /// A driver process that could not be cleaned up is fatal for the
    /// naming, even on an answer that would otherwise be retried, and is a
    /// failure, never a warning.
    #[test]
    fn a_leaked_driver_process_stops_the_loop_as_a_failure() {
        let mut namer = AgentNamer::<Refused>::default();
        namer.attempt(|| Attempt::Answered(Err(Refused(AgentNameRefusal::NoAgent, true))));
        assert!(namer.leaked());
        namer.attempt(|| panic!("no attempt after a leaked driver process"));
        assert_eq!(namer.attempts(), 1);
        assert!(matches!(namer.into_outcome(), Outcome::Leaked(_)));
    }

    #[test]
    fn a_name_set_at_the_second_attempt_ends_the_loop_without_an_issue() {
        let mut namer = AgentNamer::<Refused>::default();
        let mut answers = vec![
            Err(refused(AgentNameRefusal::NoAgent)),
            Ok(AgentNaming::Done),
        ]
        .into_iter();
        for _ in 0..10 {
            namer.attempt(|| {
                Attempt::Answered(answers.next().expect("no call after the name is set"))
            });
        }
        assert_eq!(namer.attempts(), 2);
        assert!(namer.into_issue().is_none());
    }

    #[test]
    fn an_agent_never_recognized_stops_after_six_attempts_with_one_issue() {
        let mut namer = AgentNamer::<Refused>::default();
        let mut calls = 0;
        for _ in 0..10 {
            namer.attempt(|| {
                calls += 1;
                Attempt::Answered(Err(refused(AgentNameRefusal::NoAgent)))
            });
        }
        assert_eq!(calls, ATTEMPTS);
        let issue = namer.into_issue().expect("an issue to warn about");
        assert_eq!(issue.class(), AgentNameRefusal::NoAgent);
    }

    #[test]
    fn any_other_answer_ends_the_loop_at_once() {
        for (answer, issue) in [
            (Ok(AgentNaming::NotLinked), None),
            (
                Err(refused(AgentNameRefusal::Rejected)),
                Some(AgentNameRefusal::Rejected),
            ),
            (
                Err(refused(AgentNameRefusal::Foreign)),
                Some(AgentNameRefusal::Foreign),
            ),
            // A timeout or failure is uncertain, and is not repeated.
            (
                Err(refused(AgentNameRefusal::Uncertain)),
                Some(AgentNameRefusal::Uncertain),
            ),
        ] {
            let mut namer = AgentNamer::<Refused>::default();
            let mut answer = Some(answer);
            for _ in 0..3 {
                namer.attempt(|| Attempt::Answered(answer.take().expect("one attempt only")));
            }
            assert_eq!(namer.attempts(), 1);
            assert_eq!(namer.into_issue().map(|issue| issue.class()), issue);
        }
    }

    /// A `tmt mv` (or another bind) changed the name: the run stops and
    /// leaves the naming to it, with no warning. A skipped tick counts.
    #[test]
    fn a_moved_binding_stops_the_loop_quietly_and_a_skipped_tick_counts() {
        let mut namer = AgentNamer::<Refused>::default();
        namer.attempt(|| Attempt::Answered(Err(refused(AgentNameRefusal::NoAgent))));
        namer.attempt(|| Attempt::Moved);
        namer.attempt(|| panic!("no attempt after the binding moved"));
        assert_eq!(namer.attempts(), 2);
        assert!(namer.into_issue().is_none());

        let mut namer = AgentNamer::<Refused>::default();
        for _ in 0..10 {
            namer.attempt(|| Attempt::Skipped);
        }
        assert_eq!(namer.attempts(), ATTEMPTS);
    }

    #[test]
    fn a_command_that_exits_before_the_first_tick_gets_no_warning() {
        assert!(AgentNamer::<Refused>::default().into_issue().is_none());
    }
}
