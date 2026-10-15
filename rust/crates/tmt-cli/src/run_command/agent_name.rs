//! Gives the agent that `tmt run` or `tmt resume` started the identity's name
//! as its host agent name. The host recognizes the agent only some time after
//! it starts, so the supervisor tries at each wait tick, not at the spawn.
//!
//! An attempt reads the identity's current name, calls the host with no
//! storage transaction open, and reads again. The next tick tries again only
//! while the host recognizes no agent yet; to set a name that the read after
//! the call shows changed, unless the host definitely refused, also after an
//! uncertain answer; or after a read before the call failed and nothing was
//! called. A failed read after the call adds no retry: the host's answer
//! decides. Every other answer is final, so an uncertain answer is never
//! repeated for the same name. All of this stays inside the six attempts.
//!
//! This is best effort: two concurrent renames of one identity, a read that
//! fails after the call, exhausted attempts, or a command that exits first
//! can leave an older name, which `tmt ls` shows and `tmt this` repairs. A
//! binding that ended while the run set the name gets one warning: the run
//! does not clear the name.

use std::time::{Duration, Instant};
use tmt_adapters::{
    config::ConfigPaths,
    host::{AgentNameRefusal, AgentNaming, Host, HostError},
    storage::Storage,
};
use tmt_core::{binding::Binding, identity::Identity};

/// The most attempts, one per wait tick: about 30 s at the 5 s cadence.
pub(super) const ATTEMPTS: u32 = 6;

/// The most the host call of one attempt may take. A tick blocks signal
/// forwarding to the child while it runs, so an attempt is short, and a
/// timeout ends the loop.
pub(super) const CALL: Duration = Duration::from_secs(1);

/// The most each storage read of an attempt waits for a writer.
const READ: Duration = Duration::from_millis(250);

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

/// What the naming has to say after the command exits.
pub(super) enum Issue<E> {
    /// The host's last answer for `name`.
    Host { name: String, error: E },
    /// The identity was renamed at every attempt; `name` is the newest.
    Renamed(String),
    /// The binding ended after the host may have set `name`.
    Ended(String),
}

/// How the naming ended, for after the command exits.
pub(super) enum Outcome<E> {
    /// Named, not linked, moved on, or never tried: nothing to say.
    Quiet,
    /// One warning.
    Warning(Issue<E>),
    /// A driver process could not be cleaned up: a failure, not a warning.
    Leaked(E),
}

/// What storage said after the call.
pub(super) enum After {
    /// The binding is current and the identity keeps the name it was given.
    Current,
    /// The binding is current, and the identity now has this name.
    Renamed(String),
    /// The binding ended or moved to another pane.
    Ended,
    /// Storage could not be read: the host's answer stands.
    Unknown,
}

/// What one tick did.
pub(super) enum Attempt<E> {
    /// The host answered for `name`, and storage then said `after`.
    Answered {
        name: String,
        answer: Result<AgentNaming, E>,
        after: After,
    },
    /// The binding ended before any call: whoever ended it handles the
    /// name, so the run stops without a warning.
    Moved,
    /// Storage could not be read in time, and nothing was called; the
    /// attempt counts, and the next tick tries again.
    Skipped,
}

/// The storage reads and the host call of one attempt. The run passes
/// [`Live`]; the tests pass a store and a host that they change at exact
/// points of the attempt.
pub(super) trait Ports {
    type Error;
    /// The identity of the run's binding while that binding is still
    /// current, `None` when it ended or moved, or `Err` when storage could
    /// not be read in time.
    fn current(&mut self) -> Result<Option<Identity>, ()>;
    /// Asks the host to give the agent `identity`'s name.
    fn name_agent(&mut self, identity: &Identity) -> Result<AgentNaming, Self::Error>;
}

/// The run's own storage and host.
pub(super) struct Live<'a> {
    pub(super) paths: &'a ConfigPaths,
    pub(super) host: &'a Host,
    pub(super) binding: &'a Binding,
}

impl Ports for Live<'_> {
    type Error = HostError;

    fn current(&mut self) -> Result<Option<Identity>, ()> {
        let mut storage =
            Storage::open_hook(&self.paths.database, Instant::now() + READ).map_err(|_| ())?;
        let entry = storage.binding_entry(&self.binding.identity_id);
        let _ = storage.close();
        Ok(entry.map_err(|_| ())?.and_then(|entry| {
            entry
                .binding
                .is_some_and(|current| current.id == self.binding.id)
                .then_some(entry.identity)
        }))
    }

    fn name_agent(&mut self, identity: &Identity) -> Result<AgentNaming, HostError> {
        self.host
            .name_agent(self.binding, identity, Some(Instant::now() + CALL))
    }
}

