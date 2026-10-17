//! Coalesced read-marker writes on the board's existing ordered action lane.
use crate::{
    config::{Config, ReadMarkers},
    core::Core,
};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Seen {
    actor: String,
    member: String,
    at: u64,
}
impl Seen {
    pub fn new(actor: &str, member: &str, at: u64) -> Option<Self> {
        let mut markers = ReadMarkers::default();
        markers.advance(actor, member, at).ok()?;
        Some(Self {
            actor: actor.into(),
            member: member.into(),
            at,
        })
    }
    pub fn matches_member(&self, member: &str) -> bool {
        self.member == member
    }

    fn markers(&self) -> ReadMarkers {
        let mut markers = ReadMarkers::default();
        // The controller admits only verified UUIDs and observed timestamps.
        markers
            .advance(&self.actor, &self.member, self.at)
            .expect("Seen validates the pair at admission");
        markers
    }
    fn publish(&self, core: &Core) -> Result<ReadMarkers, String> {
        Config::load(core)
            .and_then(|mut config| config.save_read_markers(&self.markers()))
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug, Default)]
struct Pending {
    updates: ReadMarkers,
    confirmed: ReadMarkers,
    ready: bool,
    queued: bool,
    failure: Option<String>,
}
impl Pending {
    fn observed(&mut self, saved: &ReadMarkers) {
        self.confirmed.merge(saved);
        self.updates.remove_confirmed(&self.confirmed);
        if self.updates.is_empty() {
            self.failure = None;
        }
    }

