//! Cursor-chain acquisition runs on the selected lane. No transcript or retention policy lives here.
use super::{Observation, Page, canonical_identity, page_with};
use crate::{board::conversation::ConversationKey, core::Core, runner::Cancellation};
use serde_json::Value;
use std::sync::Arc;

/// One admitted selected-lane read. Sharing the held page keeps frame-plan comparisons cheap.
#[derive(Clone, Debug)]
pub(in crate::board) struct AnswerRead {
    pub key: ConversationKey,
    pub before: Option<u64>,
    pub held: Option<Arc<Page>>,
    pub rebuild: bool,
}

impl PartialEq for AnswerRead {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.before == other.before
            && self.rebuild == other.rebuild
            && match (&self.held, &other.held) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}
impl Eq for AnswerRead {}

impl AnswerRead {
    pub fn read(&self, core: &Core, cancellation: &Cancellation) -> Observation {
        self.read_with(|operation, input| {
            (!cancellation.cancelled())
                .then(|| core.api(operation, input).ok())
                .flatten()
        })
    }

    pub(super) fn read_with(
        &self,
        mut call: impl FnMut(&str, Value) -> Option<Value>,
    ) -> Observation {
        if !canonical_identity(&self.key.1) {
            return Observation::Unavailable;
        }
        let Some(capabilities) = call("capabilities", serde_json::json!({})) else {
            return Observation::Unavailable;
        };
        if !capabilities["operations"]
            .as_array()
            .is_some_and(|ops| ops.iter().any(|operation| operation == "responses.recent"))
        {
            return Observation::Unavailable;
        }
        self.acquire(&mut call)
            .map_or(Observation::Unavailable, Observation::Observed)
    }

    fn acquire(&self, call: &mut impl FnMut(&str, Value) -> Option<Value>) -> Option<Arc<Page>> {
        let first = page_with(&self.key.1, self.before, call)?;
        if self.before.is_some() {
            let held = self.held.as_ref()?;
            if self.before != held.next_before {
                return None;
            }
            let mut page = first;
            page.items.extend(held.items.iter().cloned());
            return Some(Arc::new(page));
        }
        let mut pages = vec![first];
        let held = self.held.as_ref();
        let mut align = !self.rebuild && held.is_some_and(|held| !held.is_empty());
        loop {
            let page = pages.last()?;
            if align {
                let held = held?;
                let anchor = held.last()?;
                let matches: Vec<_> = page
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| *item == anchor)
                    .map(|(index, _)| index)
                    .collect();
                if let [position] = matches.as_slice() {
                    // Align once, rather than deleting repeated text throughout history.
                    if pages.len() == 1
                        && *position + 1 == page.items.len()
                        && held.capture == pages[0].capture
                        && held.coverage == pages[0].coverage
                        && (pages[0].next_before.is_some() || held.next_before.is_none())
                    {
                        return Some(Arc::clone(held));
                    }
                    let mut result = (**held).clone();
                    result
                        .items
                        .extend(page.items[position + 1..].iter().cloned());
                    for newer in pages[..pages.len() - 1].iter().rev() {
                        result.items.extend(newer.items.iter().cloned());
                    }
                    result.capture = pages[0].capture;
                    result.coverage = pages[0].coverage.clone();
                    if pages[0].next_before.is_none() {
                        result.next_before = None;
                    }
                    if result == **held {
                        return Some(Arc::clone(held));
                    }
                    return Some(Arc::new(result));
                }
                if matches.len() > 1 {
                    // Indistinguishable turns require a rebuild, not a later splice.
                    align = false;
                }
            }
            // A missing/ambiguous anchor or explicit refresh rebuilds only the
            // held window. Preserve the returned cursor for deliberate paging.
            let covered = held.is_none_or(|held| {
                pages.iter().map(|page| page.len()).sum::<usize>() >= held.len()
                    || page
                        .first()
                        .zip(held.first())
                        .is_some_and(|(oldest, held_oldest)| {
                            oldest.ended_ms <= held_oldest.ended_ms
                        })
            });
            if covered {
                return Some(Arc::new(join(pages)));
            }
            let Some(before) = page.next_before else {
                return Some(Arc::new(join(pages)));
            };
            if pages.len() == 20 {
                return None;
            }
            pages.push(page_with(&self.key.1, Some(before), call)?);
        }
    }
}