/// One attempt: a read, the host call with no transaction open, a read.
pub(super) fn attempt<P: Ports>(ports: &mut P) -> Attempt<P::Error> {
    let identity = match ports.current() {
        Ok(Some(identity)) => identity,
        Ok(None) => return Attempt::Moved,
        Err(()) => return Attempt::Skipped,
    };
    let answer = ports.name_agent(&identity);
    let after = match ports.current() {
        Ok(Some(now)) if now.name == identity.name => After::Current,
        Ok(Some(now)) => After::Renamed(now.name),
        Ok(None) => After::Ended,
        Err(()) => After::Unknown,
    };
    Attempt::Answered {
        name: identity.name,
        answer,
        after,
    }
}

/// The warning for `issue`, and its hint. `host` and `target` are the
/// binding's, for the host command that clears a name.
pub(super) fn warning(
    issue: Issue<HostError>,
    host: &str,
    target: Option<&str>,
) -> (String, Option<String>) {
    match issue {
        Issue::Host { name, error } => {
            let (what, fix) = crate::binding_command::agent_name_text(&name, &error);
            (what, Some(fix))
        }
        Issue::Renamed(name) => {
            let (what, fix) = crate::binding_command::agent_name_renamed_text(&name);
            (what, Some(fix))
        }
        Issue::Ended(name) => (
            crate::binding_command::agent_name_ended_text(&name, host, target),
            None,
        ),
    }
}

pub(super) struct AgentNamer<E = HostError> {
    attempts: u32,
    settled: bool,
    last: Option<Issue<E>>,
    leaked: Option<E>,
}

