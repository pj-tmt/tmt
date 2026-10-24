//! Revision-bound acknowledgments on the board's existing ordered action lane.
use super::{
    ANSWERS_LIMIT, Answer,
    walk::{Body, Goal, Walk},
};
use crate::core::Core;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Seen {
    pub actor: String,
    pub answer: Answer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) enum Outcome {
    Acknowledged,
    Conflict(u64),
    NotFound,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Completed {
    pub actor: String,
    pub items: Vec<(Answer, Outcome)>,
}

impl Completed {
    fn read(actor: String, answers: &[Answer], document: Value) -> Result<Self, String> {
        let items = document["items"]
            .as_array()
            .filter(|items| items.len() == answers.len())
            .ok_or("requests.ack returned an incomplete result")?;
        let mut settled = Vec::new();
        for (answer, item) in answers.iter().zip(items) {
            if item["requestId"].as_str() != Some(&answer.request)
                || item["revision"].as_u64() != Some(answer.revision)
            {
                return Err("requests.ack returned a different request or revision".into());
            }
            let status = match item["status"].as_str() {
                Some("acknowledged" | "already") => Outcome::Acknowledged,
                Some("notFound") => Outcome::NotFound,
                Some("conflict") => Outcome::Conflict(
                    super::timestamp(&item["current"])
                        .filter(|current| *current > answer.revision)
                        .ok_or("requests.ack returned an invalid conflict revision")?,
                ),
                _ => return Err("requests.ack returned an unknown status".into()),
            };
            settled.push((answer.clone(), status));
        }
        Ok(Self {
            actor,
            items: settled,
        })
    }
}

/// One public batch. An empty page settles without invoking the write API.
pub(super) fn acknowledge(
    core: &Core,
    actor: &str,
    answers: &[Answer],
) -> Result<Completed, String> {
    if answers.is_empty() {
        return Ok(Completed {
            actor: actor.into(),
            items: Vec::new(),
        });
    }
    let items: Vec<_> = answers
        .iter()
        .map(|answer| json!({"requestId":answer.request,"revision":answer.revision}))
        .collect();
    let document = core
        .api_write("requests.ack", json!({"items":items}), Some(actor))
        .map_err(|error| error.to_string())?;
    Completed::read(actor.into(), answers, document)
}

#[derive(Debug, Default)]
struct Pending {
    updates: BTreeMap<(String, String), Answer>,
    ready: bool,
    queued: bool,
    failures: BTreeMap<(String, String), (u64, String)>,
    failed_walks: Vec<(Walk, String)>,
    walks: VecDeque<Walk>,
}

#[derive(Clone, Debug, Default)]
pub(in crate::board) struct Writer(Arc<Mutex<Pending>>);
impl Writer {
    pub fn failure(&self) -> Option<String> {
        let pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        pending
            .failures
            .values()
            .next()
            .map(|(_, error)| error.clone())
            .or_else(|| pending.failed_walks.first().map(|(_, error)| error.clone()))
    }

    /// Failure bookkeeping consumes the UI's existing Core projection; it is
    /// not a second unread cache. This also resolves a late error after another
    /// surface already acknowledged the same or a newer revision.
    pub fn observed(&self, page: &super::Page) {
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if pending.failures.is_empty() {
            return;
        }
        for answer in page
            .replies
            .values()
            .flatten()
            .filter(|answer| answer.acknowledged)
        {
            let key = (page.actor.clone(), answer.request.clone());
            if pending
                .failures
                .get(&key)
                .is_some_and(|(revision, _)| *revision <= answer.revision)
            {
                pending.failures.remove(&key);
            }
        }
    }

    pub fn recovery(&self) -> &'static str {
        let pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if pending.failures.is_empty()
            && pending
                .failed_walks
                .first()
                .is_some_and(|(walk, _)| matches!(walk.goal, Goal::Legacy(_)))
        {
            "Restart the board to retry the read migration."
        } else {
            "Reopen the conversation or use Mark read to retry."
        }
    }

    pub fn seen(&self, seen: &Seen) {
        if seen.answer.acknowledged {
            return;
        }
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let key = (seen.actor.clone(), seen.answer.request.clone());
        let held = pending
            .updates
            .entry(key)
            .or_insert_with(|| seen.answer.clone());
        if seen.answer.revision >= held.revision {
            *held = seen.answer.clone();
        }
        pending.ready = true;
    }

    pub fn body(&self, body: &Body) {
        if let Some(revision) = body.revision {
            self.seen(&Seen {
                actor: body.actor.clone(),
                answer: Answer {
                    member: body.member.clone(),
                    request: body.request.clone(),
                    kind: Some(body.kind),
                    answered_at_ms: body.at_ms,
                    revision,
                    acknowledged: false,
                },
            });
        } else {
            self.walk(
                Walk::new(body.actor.clone(), Goal::Body(body.clone())),
                true,
            );
        }
    }
    pub fn mark_member(&self, actor: &str, member: &str, before: Value) {
        self.walk(
            Walk::new(actor.into(), Goal::Member(member.into())).after(before),
            true,
        );
    }
    pub fn legacy(&self, actor: &str, markers: &crate::config::ReadMarkers) {
        let cutoffs = markers.for_actor(actor);
        if !cutoffs.is_empty() {
            self.walk(Walk::new(actor.into(), Goal::Legacy(cutoffs)), false);
        }
    }
    fn walk(&self, walk: Walk, retry: bool) {
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if pending
            .failed_walks
            .iter()
            .any(|(held, _)| held.same_goal(&walk))
            && !retry
        {
            return;
        }
        pending
            .failed_walks
            .retain(|(held, _)| !held.same_goal(&walk));
        if !pending.walks.iter().any(|held| held.same_goal(&walk)) {
            pending.walks.push_back(walk);
            pending.ready = true;
        }
    }

    pub fn offer(&self) -> Option<Self> {
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if !pending.ready
            || pending.queued
            || (pending.updates.is_empty() && pending.walks.is_empty())
        {
            return None;
        }
        pending.queued = true;
        Some(self.clone())
    }

    pub fn publish(&self, core: &Core) -> Result<Completed, String> {
        let walk = {
            let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
            if pending.updates.is_empty() {
                pending.ready = false;
                pending.walks.front().cloned()
            } else {
                None
            }
        };
        if let Some(walk) = walk {
            let result = walk.step(core);
            let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
            return match result {
                Ok((completed, next)) => {
                    pending.walks.pop_front();
                    for (answer, outcome) in &completed.items {
                        if matches!(outcome, Outcome::Acknowledged) {
                            let key = (completed.actor.clone(), answer.request.clone());
                            if pending
                                .failures
                                .get(&key)
                                .is_some_and(|(revision, _)| *revision <= answer.revision)
                            {
                                pending.failures.remove(&key);
                            }
                        }
                    }
                    pending
                        .failed_walks
                        .retain(|(held, _)| !held.same_goal(&walk));
                    if matches!(walk.goal, Goal::Legacy(_))
                        && completed
                            .items
                            .iter()
                            .any(|(_, outcome)| matches!(outcome, Outcome::Conflict(_)))
                    {
                        let reason = "Legacy read migration met a newer answer; restart the board to reacquire it.";
                        pending.failed_walks.push((walk, reason.into()));
                    } else if let Some(next) = next {
                        pending.walks.push_back(next);
                    }
                    pending.ready |= !pending.updates.is_empty() || !pending.walks.is_empty();
                    Ok(completed)
                }
                Err(error) => {
                    pending.walks.pop_front();
                    pending.failed_walks.push((walk, error.clone()));
                    pending.ready |= !pending.updates.is_empty() || !pending.walks.is_empty();
                    Err(error)
                }
            };
        }
        self.complete(|actor, answers| acknowledge(core, actor, answers))
    }

    fn complete(
        &self,
        write: impl FnOnce(&str, &[Answer]) -> Result<Completed, String>,
    ) -> Result<Completed, String> {
        let (actor, answers) = {
            let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
            pending.ready = false;
            let actor = pending
                .updates
                .keys()
                .next()
                .map(|(actor, _)| actor.clone())
                .ok_or("No admitted acknowledgments")?;
            let answers = pending
                .updates
                .iter()
                .filter(|((owner, _), _)| owner == &actor)
                .take(ANSWERS_LIMIT)
                .map(|(_, answer)| answer.clone())
                .collect::<Vec<_>>();
            (actor, answers)
        };
        let result = write(&actor, &answers);
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        match &result {
            Ok(_) => {
                for answer in &answers {
                    let key = (actor.clone(), answer.request.clone());
                    if pending
                        .updates
                        .get(&key)
                        .is_some_and(|held| held.revision == answer.revision)
                    {
                        pending.updates.remove(&key);
                    }
                }
                for answer in &answers {
                    pending
                        .failures
                        .remove(&(actor.clone(), answer.request.clone()));
                }
                pending.ready |= !pending.updates.is_empty() || !pending.walks.is_empty();
            }
            Err(error) => {
                for answer in &answers {
                    let key = (actor.clone(), answer.request.clone());
                    pending
                        .failures
                        .insert(key.clone(), (answer.revision, error.clone()));
                    if pending
                        .updates
                        .get(&key)
                        .is_some_and(|held| held.revision == answer.revision)
                    {
                        pending.updates.remove(&key);
                    }
                }
                pending.ready |= !pending.updates.is_empty() || !pending.walks.is_empty();
            }
        }
        // The UI must adopt the result before the lane can admit another batch.
        // Failure is also retained here if shutdown joins before consuming the event.
        result
    }

    pub fn settled(&self) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .queued = false;
    }

    pub fn finish(&self, core: &Core) -> Result<(), crate::core::SquadError> {
        self.settled();
        // Only already admitted writes drain on exit. A continuation is a new
        // read, not a write the user saw; its legacy cutoff remains for restart.
        while {
            let pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
            pending.ready && !pending.updates.is_empty()
        } {
            let Some(writer) = self.offer() else {
                break;
            };
            let result = writer.publish(core);
            self.settled();
            if result.is_err() {
                break;
            }
        }
        match self.failure() {
            Some(error) => Err(crate::core::SquadError::hinted(
                "SQUAD_ACK_FAILED",
                &format!("Unread update failed: {error}"),
                " ",
                self.recovery(),
            )),
            None => Ok(()),
        }
    }
}