    fn recorded(
        &mut self,
        updates: &ReadMarkers,
        result: Result<ReadMarkers, String>,
    ) -> Result<ReadMarkers, String> {
        match result {
            Ok(saved) => {
                self.observed(&saved);
                Ok(self.confirmed.clone())
            }
            Err(error) => {
                let mut remaining = updates.clone();
                remaining.remove_confirmed(&self.confirmed);
                // Another layout may have durably published this exact pair
                // while the attempted write was in flight.
                if remaining.is_empty() {
                    return Ok(self.confirmed.clone());
                }
                self.updates.merge(&remaining);
                self.failure = Some(error.clone());
                Err(error)
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(in crate::board) struct Writer(Arc<Mutex<Pending>>);
impl Writer {
    pub fn confirmed(&self, actor: &str, member: &str) -> u64 {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .confirmed
            .get(actor, member)
    }

    /// Fresh Config observations and successful publications share one
    /// monotonic confirmed map, including writes made by another layout.
    pub fn observed(&self, saved: &ReadMarkers) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .observed(saved);
    }

    pub fn failure(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .failure
            .clone()
    }

    /// A successful guarded jump shares failure bookkeeping with quiet writes,
    /// without draining or rescheduling their already admitted batch.
    pub fn publish_seen(&self, seen: &Seen, core: &Core) -> Result<ReadMarkers, String> {
        let result = seen.publish(core);
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .recorded(&seen.markers(), result)
    }

    /// Called after the lane has joined. A result event need not have reached
    /// the UI before quitting, so the writer retains the unresolved failure.
    pub fn finish(&self, core: &Core) -> Result<(), crate::core::SquadError> {
        if let Some(writer) = self.offer() {
            let _ = writer.publish(core);
        }
        match self.failure() {
            Some(error) => Err(crate::core::SquadError::hinted(
                "SQUAD_CONFIG_WRITE_FAILED",
                &format!("Read marker not saved: {error}"),
                " ",
                "Reopen the conversation or jump to retry.",
            )),
            None => Ok(()),
        }
    }

    pub fn seen(&self, seen: &Seen) {
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let mut updates = seen.markers();
        updates.remove_confirmed(&pending.confirmed);
        if !updates.is_empty() {
            pending.updates.merge(&updates);
            pending.ready = true;
        }
    }
    pub fn offer(&self) -> Option<Self> {
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if !pending.ready || pending.queued || pending.updates.is_empty() {
            return None;
        }
        pending.queued = true;
        Some(self.clone())
    }
    pub fn publish(&self, core: &Core) -> Result<ReadMarkers, String> {
        self.complete(|updates| {
            Config::load(core)
                .and_then(|mut config| config.save_read_markers(updates))
                .map_err(|error| error.to_string())
        })
    }
    fn complete(
        &self,
        save: impl FnOnce(&ReadMarkers) -> Result<ReadMarkers, String>,
    ) -> Result<ReadMarkers, String> {
        let updates = {
            let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
            pending.ready = false;
            pending.updates.clone()
        };
        let result = save(&updates);
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        // A failure remains idle until a new explicit opening. An opening
        // admitted during this write may already have made the next batch ready.
        let result = pending.recorded(&updates, result);
        pending.queued = false;
        result
    }
}

#[derive(Default)]
pub(in crate::board) struct Session {
    pub writer: Writer,
    pub displayed: Option<Seen>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn seen(member: &str, at: u64) -> Seen {
        Seen {
            actor: "11111111-1111-1111-1111-111111111111".into(),
            member: member.into(),
            at,
        }
    }
    const A: &str = "22222222-2222-2222-2222-222222222222";
    const B: &str = "33333333-3333-3333-3333-333333333333";
    #[test]
    fn coalesces_preserves_inflight_updates_and_retries_only_after_an_opening() {
        let writer = Writer::default();
        writer.seen(&seen(A, 10));
        writer.seen(&seen(A, 20));
        let job = writer.offer().unwrap();
        assert!(writer.offer().is_none());
        let result = job
            .complete(|updates| {
                assert_eq!(updates.get(&seen(A, 0).actor, A), 20);
                writer.seen(&seen(B, 30));
                Ok(updates.clone())
            })
            .unwrap();
        assert_eq!(result.get(&seen(A, 0).actor, A), 20);
        let job = writer.offer().unwrap();
        assert!(job.complete(|_| Err("disk full".into())).is_err());
        assert!(
            writer.offer().is_none(),
            "a failed save is not an automatic retry"
        );
        assert_eq!(writer.failure().as_deref(), Some("disk full"));
        writer.seen(&seen(B, 15));
        let result = writer
            .offer()
            .unwrap()
            .complete(|updates| Ok(updates.clone()))
            .unwrap();
        assert_eq!(result.get(&seen(A, 0).actor, B), 30);
        assert_eq!(result.get(&seen(A, 0).actor, A), 20);
        assert!(writer.offer().is_none());
        assert!(writer.failure().is_none());
    }

    #[test]
    fn an_unrelated_success_cannot_erase_a_failed_member_or_a_newer_update() {
        let writer = Writer::default();
        writer.seen(&seen(A, 20));
        writer
            .offer()
            .unwrap()
            .complete(|_| Err("disk full".into()))
            .unwrap_err();
        let mut saved = seen(B, 30).markers();
        {
            let mut pending = writer.0.lock().unwrap();
            pending
                .recorded(&seen(B, 30).markers(), Ok(saved.clone()))
                .unwrap();
        }
        assert_eq!(writer.failure().as_deref(), Some("disk full"));
        assert!(
            writer.offer().is_none(),
            "another success is not an automatic retry"
        );
        writer.seen(&seen(A, 40));
        saved.merge(&seen(A, 20).markers());
        writer
            .0
            .lock()
            .unwrap()
            .recorded(&seen(A, 20).markers(), Ok(saved))
            .unwrap();
        assert!(writer.failure().is_some(), "a newer unsaved marker remains");
        let saved = writer
            .offer()
            .unwrap()
            .complete(|updates| Ok(updates.clone()))
            .unwrap();
        assert_eq!(saved.get(&seen(A, 0).actor, A), 40);
        assert!(writer.failure().is_none());
    }
    #[test]
    fn a_fresh_shared_marker_resolves_failure_including_a_late_failed_write() {
        let writer = Writer::default();
        writer.seen(&seen(A, 20));
        writer
            .offer()
            .unwrap()
            .complete(|_| Err("disk full".into()))
            .unwrap_err();
        writer.observed(&seen(B, 30).markers());
        assert!(
            writer.failure().is_some(),
            "an unrelated marker cannot resolve A"
        );
        writer.observed(&seen(A, 20).markers());
        assert!(writer.failure().is_none());
        assert!(
            writer.offer().is_none(),
            "observation resolves without another write"
        );
        writer.seen(&seen(A, 40));
        let saved = writer
            .offer()
            .unwrap()
            .complete(|_| {
                writer.observed(&seen(A, 40).markers());
                Err("late failed local write".into())
            })
            .unwrap();
        assert_eq!(saved.get(&seen(A, 0).actor, A), 40);
        assert_eq!(writer.confirmed(&seen(A, 0).actor, A), 40);
        assert!(writer.failure().is_none());
        assert!(writer.offer().is_none());
    }
}