fn join(pages: Vec<Page>) -> Page {
    let capture = pages.first().and_then(|page| page.capture);
    let coverage = pages
        .last()
        .map(|page| page.coverage.clone())
        .unwrap_or_default();
    let next_before = pages.last().and_then(|page| page.next_before);
    Page {
        items: pages
            .into_iter()
            .rev()
            .flat_map(|page| page.items)
            .collect(),
        coverage,
        next_before,
        capture,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ID: &str = "11111111-1111-4111-8111-111111111111";

    fn request(held: Option<Arc<Page>>, before: Option<u64>, rebuild: bool) -> AnswerRead {
        AnswerRead {
            key: (Some("person".into()), ID.into()),
            held,
            before,
            rebuild,
        }
    }

    fn item(index: usize) -> Value {
        let text = format!("answer {index}");
        json!({"text":text,"endedMs":1000+index,"originalBytes":text.len(),"provider":"codex","sessionId":"session","truncated":false})
    }

    fn resource(end: usize, count: usize) -> Value {
        let floor = count.saturating_sub(1000);
        let start = end.saturating_sub(50).max(floor);
        let mut coverage = json!({"olderDropped":floor > 0});
        if floor > 0 {
            coverage["droppedThroughMs"] = json!(1000 + floor - 1);
        }
        let mut value = json!({"identityId":ID,"items":(start..end).rev().map(item).collect::<Vec<_>>(),"coverage":coverage,"capture":{"enabled":true}});
        if start > floor {
            value["nextBefore"] = json!(start);
        }
        assert!(end <= count);
        value
    }

    fn serve(read: &AnswerRead, count: usize, calls: &mut Vec<Value>) -> Observation {
        read.read_with(|operation, input| {
            if operation == "capabilities" {
                return Some(json!({"operations":["responses.recent"]}));
            }
            let end = input
                .get("before")
                .and_then(Value::as_u64)
                .map_or(count, |n| n as usize);
            calls.push(input);
            Some(resource(end, count))
        })
    }

    fn observed(observation: Observation) -> Arc<Page> {
        let Observation::Observed(page) = observation else {
            panic!("a complete page")
        };
        page
    }

    // Build held history through the same deliberate older-page requests as a user.
    fn held_history(count: usize, calls: &mut Vec<Value>) -> Arc<Page> {
        let mut page = observed(serve(&request(None, None, false), count, calls));
        while let Some(before) = page.next_before {
            page = observed(serve(
                &request(Some(page), Some(before), false),
                count,
                calls,
            ));
        }
        page
    }

    #[test]
    fn every_retained_answer_is_reachable_with_exact_server_cursors_and_no_eager_read() {
        let mut calls = Vec::new();
        let mut page = observed(serve(&request(None, None, false), 1000, &mut calls));
        assert_eq!(calls.len(), 1);
        assert_eq!(page.len(), 50);
        while let Some(before) = page.next_before {
            page = observed(serve(
                &request(Some(page), Some(before), false),
                1000,
                &mut calls,
            ));
        }
        assert_eq!(calls.len(), 20);
        assert_eq!(page.len(), 1000);
        for (i, item) in page.items.iter().enumerate() {
            assert_eq!(item.text, format!("answer {i}"));
        }
        assert!(!calls[0].as_object().unwrap().contains_key("before"));
        assert_eq!(calls[1]["before"], 950);
    }

    #[test]
    fn one_page_held_and_a_missing_anchor_rebuilds_only_one_page() {
        let mut calls = Vec::new();
        let held = observed(serve(&request(None, None, false), 1000, &mut calls));
        let mut rewritten = (*held).clone();
        rewritten.items[0].ended_ms = 0;
        let anchor = rewritten.items.last_mut().unwrap();
        anchor.text = "an answer replaced by Core".into();
        anchor.original_bytes = anchor.text.len() as u64;
        calls.clear();
        let rebuilt = observed(serve(
            &request(Some(Arc::new(rewritten)), None, false),
            1000,
            &mut calls,
        ));
        assert_eq!(
            calls.len(),
            1,
            "a missing anchor cannot eagerly page all history"
        );
        assert_eq!(rebuilt.len(), 50);
        assert_eq!(rebuilt.next_before, Some(950));
        assert_eq!(rebuilt[0].text, "answer 950");
        let older = observed(serve(
            &request(Some(rebuilt), Some(950), false),
            1000,
            &mut calls,
        ));
        assert_eq!(calls.len(), 2, "the next page needs deliberate demand");
        assert_eq!(older.len(), 100);
        assert_eq!(older.next_before, Some(900));
    }

    #[test]
    fn a_rebuild_stops_at_the_oldest_held_end_time_before_the_item_limit() {
        let held = held_history(200, &mut Vec::new());
        let mut calls = 0;
        let rebuilt = request(Some(held), None, true).read_with(|operation, _| {
            if operation == "capabilities" {
                return Some(json!({"operations":["responses.recent"]}));
            }
            calls += 1;
            let mut value = resource(1000, 1000);
            // Core preserves capture order even when the observed clock rolls back.
            value["items"].as_array_mut().unwrap().last_mut().unwrap()["endedMs"] = json!(900);
            Some(value)
        });
        let rebuilt = observed(rebuilt);
        assert_eq!(calls, 1);
        assert_eq!(rebuilt.len(), 50);
        assert_eq!(rebuilt[0].text, "answer 950");
        assert_eq!(rebuilt[0].ended_ms, 900);
        assert_eq!(rebuilt.next_before, Some(950));
    }

    #[test]
    fn explicit_refresh_without_held_history_reads_only_the_newest_page() {
        let mut calls = Vec::new();
        let page = observed(serve(&request(None, None, true), 1000, &mut calls));
        assert_eq!(calls.len(), 1);
        assert_eq!(page.len(), 50);
        assert_eq!(page.next_before, Some(950));
    }

    #[test]
    fn normal_refresh_splices_new_turns_without_rewalking_twenty_loaded_pages() {
        let mut calls = Vec::new();
        let held = held_history(1000, &mut calls);
        calls.clear();
        let refreshed = observed(serve(&request(Some(held), None, false), 1001, &mut calls));
        assert_eq!(calls.len(), 1, "the held newest turn is the splice anchor");
        assert_eq!(refreshed.len(), 1001, "no client-side retention rule");
        assert_eq!(refreshed.last().unwrap().text, "answer 1000");
        assert!(refreshed.coverage.older_dropped);
        assert_eq!(refreshed.coverage.dropped_through_ms, Some(1000));
        calls.clear();
        let rebuilt = observed(serve(
            &request(Some(refreshed), None, true),
            1001,
            &mut calls,
        ));
        assert_eq!(calls.len(), 20);
        assert_eq!(rebuilt.len(), 1000);
        assert_eq!(
            rebuilt[0].text, "answer 1",
            "Core, rather than a local expiry rule, replaces the retained window"
        );
    }

    #[test]
    fn explicit_refresh_and_an_unalignable_rewrite_rebuild_the_held_chain() {
        let mut calls = Vec::new();
        let held = held_history(120, &mut calls);
        calls.clear();
        let full = observed(serve(
            &request(Some(Arc::clone(&held)), None, true),
            120,
            &mut calls,
        ));
        assert_eq!(calls.len(), 3);
        assert_eq!(*full, *held);
        let mut calls = 0;
        let rewritten = request(Some(held), None, false).read_with(|operation, input| {
            if operation == "capabilities" {
                return Some(json!({"operations":["responses.recent"]}));
            }
            calls += 1;
            let end = input["before"].as_u64().unwrap_or(120) as usize;
            let mut value = resource(end, 120);
            for item in value["items"].as_array_mut().unwrap() {
                item["sessionId"] = json!("replacement");
            }
            Some(value)
        });
        assert_eq!(calls, 3);
        assert_eq!(observed(rewritten).len(), 120);
    }

    #[test]
    fn identical_turns_are_not_deduplicated_and_ambiguous_anchor_rebuilds() {
        let value =
            json!({"identityId":ID,"items":[item(7),item(7)],"coverage":{"olderDropped":false}});
        let held = Arc::new(super::super::parse(ID, &value).unwrap());
        let mut calls = 0;
        let result = request(Some(held), None, false).read_with(|operation, _| {
            if operation == "capabilities" {
                return Some(json!({"operations":["responses.recent"]}));
            }
            calls += 1;
            let mut page = value.clone();
            page["items"].as_array_mut().unwrap().push(item(7));
            Some(page)
        });
        assert_eq!(calls, 1);
        assert_eq!(observed(result).len(), 3);
    }

    #[test]
    fn a_different_provider_cannot_impersonate_the_held_anchor_at_a_page_boundary() {
        let old = item(7);
        let mut other_provider = old.clone();
        other_provider["provider"] = json!("claude");
        let held = Arc::new(
            super::super::parse(
                ID,
                &json!({"identityId":ID,"items":[old.clone()],"coverage":{"olderDropped":false}}),
            )
            .unwrap(),
        );
        let mut newest = (10..59).rev().map(item).collect::<Vec<_>>();
        newest.push(other_provider);
        let mut reads = Vec::new();
        let result=request(Some(held),None,false).read_with(|operation,input| {
            if operation=="capabilities" {return Some(json!({"operations":["responses.recent"]}));}
            reads.push(input.clone());
            Some(if input.get("before").is_none() {
                json!({"identityId":ID,"items":newest,"coverage":{"olderDropped":false},"nextBefore":1})
            } else {
                json!({"identityId":ID,"items":[old],"coverage":{"olderDropped":false}})
            })
        });
        let page = observed(result);
        assert_eq!(reads.len(), 1, "do not search beyond the held window");
        assert_eq!(page.len(), 50);
        assert_eq!(
            page[0].provider, "claude",
            "the other provider is not an anchor"
        );
        assert_eq!(page.next_before, Some(1));
        let older = request(Some(page), Some(1), false).read_with(|operation, input| {
            if operation == "capabilities" {
                return Some(json!({"operations":["responses.recent"]}));
            }
            reads.push(input);
            Some(json!({"identityId":ID,"items":[old],"coverage":{"olderDropped":false}}))
        });
        let older = observed(older);
        assert_eq!(reads.len(), 2);
        assert_eq!(reads[1]["before"], 1);
        assert_eq!(older.len(), 51);
        assert_eq!(older[0].provider, "codex");
        assert_eq!(older[1].provider, "claude");
    }

    #[test]
    fn failed_or_nonprogressing_page_never_returns_a_partial_replacement() {
        let held = observed(serve(&request(None, None, false), 100, &mut Vec::new()));
        for bad in [None, Some(resource(100, 100))] {
            let read = request(Some(Arc::clone(&held)), held.next_before, false);
            let result = read.read_with(|operation, _| {
                if operation == "capabilities" {
                    Some(json!({"operations":["responses.recent"]}))
                } else {
                    bad.clone()
                }
            });
            assert_eq!(result, Observation::Unavailable);
        }
        assert_eq!(held.len(), 50);
        assert_eq!(held.next_before, Some(50));
    }

    #[test]
    fn the_selected_worker_reads_one_older_cursor_and_publishes_one_coherent_chain() {
        use crate::board::{
            BoardEvent,
            refresh::{SelectedRead, Worker},
        };
        use std::{sync::mpsc, time::Duration};
        let root =
            std::env::temp_dir().join(format!("ops-answer-page-{}", crate::id::new_v4().unwrap()));
        std::fs::create_dir(&root).unwrap();
        let calls = root.join("calls");
        let executable = root.join("core");
        let output = resource(50, 100).to_string();
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" != api ]; then printf '%s\\n' '{{}}'; exit 0; fi\nrequest=$(cat)\nprintf '%s\\n' \"$request\" >> '{}'\ncase \"$request\" in\n*capabilities*) printf '%s\\n' '{{\"operations\":[\"responses.recent\"]}}';;\n*responses.recent*) printf '%s\\n' '{}';;\n*) exit 1;;\nesac\n",
            calls.display(),
            output
        );
        crate::test_support::write_ready_executable(&executable, &script);
        let held = observed(serve(&request(None, None, false), 100, &mut Vec::new()));
        let read = request(Some(held), Some(50), false);
        let (events, input) = mpsc::channel();
        let worker = Worker::spawn(Core::at(executable), false, events, None);
        worker.selected(41, Some(SelectedRead::PaneOutput(read.clone())));
        let BoardEvent::PaneOutput {
            read: delivered,
            revision,
            observation,
            cancellation,
        } = input.recv_timeout(Duration::from_secs(15)).unwrap()
        else {
            panic!("one page projection")
        };
        assert_eq!(delivered, read);
        assert_eq!(revision, 41);
        assert!(!cancellation.cancelled());
        let page = observed(observation);
        assert_eq!(page.len(), 100);
        assert_eq!(page.next_before, None);
        assert_eq!(page[0].text, "answer 0");
        assert_eq!(page[99].text, "answer 99");
        drop(worker);
        assert_eq!(input.into_iter().count(), 0, "no intermediate publications");
        let calls = std::fs::read_to_string(calls)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[1]["input"],
            json!({"identityId":ID,"before":50,"limit":50})
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
