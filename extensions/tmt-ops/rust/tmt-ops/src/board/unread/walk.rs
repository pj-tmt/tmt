//! One public answers page per ordered-lane step; no history cache or per-item show.
use super::{
    BodyKind, Page,
    publish::{Completed, Outcome},
};
use crate::{config::Config, core::Core};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Immutable provenance of an answer body in a successful frame. A revision is
/// optional when the body predates the bounded summary page; the lane resolves it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Body {
    pub actor: String,
    pub member: String,
    pub request: String,
    pub kind: BodyKind,
    pub at_ms: u64,
    pub revision: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Goal {
    Legacy(BTreeMap<String, u64>),
    Member(String),
    Body(Body),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Walk {
    pub actor: String,
    pub goal: Goal,
    before: Option<Value>,
}
impl Walk {
    pub fn new(actor: String, goal: Goal) -> Self {
        Self {
            actor,
            goal,
            before: None,
        }
    }
    pub fn after(mut self, before: Value) -> Self {
        self.before = Some(before);
        self
    }
    pub fn same_goal(&self, other: &Self) -> bool {
        self.actor == other.actor && self.goal == other.goal
    }

    pub fn step(&self, core: &Core) -> Result<(Completed, Option<Self>), String> {
        let mut walk = self.clone();
        if walk.before.is_none()
            && let Goal::Legacy(cutoffs) = &mut walk.goal
        {
            let markers = Config::load(core)
                .and_then(|config| config.read_markers())
                .map_err(|error| error.to_string())?;
            // A delayed snapshot cannot restart a consumed conversation, or inherit
            // a concurrently changed cutoff. No persistent migration flag is needed.
            cutoffs.retain(|member, cutoff| markers.get(&walk.actor, member) == *cutoff);
            if cutoffs.is_empty() {
                return Ok((
                    Completed {
                        actor: walk.actor,
                        items: Vec::new(),
                    },
                    None,
                ));
            }
        }
        let mut input =
            json!({"originatorId":self.actor,"view":"answers","limit":super::ANSWERS_LIMIT});
        if let Some(before) = &self.before {
            input["before"] = before.clone();
        }
        let document = core
            .api("requests.list", input)
            .map_err(|error| error.to_string())?;
        walk.read_page(core, document)
    }
    fn read_page(&self, core: &Core, document: Value) -> Result<(Completed, Option<Self>), String> {
        let page = Page::read(&self.actor, &document)
            .ok_or("requests.list returned malformed answer metadata")?;
        let mut found = false;
        let answers = page
            .replies
            .values()
            .flatten()
            .filter(|answer| {
                let eligible = match &self.goal {
                    // Cutoff compares answeredAtMs; conservative by construction: a capture
                    // settled after the watermark stays unread even if its turn ended earlier.
                    Goal::Legacy(cutoffs) => cutoffs
                        .get(&answer.member)
                        .is_some_and(|cutoff| answer.answered_at_ms <= *cutoff),
                    Goal::Member(member) => &answer.member == member,
                    Goal::Body(body) => {
                        if answer.member == body.member && answer.request == body.request {
                            found = true;
                            answer.kind == Some(body.kind)
                                && answer.answered_at_ms == body.at_ms
                                && body
                                    .revision
                                    .is_none_or(|revision| revision == answer.revision)
                        } else {
                            false
                        }
                    }
                };
                eligible && !answer.acknowledged
            })
            .cloned()
            .collect::<Vec<_>>();
        let completed = super::publish::acknowledge(core, &self.actor, &answers)?;
        if matches!(self.goal, Goal::Legacy(_))
            && completed
                .items
                .iter()
                .any(|(_, outcome)| matches!(outcome, Outcome::Conflict(_)))
        {
            return Ok((completed, Some(self.clone())));
        }
        let next = if found { None } else { page.next_before };
        if let Some(next) = next {
            if let Some(before) = &self.before {
                let key = |cursor: &Value| {
                    (
                        cursor["answeredAtMs"].as_u64().unwrap_or(0),
                        cursor["requestId"].as_str().unwrap_or("").to_owned(),
                    )
                };
                if key(&next) >= key(before) {
                    return Err("Answer continuation did not advance".into());
                }
            }
            let mut walk = self.clone();
            walk.before = Some(next);
            Ok((completed, Some(walk)))
        } else {
            if let Goal::Legacy(cutoffs) = &self.goal {
                Config::load(core)
                    .and_then(|mut config| config.consume_read_markers(&self.actor, cutoffs))
                    .map_err(|error| error.to_string())?;
            }
            Ok((completed, None))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::Fixture;
    use super::*;
    use std::fs;
    const ACTOR: &str = "33333333-3333-4333-8333-333333333333";
    const MEMBER: &str = "22222222-2222-4222-8222-222222222222";
    fn answer(id: &str, at: u64, kind: &str) -> Value {
        json!({"requestId":id,"kind":"request","sender":{"identityId":ACTOR},
            "recipientId":MEMBER,"answeredAtMs":at,"answer":kind,"revision":1,"acknowledged":false})
    }
    fn legacy(fixture: &Fixture) -> Walk {
        fs::write(
            fixture.0.join("ops.toml"),
            format!("# authored preferences\n[workspace.read_markers.{ACTOR}]\n{MEMBER}=100\n"),
        )
        .unwrap();
        Walk::new(
            ACTOR.into(),
            Goal::Legacy(BTreeMap::from([(MEMBER.into(), 100)])),
        )
    }
    fn pages(fixture: &Fixture) {
        fs::write(fixture.0.join("pages.json"),json!({
            "":{"items":[answer("new-turn",101,"turn"),answer("equal",100,"final")],"nextBefore":{"answeredAtMs":100,"requestId":"equal"}},
            "equal":{"items":[answer("old",90,"turn")],"nextBefore":null}
        }).to_string()).unwrap();
    }
    #[test]
    fn legacy_walk_yields_per_page_and_uses_answer_settle_cutoff_then_consumes() {
        let fixture = Fixture::new();
        let walk = legacy(&fixture);
        pages(&fixture);
        let (first, next) = walk.step(&fixture.core()).unwrap();
        assert_eq!(first.items.len(), 1);
        assert_eq!(first.items[0].0.request, "equal");
        assert_eq!(
            fixture.config().read_markers().unwrap().get(ACTOR, MEMBER),
            100,
            "partial walk is not migrated"
        );
        let (second, next) = next.unwrap().step(&fixture.core()).unwrap();
        assert!(next.is_none());
        assert_eq!(second.items[0].0.request, "old");
        assert_eq!(
            fixture.config().read_markers().unwrap().get(ACTOR, MEMBER),
            0
        );
        let acked: Value =
            serde_json::from_slice(&fs::read(fixture.0.join("acknowledged.json")).unwrap())
                .unwrap();
        assert_eq!(acked, json!({"equal":1,"old":1}));
        let calls = fs::read_to_string(fixture.0.join("calls")).unwrap();
        let calls = calls
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            calls
                .iter()
                .filter(|call| call["operation"] == "requests.list")
                .count(),
            2
        );
        assert!(
            !calls
                .iter()
                .any(|call| call["operation"] == "requests.show")
        );
        assert!(
            calls
                .iter()
                .filter(|call| call["operation"] == "requests.ack")
                .all(|call| call["identity"] == ACTOR)
        );
    }
    #[test]
    fn partial_failure_keeps_legacy_entry_and_restart_repeats_idempotently() {
        let fixture = Fixture::new();
        let walk = legacy(&fixture);
        fs::write(fixture.0.join("pages.json"),json!({"":{"items":[answer("one",90,"final"),answer("two",80,"turn")],"nextBefore":null}}).to_string()).unwrap();
        fs::write(fixture.0.join("partial"), "").unwrap();
        assert!(walk.step(&fixture.core()).is_err());
        assert_eq!(
            fixture.config().read_markers().unwrap().get(ACTOR, MEMBER),
            100
        );
        fs::remove_file(fixture.0.join("partial")).unwrap();
        let (completed, next) = walk.step(&fixture.core()).unwrap();
        assert!(next.is_none());
        assert_eq!(
            completed.items.len(),
            1,
            "the already durable item is not re-acked"
        );
        assert_eq!(
            fixture.config().read_markers().unwrap().get(ACTOR, MEMBER),
            0
        );
        let acked: Value =
            serde_json::from_slice(&fs::read(fixture.0.join("acknowledged.json")).unwrap())
                .unwrap();
        assert_eq!(acked, json!({"one":1,"two":1}));
    }
    #[test]
    fn conflict_keeps_legacy_cutoff_and_new_revision_unread() {
        let fixture = Fixture::new();
        let walk = legacy(&fixture);
        pages(&fixture);
        fs::write(
            fixture.0.join("outcomes.json"),
            json!({"equal":{"status":"conflict","current":2}}).to_string(),
        )
        .unwrap();
        let (completed, next) = walk.step(&fixture.core()).unwrap();
        assert!(next.is_some());
        assert!(
            matches!(completed.items[0].1, Outcome::Conflict(2)),
            "the UI must adopt current even when migration pauses"
        );
        assert_eq!(
            fixture.config().read_markers().unwrap().get(ACTOR, MEMBER),
            100
        );
    }
    #[test]
    fn older_visible_body_resolves_exact_source_without_a_show_or_count_cache() {
        let fixture = Fixture::new();
        pages(&fixture);
        let walk = Walk::new(
            ACTOR.into(),
            Goal::Body(Body {
                actor: ACTOR.into(),
                member: MEMBER.into(),
                request: "old".into(),
                kind: BodyKind::Turn,
                at_ms: 90,
                revision: None,
            }),
        );
        let (first, next) = walk.step(&fixture.core()).unwrap();
        assert!(first.items.is_empty());
        let (second, next) = next.unwrap().step(&fixture.core()).unwrap();
        assert!(next.is_none());
        assert_eq!(second.items[0].0.request, "old");
        let stale = Walk::new(
            ACTOR.into(),
            Goal::Body(Body {
                actor: ACTOR.into(),
                member: MEMBER.into(),
                request: "new-turn".into(),
                kind: BodyKind::Final,
                at_ms: 101,
                revision: None,
            }),
        );
        let (result, next) = stale.step(&fixture.core()).unwrap();
        assert!(result.items.is_empty());
        assert!(
            next.is_none(),
            "a mismatched current source never authorizes an older body"
        );
    }
    #[test]
    fn explicit_member_walk_continues_beyond_the_summary_and_no_watermark_is_needed() {
        let fixture = Fixture::new();
        pages(&fixture);
        let walk = Walk::new(ACTOR.into(), Goal::Member(MEMBER.into()))
            .after(json!({"answeredAtMs":100,"requestId":"equal"}));
        let (result, next) = walk.step(&fixture.core()).unwrap();
        assert!(next.is_none());
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].0.request, "old");
        assert_eq!(
            fs::read_to_string(fixture.0.join("ops.toml")).unwrap(),
            "# authored preferences\n"
        );
    }
    #[test]
    fn a_stale_snapshot_or_absent_cutoff_cannot_restart_legacy_migration() {
        let fixture = Fixture::new();
        let walk = legacy(&fixture);
        pages(&fixture);
        let (_, next) = walk.step(&fixture.core()).unwrap();
        next.unwrap().step(&fixture.core()).unwrap();
        let before = fs::read_to_string(fixture.0.join("calls")).unwrap();
        let (completed, next) = walk.step(&fixture.core()).unwrap();
        assert!(completed.items.is_empty());
        assert!(next.is_none());
        assert_eq!(
            fs::read_to_string(fixture.0.join("calls")).unwrap(),
            before,
            "no answer walk or ack after the cutoff is gone"
        );
    }
    #[test]
    fn shutdown_does_not_start_another_history_page_and_restart_resumes_the_cutoff() {
        use super::super::publish::Writer;
        let fixture = Fixture::new();
        let _ = legacy(&fixture);
        pages(&fixture);
        let writer = Writer::default();
        writer.legacy(ACTOR, &fixture.config().read_markers().unwrap());
        let completed = writer.offer().unwrap().publish(&fixture.core()).unwrap();
        assert_eq!(completed.items.len(), 1);
        let before = fs::read_to_string(fixture.0.join("calls")).unwrap();
        writer.finish(&fixture.core()).unwrap();
        assert_eq!(
            fs::read_to_string(fixture.0.join("calls")).unwrap(),
            before,
            "shutdown joins the admitted page but never starts a continuation"
        );
        assert_eq!(
            fixture.config().read_markers().unwrap().get(ACTOR, MEMBER),
            100
        );
        let restarted = Writer::default();
        restarted.legacy(ACTOR, &fixture.config().read_markers().unwrap());
        while let Some(job) = restarted.offer() {
            job.publish(&fixture.core()).unwrap();
            restarted.settled();
        }
        assert_eq!(
            fixture.config().read_markers().unwrap().get(ACTOR, MEMBER),
            0
        );
    }
}
