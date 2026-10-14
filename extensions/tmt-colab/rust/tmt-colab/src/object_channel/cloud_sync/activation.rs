//! When to ask Remote to activate native sync, and what to tell a person. Pure policy: the
//! control exchange itself is `supervisor::activate_sync`, one bounded call at a time.
use super::{ActivationFailure, Refusal};
use std::time::Duration;

/// Attempts per trigger (serve start, or a channel the worker did not activate), and the wait
/// before each retry. A trigger that exhausts them idles until the next one.
const RETRIES: [Duration; 2] = if cfg!(test) {
    [Duration::from_millis(100), Duration::from_millis(200)]
} else {
    [Duration::from_secs(5), Duration::from_secs(30)]
};

/// What the worker does after one activation attempt failed.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Next {
    /// Try again after this wait.
    Retry(Duration),
    /// Out of attempts for this trigger. A new trigger starts a fresh budget.
    Idle,
    /// Remote's verdict holds until it restarts: nothing is retried.
    Sticky,
}

#[derive(Default)]
pub(super) struct Policy {
    tried: usize,
    sticky: bool,
}
impl Policy {
    /// A new trigger: serve start or a channel generation this worker did not activate. A
    /// Remote that restarted resolves its project again, so its earlier verdict no longer holds.
    pub(super) fn trigger(&mut self) {
        self.tried = 0;
        self.sticky = false;
    }
    pub(super) fn is_sticky(&self) -> bool {
        self.sticky
    }
    pub(super) fn failed(&mut self, failure: &ActivationFailure) -> Next {
        if let ActivationFailure::Remote(remote) = failure
            && sticky(remote)
        {
            self.sticky = true;
            return Next::Sticky;
        }
        match RETRIES.get(self.tried) {
            Some(wait) => {
                self.tried += 1;
                Next::Retry(*wait)
            }
            None => Next::Idle,
        }
    }
}
/// Typed refusals that name a state of Remote, not of this attempt. A deadline, an absent door
/// and a reply that did not complete are all attempt-shaped and never sticky.
fn sticky(failure: &Refusal) -> bool {
    match failure.code.as_str() {
        "REMOTE_STATE_UNAVAILABLE" => failure.state.as_deref() != Some("deadline"),
        "REMOTE_SERVE_OUTDATED"
        | "REMOTE_CONTROL_UNSUPPORTED"
        | "REMOTE_SCOPE_DENIED"
        | "REMOTE_STATE_UNSAFE" => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn remote(code: &str, state: Option<&str>) -> ActivationFailure {
        ActivationFailure::Remote(Refusal {
            code: code.into(),
            state: state.map(str::to_owned),
        })
    }
    #[test]
    fn a_remote_verdict_is_never_retried_until_a_restarted_remote_is_a_new_trigger() {
        for code in ["REMOTE_STATE_UNAVAILABLE", "REMOTE_SERVE_OUTDATED"] {
            let mut policy = Policy::default();
            let failure = remote(code, None);
            assert_eq!(policy.failed(&failure), Next::Sticky);
            assert!(policy.is_sticky());
            // Remote resolved its project once, at its own start: a restart asks again.
            policy.trigger();
            assert!(!policy.is_sticky());
            assert_eq!(policy.failed(&failure), Next::Sticky);
        }
    }
    #[test]
    fn attempt_shaped_failures_retry_a_bounded_number_of_times_per_trigger() {
        for failure in [
            remote("REMOTE_STATE_UNAVAILABLE", Some("deadline")),
            remote("REMOTE_NOT_RUNNING", None),
            ActivationFailure::Local,
        ] {
            let mut policy = Policy::default();
            assert_eq!(policy.failed(&failure), Next::Retry(RETRIES[0]));
            assert_eq!(policy.failed(&failure), Next::Retry(RETRIES[1]));
            assert_eq!(policy.failed(&failure), Next::Idle);
            assert!(!policy.is_sticky());
            // A new trigger gets a fresh budget.
            policy.trigger();
            assert_eq!(policy.failed(&failure), Next::Retry(RETRIES[0]));
        }
    }
}