impl<E> Default for AgentNamer<E> {
    fn default() -> Self {
        Self {
            attempts: 0,
            settled: false,
            last: None,
            leaked: None,
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
        let exhausted = self.attempts >= ATTEMPTS;
        let (settled, last) = match call() {
            Attempt::Moved => (true, None),
            Attempt::Skipped => (exhausted, self.last.take()),
            Attempt::Answered {
                answer: Err(error), ..
            } if error.leaked() => {
                self.leaked = Some(error);
                (true, None)
            }
            Attempt::Answered {
                name,
                answer,
                after: After::Ended,
            } => {
                // The host may hold the name of a binding that is gone.
                let maybe_set = match &answer {
                    Ok(naming) => *naming == AgentNaming::Done,
                    Err(error) => error.class() == AgentNameRefusal::Uncertain,
                };
                (true, maybe_set.then_some(Issue::Ended(name)))
            }
            Attempt::Answered {
                answer: Ok(AgentNaming::NotLinked),
                ..
            } => (true, None),
            Attempt::Answered {
                answer,
                after: After::Renamed(newer),
                ..
            } => match answer {
                // The call may have landed after the rename's own naming,
                // so the next tick sets the newer name.
                Ok(_) => (exhausted, Some(Issue::Renamed(newer))),
                Err(error)
                    if matches!(
                        error.class(),
                        AgentNameRefusal::NoAgent | AgentNameRefusal::Uncertain
                    ) =>
                {
                    (exhausted, Some(Issue::Renamed(newer)))
                }
                // A definite refusal changed nothing, and the rename names
                // the agent itself.
                Err(_) => (true, None),
            },
            Attempt::Answered { answer: Ok(_), .. } => (true, None),
            Attempt::Answered {
                name,
                answer: Err(error),
                ..
            } => (
                error.class() != AgentNameRefusal::NoAgent || exhausted,
                Some(Issue::Host { name, error }),
            ),
        };
        self.settled = settled;
        self.last = last;
    }

    /// Whether a driver process could not be cleaned up: the run then stops
    /// all optional work and keeps only the command's own supervision.
    pub(super) fn leaked(&self) -> bool {
        self.leaked.is_some()
    }

    /// How the naming ended. A command that exits before the first attempt
    /// gets nothing to say.
    pub(super) fn into_outcome(self) -> Outcome<E> {
        match (self.leaked, self.last) {
            (Some(error), _) => Outcome::Leaked(error),
            (None, Some(issue)) => Outcome::Warning(issue),
            (None, None) => Outcome::Quiet,
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

    /// The host's answer for `worker`, with the binding still current.
    fn answered(answer: Result<AgentNaming, Refused>) -> Attempt<Refused> {
        answered_then(answer, After::Current)
    }

    fn answered_then(answer: Result<AgentNaming, Refused>, after: After) -> Attempt<Refused> {
        Attempt::Answered {
            name: "worker".into(),
            answer,
            after,
        }
    }

    /// The issue a namer ends with, as the class of a host answer or the
    /// name of a rename or an ended binding.
    fn issue(namer: AgentNamer<Refused>) -> Option<String> {
        match namer.into_outcome() {
            Outcome::Quiet => None,
            Outcome::Warning(Issue::Host { name, error }) => Some(format!("{name}:{:?}", error.0)),
            Outcome::Warning(Issue::Renamed(name)) => Some(format!("renamed:{name}")),
            Outcome::Warning(Issue::Ended(name)) => Some(format!("ended:{name}")),
            Outcome::Leaked(_) => Some("leaked".into()),
        }
    }

    /// A driver process that could not be cleaned up is fatal for the
    /// naming, even on an answer that would otherwise be retried, and is a
    /// failure, never a warning.
    #[test]
    fn a_leaked_driver_process_stops_the_loop_as_a_failure() {
        let mut namer = AgentNamer::<Refused>::default();
        namer.attempt(|| answered(Err(Refused(AgentNameRefusal::NoAgent, true))));
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
            namer.attempt(|| answered(answers.next().expect("no call after the name is set")));
        }
        assert_eq!(namer.attempts(), 2);
        assert_eq!(issue(namer), None);
    }

    #[test]
    fn an_agent_never_recognized_stops_after_six_attempts_with_one_issue() {
        let mut namer = AgentNamer::<Refused>::default();
        let mut calls = 0;
        for _ in 0..10 {
            namer.attempt(|| {
                calls += 1;
                answered(Err(refused(AgentNameRefusal::NoAgent)))
            });
        }
        assert_eq!(calls, ATTEMPTS);
        assert_eq!(issue(namer).as_deref(), Some("worker:NoAgent"));
    }

    #[test]
    fn any_other_answer_ends_the_loop_at_once() {
        for (answer, expected) in [
            (Ok(AgentNaming::NotLinked), None),
            (
                Err(refused(AgentNameRefusal::Rejected)),
                Some("worker:Rejected"),
            ),
            (
                Err(refused(AgentNameRefusal::Foreign)),
                Some("worker:Foreign"),
            ),
            // A timeout or failure is uncertain, and is not repeated.
            (
                Err(refused(AgentNameRefusal::Uncertain)),
                Some("worker:Uncertain"),
            ),
        ] {
            let mut namer = AgentNamer::<Refused>::default();
            let mut answer = Some(answer);
            for _ in 0..3 {
                namer.attempt(|| answered(answer.take().expect("one attempt only")));
            }
            assert_eq!(namer.attempts(), 1);
            assert_eq!(issue(namer).as_deref(), expected);
        }
    }

    /// A binding that ended before any call stops the run quietly. A skipped
    /// tick counts, and called nothing.
    #[test]
    fn a_binding_gone_before_the_call_stops_the_loop_quietly_and_a_skipped_tick_counts() {
        let mut namer = AgentNamer::<Refused>::default();
        namer.attempt(|| answered(Err(refused(AgentNameRefusal::NoAgent))));
        namer.attempt(|| Attempt::Moved);
        namer.attempt(|| panic!("no attempt after the binding moved"));
        assert_eq!(namer.attempts(), 2);
        assert_eq!(issue(namer), None);

        let mut namer = AgentNamer::<Refused>::default();
        for _ in 0..10 {
            namer.attempt(|| Attempt::Skipped);
        }
        assert_eq!(namer.attempts(), ATTEMPTS);
    }

    /// A storage read that fails after the call never turns the host's
    /// answer into a retry: the answer stands.
    #[test]
    fn a_failed_read_after_the_call_keeps_the_hosts_answer() {
        for (answer, expected) in [
            (Ok(AgentNaming::Done), None),
            (
                Err(refused(AgentNameRefusal::Uncertain)),
                Some("worker:Uncertain"),
            ),
        ] {
            let mut namer = AgentNamer::<Refused>::default();
            let mut answer = Some(answer);
            for _ in 0..3 {
                namer.attempt(|| {
                    answered_then(answer.take().expect("one attempt only"), After::Unknown)
                });
            }
            assert_eq!(namer.attempts(), 1);
            assert_eq!(issue(namer).as_deref(), expected);
        }
    }

    /// A rename seen after a call is set at the next tick, again after each
    /// call, and a run that never catches up says so once.
    #[test]
    fn a_rename_after_a_call_is_set_at_the_next_tick() {
        let mut namer = AgentNamer::<Refused>::default();
        namer.attempt(|| answered_then(Ok(AgentNaming::Done), After::Renamed("boss".into())));
        namer.attempt(|| answered_then(Ok(AgentNaming::Done), After::Renamed("chief".into())));
        namer.attempt(|| answered(Ok(AgentNaming::Done)));
        namer.attempt(|| panic!("no attempt after the newest name is set"));
        assert_eq!(namer.attempts(), 3);
        assert_eq!(issue(namer), None);

        let mut namer = AgentNamer::<Refused>::default();
        for round in 0..10 {
            namer.attempt(|| {
                answered_then(
                    Err(refused(AgentNameRefusal::Uncertain)),
                    After::Renamed(format!("name{round}")),
                )
            });
        }
        assert_eq!(namer.attempts(), ATTEMPTS);
        assert_eq!(issue(namer).as_deref(), Some("renamed:name5"));

        // A definite refusal changed nothing: the rename names the agent.
        let mut namer = AgentNamer::<Refused>::default();
        namer.attempt(|| {
            answered_then(
                Err(refused(AgentNameRefusal::Foreign)),
                After::Renamed("boss".into()),
            )
        });
        namer.attempt(|| panic!("no attempt after a definite refusal"));
        assert_eq!(issue(namer), None);
    }

    /// A binding that ended after a call that may have set the name gets
    /// one warning; one whose call changed nothing gets none.
    #[test]
    fn a_binding_that_ended_after_a_call_warns_only_when_the_name_may_be_set() {
        for (answer, expected) in [
            (Ok(AgentNaming::Done), Some("ended:worker")),
            (
                Err(refused(AgentNameRefusal::Uncertain)),
                Some("ended:worker"),
            ),
            (Err(refused(AgentNameRefusal::NoAgent)), None),
            (Err(refused(AgentNameRefusal::Foreign)), None),
            (Ok(AgentNaming::NotLinked), None),
        ] {
            let mut namer = AgentNamer::<Refused>::default();
            let mut answer = Some(answer);
            for _ in 0..3 {
                namer.attempt(|| {
                    answered_then(answer.take().expect("one attempt only"), After::Ended)
                });
            }
            assert_eq!(issue(namer).as_deref(), expected);
        }
    }

    #[test]
    fn a_command_that_exits_before_the_first_tick_gets_no_warning() {
        assert_eq!(issue(AgentNamer::<Refused>::default()), None);
    }

    /// One other command's effect: its commit and its own naming.
    type Move = fn(&mut World);

    /// The store and the host of a race, with the other commands' moves at
    /// exact points of each run call: before the call lands (between the
    /// run's first read and its call) or after it (between the call and the
    /// second read). No clock or thread decides the order.
    struct World {
        /// The identity's name in storage.
        name: String,
        bound: bool,
        /// The host's agent name.
        agent: Option<String>,
        /// The names the run asked for, in order.
        calls: Vec<String>,
        /// Per run call: the moves before and after it lands.
        moves: std::collections::VecDeque<(Option<Move>, Option<Move>)>,
    }

    impl World {
        fn new(moves: Vec<(Option<Move>, Option<Move>)>) -> Self {
            Self {
                name: "worker".into(),
                bound: true,
                agent: None,
                calls: Vec::new(),
                moves: moves.into(),
            }
        }

        /// `tmt mv`: the new name is committed, then the mv names the agent.
        fn mv(&mut self, name: &str) {
            self.name = name.into();
            self.agent = Some(name.into());
        }

        /// `tmt unbind`: the unbind takes the agent name away, then the
        /// binding ends.
        fn unbind(&mut self) {
            self.agent = None;
            self.bound = false;
        }

        /// The run's naming at each of `ticks` wait ticks.
        fn run(&mut self, ticks: u32) -> AgentNamer<Refused> {
            let mut namer = AgentNamer::<Refused>::default();
            for _ in 0..ticks {
                namer.attempt(|| attempt(self));
            }
            namer
        }
    }

    impl Ports for World {
        type Error = Refused;

        fn current(&mut self) -> Result<Option<Identity>, ()> {
            Ok(self.bound.then(|| Identity {
                id: "identity".into(),
                name: self.name.clone(),
                canonical_name: self.name.clone(),
                lifetime: tmt_core::identity::Lifetime::Saved,
                created_at: String::new(),
                updated_at: String::new(),
            }))
        }

        fn name_agent(&mut self, identity: &Identity) -> Result<AgentNaming, Refused> {
            let (before, after) = self.moves.pop_front().unwrap_or_default();
            if let Some(other) = before {
                other(self);
            }
            self.calls.push(identity.name.clone());
            self.agent = Some(identity.name.clone());
            if let Some(other) = after {
                other(self);
            }
            Ok(AgentNaming::Done)
        }
    }

    /// A mv that commits and names the agent between the run's read and its
    /// call: the run's call lands last with the older name, the read after
    /// it sees the new name, and the next tick sets that.
    #[test]
    fn a_rename_between_the_read_and_the_call_is_set_at_the_next_tick() {
        let mut world = World::new(vec![(Some(|world| world.mv("boss")), None)]);
        let mut namer = world.run(1);
        assert_eq!(
            world.agent.as_deref(),
            Some("worker"),
            "the older name landed last"
        );
        for _ in 0..5 {
            namer.attempt(|| attempt(&mut world));
        }
        assert_eq!(world.calls, ["worker", "boss"]);
        assert_eq!(world.agent.as_deref(), Some("boss"));
        assert_eq!(issue(namer), None);
    }

    /// Two renames, each inside a run call, end with the newest name; so
    /// does a rename between the call and the second read.
    #[test]
    fn two_renames_during_the_runs_calls_end_with_the_newest_name() {
        let mut world = World::new(vec![
            (Some(|world| world.mv("boss")), None),
            (Some(|world| world.mv("chief")), None),
        ]);
        let namer = world.run(10);
        assert_eq!(world.calls, ["worker", "boss", "chief"]);
        assert_eq!(world.agent.as_deref(), Some("chief"));
        assert_eq!(issue(namer), None);

        let mut world = World::new(vec![(None, Some(|world| world.mv("boss")))]);
        let namer = world.run(10);
        assert_eq!(world.calls, ["worker", "boss"]);
        assert_eq!(world.agent.as_deref(), Some("boss"));
        assert_eq!(issue(namer), None);
    }

    /// A binding that ends while a run call is in flight: the run makes no
    /// later call, does not clear the name, and warns once. That holds
    /// whether the unbind's own clear came before the run's call landed
    /// (the name stays) or after it.
    #[test]
    fn a_binding_that_ends_during_the_runs_call_gets_one_warning() {
        let mut world = World::new(vec![(Some(World::unbind), None)]);
        let namer = world.run(10);
        assert_eq!(world.calls, ["worker"]);
        assert_eq!(world.agent.as_deref(), Some("worker"), "the run left it");
        assert_eq!(issue(namer).as_deref(), Some("ended:worker"));

        let mut world = World::new(vec![(None, Some(World::unbind))]);
        let namer = world.run(10);
        assert_eq!(world.calls, ["worker"]);
        assert_eq!(world.agent, None);
        assert_eq!(issue(namer).as_deref(), Some("ended:worker"));
    }

    /// The rename and ended warnings, after the command: the ended one
    /// names the Herdr clear command only for a Herdr pane with a target.
    #[test]
    fn the_rename_and_ended_warnings_say_how_to_repair() {
        let (what, fix) = warning(Issue::Renamed("chief".into()), "herdr", Some("w1:p2"));
        assert!(
            what.starts_with(
                "TMT could not confirm the agent name chief: the identity was renamed"
            ),
            "{what}"
        );
        assert_eq!(
            fix.as_deref(),
            Some("To retry setting the agent name, in the agent's pane run: tmt this chief")
        );
        let (what, fix) = warning(Issue::Ended("worker".into()), "herdr", Some("w1:p2"));
        assert_eq!(
            what,
            "The binding ended while TMT set the agent name worker. If the herdr agent still \
             has that agent name, clear it in the host (herdr agent rename 'w1:p2' --clear)."
        );
        assert_eq!(fix, None);
        let (what, _) = warning(Issue::Ended("worker".into()), "fake", Some("f1"));
        assert!(what.ends_with("clear it in the host."), "{what}");
    }
}