#[derive(Default)]
pub(in crate::board) struct Session {
    pub writer: Writer,
    pub displayed: Vec<Body>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn seen(request: usize, revision: u64) -> Seen {
        Seen {
            actor: "actor".into(),
            answer: Answer {
                request: request.to_string(),
                member: "member".into(),
                answered_at_ms: 10,
                kind: Some(super::super::BodyKind::Final),
                revision,
                acknowledged: false,
            },
        }
    }
    fn success(actor: &str, answers: &[Answer]) -> Result<Completed, String> {
        Ok(Completed {
            actor: actor.into(),
            items: answers
                .iter()
                .map(|answer| (answer.clone(), Outcome::Acknowledged))
                .collect(),
        })
    }
    #[test]
    fn batches_fifty_distinct_requests_and_waits_for_ui_adoption() {
        let writer = Writer::default();
        for request in 0..51 {
            writer.seen(&seen(request, 1));
            writer.seen(&seen(request, 1));
        }
        let first = writer.offer().unwrap();
        first
            .complete(|actor, answers| {
                assert_eq!(answers.len(), 50);
                success(actor, answers)
            })
            .unwrap();
        assert!(
            writer.offer().is_none(),
            "the UI has not adopted the first batch"
        );
        writer.settled();
        writer
            .offer()
            .unwrap()
            .complete(|actor, answers| {
                assert_eq!(answers.len(), 1);
                success(actor, answers)
            })
            .unwrap();
        writer.settled();
        assert!(writer.offer().is_none());
    }
    #[test]
    fn failure_waits_for_explicit_admission_and_inflight_new_revision_survives() {
        let writer = Writer::default();
        writer.seen(&seen(1, 1));
        writer
            .offer()
            .unwrap()
            .complete(|_, _| Err("partial failure".into()))
            .unwrap_err();
        writer.settled();
        assert!(writer.offer().is_none());
        writer.seen(&seen(1, 1));
        writer
            .offer()
            .unwrap()
            .complete(|actor, answers| {
                writer.seen(&seen(1, 2));
                success(actor, answers)
            })
            .unwrap();
        writer.settled();
        writer
            .offer()
            .unwrap()
            .complete(|actor, answers| {
                assert_eq!(answers[0].revision, 2);
                success(actor, answers)
            })
            .unwrap();
        writer.settled();
        assert!(writer.failure().is_none());
        assert!(writer.offer().is_none());
    }
    #[test]
    fn result_parser_preserves_independent_outcomes_and_rejects_wrong_pairs() {
        let answers = (0..4).map(|n| seen(n, 1).answer).collect::<Vec<_>>();
        let document = json!({"items":[
            {"requestId":"0","revision":1,"status":"acknowledged"},
            {"requestId":"1","revision":1,"status":"already"},
            {"requestId":"2","revision":1,"status":"conflict","current":2},
            {"requestId":"3","revision":1,"status":"notFound"}
        ]});
        let result = Completed::read("actor".into(), &answers, document.clone()).unwrap();
        assert_eq!(
            result
                .items
                .iter()
                .map(|(_, status)| status.clone())
                .collect::<Vec<_>>(),
            vec![
                Outcome::Acknowledged,
                Outcome::Acknowledged,
                Outcome::Conflict(2),
                Outcome::NotFound
            ]
        );
        let mut wrong = document;
        wrong["items"][0]["revision"] = json!(2);
        assert!(Completed::read("actor".into(), &answers, wrong).is_err());
    }
    #[test]
    fn an_unrelated_success_does_not_erase_an_unresolved_failure() {
        let writer = Writer::default();
        writer.seen(&seen(1, 1));
        writer
            .offer()
            .unwrap()
            .complete(|_, _| Err("failed one".into()))
            .unwrap_err();
        writer.settled();
        writer.seen(&seen(2, 1));
        writer.offer().unwrap().complete(success).unwrap();
        writer.settled();
        assert_eq!(writer.failure().as_deref(), Some("failed one"));
        assert!(
            writer.offer().is_none(),
            "other work cannot retry the failed pair"
        );
        writer.seen(&seen(1, 1));
        writer.offer().unwrap().complete(success).unwrap();
        writer.settled();
        assert!(writer.failure().is_none());
    }
}
