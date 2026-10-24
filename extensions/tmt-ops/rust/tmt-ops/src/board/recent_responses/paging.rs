//! Cursor-chain acquisition runs on the selected lane. No transcript or retention policy lives here.
use super::{Coverage, Item, Observation, Page, canonical_identity, page_with};
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
        let align = !self.rebuild
            && held.is_some_and(|held| !held.is_empty() && held.coverage == pages[0].coverage);
        loop {
            let page = pages.last()?;
            if align {
                let held = held?;
                let anchor = held.last()?;
                let matches: Vec<_> = page
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| same_anchor(item, anchor))
                    .map(|(index, _)| index)
                    .collect();
                if let [position] = matches.as_slice() {
                    // Align once, rather than deleting repeated text throughout history.
                    if pages.len() == 1
                        && *position + 1 == page.items.len()
                        && held.capture == pages[0].capture
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
                    if result == **held {
                        return Some(Arc::clone(held));
                    }
                    return Some(Arc::new(result));
                }
                if matches.len() > 1 {
                    // Indistinguishable anchor turns cannot establish a splice boundary.
                    break;
                }
            } else if held.is_none() && !self.rebuild {
                return Some(Arc::new(pages.pop()?));
            }
            let Some(before) = page.next_before else {
                return Some(Arc::new(join(pages)));
            };
            if pages.len() == 20 {
                return None;
            }
            pages.push(page_with(&self.key.1, Some(before), call)?);
        }
        // A gap, retention cut, ambiguous/missing anchor or explicit refresh rebuilds
        // the chain. Core's public maximum is 1,000 answers, 50 per page. The bound
        // refuses an invalid resource; it never evicts a held answer locally.
        while let Some(before) = pages.last()?.next_before {
            if pages.len() == 20 {
                return None;
            }
            pages.push(page_with(&self.key.1, Some(before), call)?);
        }
        Some(Arc::new(join(pages)))
    }
}

fn same_anchor(a: &Item, b: &Item) -> bool {
    (a.ended_ms, a.original_bytes, &a.text, &a.session)
        == (b.ended_ms, b.original_bytes, &b.text, &b.session)
}

fn join(pages: Vec<Page>) -> Page {
    let capture = pages.first().and_then(|page| page.capture);
    let coverage = pages
        .last()
        .map(|page| page.coverage.clone())
        .unwrap_or_else(Coverage::default);
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
        json!({"text":text,"endedMs":index,"originalBytes":text.len(),"provider":"codex","sessionId":"session","truncated":false})
    }

    fn resource(end: usize, count: usize) -> Value {
        let mut value = json!({"identityId":ID,"items":(end.saturating_sub(50)..end).rev().map(item).collect::<Vec<_>>(),"coverage":{"olderDropped":false},"capture":{"enabled":true}});
        if end > 50 {
            value["nextBefore"] = json!(end - 50);
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
    fn normal_refresh_splices_new_turns_without_rewalking_twenty_loaded_pages() {
        let mut calls = Vec::new();
        let held = observed(serve(&request(None, None, true), 1000, &mut calls));
        calls.clear();
        let refreshed = observed(serve(&request(Some(held), None, false), 1001, &mut calls));
        assert_eq!(calls.len(), 1, "the held newest turn is the splice anchor");
        assert_eq!(refreshed.len(), 1001, "no client-side retention rule");
        assert_eq!(refreshed.last().unwrap().text, "answer 1000");
    }

    #[test]
    fn explicit_refresh_and_an_unalignable_rewrite_rebuild_the_chain() {
        let mut calls = Vec::new();
        let held = observed(serve(&request(None, None, true), 120, &mut calls));
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
