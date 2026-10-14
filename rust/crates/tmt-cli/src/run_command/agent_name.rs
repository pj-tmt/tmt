//! Gives the agent that `tmt run` or `tmt resume` started the identity's name
//! as its host agent name. The host recognizes the agent only some time after
//! it starts, so the supervisor tries at each wait tick, not at the spawn. It
//! retries only while the host recognizes no agent yet, and stops at the
//! first other answer: a definite refusal is final, and an uncertain one is
//! never repeated in the loop.

use std::time::Duration;
use tmt_adapters::host::{AgentNameRefusal, AgentNaming, HostError};

/// The most attempts, one per wait tick: about 30 s at the 5 s cadence.
pub(super) const ATTEMPTS: u32 = 6;

/// The most one attempt may take. A tick blocks signal forwarding to the
/// child while it runs, so an attempt is short, and a timeout ends the loop.
pub(super) const CALL: Duration = Duration::from_secs(1);

/// An answer's refusal class, which alone decides a retry.
pub(super) trait Refusal {
    fn class(&self) -> AgentNameRefusal;
}

impl Refusal for HostError {
    fn class(&self) -> AgentNameRefusal {
        AgentNameRefusal::of(self)
    }
}

pub(super) struct AgentNamer<E = HostError> {
    attempts: u32,
    settled: bool,
    last: Option<E>,
}

impl<E> Default for AgentNamer<E> {
    fn default() -> Self {
        Self {
            attempts: 0,
            settled: false,
            last: None,
        }
    }
}

impl<E: Refusal> AgentNamer<E> {
    /// One attempt, while the name is still pending.
    pub(super) fn attempt(&mut self, call: impl FnOnce() -> Result<AgentNaming, E>) {
        if self.settled {
            return;
        }
        self.attempts += 1;
        match call() {
            Ok(_) => {
                self.settled = true;
                self.last = None;
            }
            Err(error) => {
                self.settled =
                    error.class() != AgentNameRefusal::NoAgent || self.attempts >= ATTEMPTS;
                self.last = Some(error);
            }
        }
    }

    /// Why the agent has no confirmed name, for one warning after the
    /// command exits. A command that exits before the first attempt gets none.
    pub(super) fn into_issue(self) -> Option<E> {
        self.last
    }

    #[cfg(test)]
    fn attempts(&self) -> u32 {
        self.attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An answer reduced to its class, as the driver's code maps to it.
    #[derive(Debug)]
    struct Refused(AgentNameRefusal);

    impl Refusal for Refused {
        fn class(&self) -> AgentNameRefusal {
            self.0
        }
    }

    fn refused(class: AgentNameRefusal) -> Refused {
        Refused(class)
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
            namer.attempt(|| answers.next().expect("no call after the name is set"));
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
                Err(refused(AgentNameRefusal::NoAgent))
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
                namer.attempt(|| answer.take().expect("one attempt only"));
            }
            assert_eq!(namer.attempts(), 1);
            assert_eq!(namer.into_issue().map(|issue| issue.class()), issue);
        }
    }

    #[test]
    fn a_command_that_exits_before_the_first_tick_gets_no_warning() {
        assert!(AgentNamer::<Refused>::default().into_issue().is_none());
    }
}
