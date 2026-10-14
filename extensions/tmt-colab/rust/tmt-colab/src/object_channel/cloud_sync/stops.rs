//! Which pages' cloud reads are stopped and why. Each pass derives the answer from the Store and
//! the cloud, so nothing here is stored: a restart finds the same condition on its first pass.
//! A stop is announced once, and so is the recovery, never once per pass.
//!
//! The worker's session owns this state and lives as long as the session. No command or route
//! reads it yet: a readout attaches here, and where it is shown is decided with the browser
//! surface, not in this module.
use super::publisher::Fault;
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Stopped {
    pub reason: &'static str,
    /// When the page first stopped, in milliseconds since the epoch.
    pub since_ms: u64,
}

#[derive(Default)]
pub(super) struct Stops(BTreeMap<String, Stopped>);
impl Stops {
    /// Record what the latest read of `page` found, and the one line to show if that changed
    /// the page's state. A read that merely has to wait changes nothing.
    pub(super) fn observe(
        &mut self,
        page: &str,
        read: &Result<bool, Fault>,
        now_ms: u64,
    ) -> Option<String> {
        match read {
            Err(Fault::Stuck(reason)) => {
                let since_ms = self.0.get(page).map_or(now_ms, |stopped| stopped.since_ms);
                let before = self.0.insert(page.to_owned(), Stopped { reason, since_ms });
                (before.is_none_or(|stopped| stopped.reason != *reason))
                    .then(|| super::stopped_line(page, reason))
            }
            Ok(_) => self.0.remove(page).map(|_| super::resumed_line(page)),
            // The page was removed or moved to another epoch: the stop was about the old one.
            Err(Fault::Moved) => {
                self.0.remove(page);
                None
            }
            Err(Fault::Retry | Fault::Budget(_)) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "10000000-0000-4000-8000-000000000001";

    #[test]
    fn a_stop_and_its_recovery_are_each_shown_once() {
        let mut stops = Stops::default();
        let stuck = Err(Fault::Stuck("The cloud has newer sharing changes."));
        let first = stops.observe(PAGE, &stuck, 5).unwrap();
        assert!(first.starts_with("Cloud sync stopped for page "), "{first}");
        assert_eq!(
            first,
            format!(
                "Cloud sync stopped for page {PAGE}. The cloud has newer sharing changes. Edits continue on this machine."
            )
        );
        // Every later pass finds the same thing and says nothing.
        assert_eq!(stops.observe(PAGE, &stuck, 9), None);
        assert_eq!(stops.0[PAGE].since_ms, 5);
        // Waiting is neither a recovery nor a new stop.
        assert_eq!(stops.observe(PAGE, &Err(Fault::Retry), 11), None);
        assert_eq!(stops.observe(PAGE, &Err(Fault::Budget(1)), 12), None);
        assert!(stops.0.contains_key(PAGE));
        let resumed = stops.observe(PAGE, &Ok(false), 13).unwrap();
        assert_eq!(resumed, format!("Cloud sync resumed for page {PAGE}."));
        assert_eq!(stops.observe(PAGE, &Ok(true), 14), None);
        assert!(stops.0.is_empty());
    }

    #[test]
    fn a_new_reason_is_shown_and_keeps_when_the_page_first_stopped() {
        let mut stops = Stops::default();
        assert!(
            stops
                .observe(PAGE, &Err(Fault::Stuck("The cloud has newer sharing.")), 5)
                .is_some()
        );
        let second = stops
            .observe(
                PAGE,
                &Err(Fault::Stuck("The cloud and this machine disagree.")),
                8,
            )
            .unwrap();
        assert!(second.contains("disagree. Edits continue"));
        assert_eq!(stops.0[PAGE].since_ms, 5);
    }

    #[test]
    fn pages_stop_and_recover_independently_and_a_moved_page_forgets_its_stop() {
        let mut stops = Stops::default();
        let other = "10000000-0000-4000-8000-000000000002";
        stops.observe(PAGE, &Err(Fault::Stuck("A.")), 1);
        stops.observe(other, &Err(Fault::Stuck("B.")), 2);
        assert!(stops.observe(PAGE, &Ok(false), 3).is_some());
        assert!(stops.0.contains_key(other));
        assert_eq!(stops.observe(other, &Err(Fault::Moved), 4), None);
        assert!(stops.0.is_empty());
    }
}
