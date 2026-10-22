//! One selected refresh: acquire on the existing worker, then admit one projection.
//! Full request documents live only for this refresh; Session retains the presentation.
use super::{
    app::App,
    conversation::ConversationKey,
    home_leads::{FinalsKey, Kind, MessageKey, SentKey, SentTexts},
    recent_responses,
    row_detail::Target,
};
use crate::{core::Core, runner::Cancellation};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Message {
    key: MessageKey,
    at_ms: Option<u64>,
    acquired: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Plan {
    key: FinalsKey,
    squad: String,
    finals: bool,
    sent: bool,
    messages: Vec<Message>,
    pane: Option<ConversationKey>,
}

pub(super) struct Projection {
    finals: Option<Result<Vec<Value>, String>>,
    sent: Option<Result<Vec<Value>, String>>,
    bodies: Vec<(MessageKey, Result<String, String>)>,
    pane: Option<recent_responses::Observation>,
}

impl Plan {
    #[cfg(test)]
    pub(super) fn test_message_key(&self) -> &MessageKey {
        &self.messages[0].key
    }

    #[cfg(test)]
    pub(super) fn test_wrong_sender(mut self) -> Self {
        self.key.sender = "another sender".into();
        self
    }

    #[cfg(test)]
    pub(super) fn test_projection(&self, body: &str, pane: Option<&str>) -> Projection {
        Projection {
            finals: self.finals.then(|| Ok(Vec::new())),
            sent: self.sent.then(|| Ok(Vec::new())),
            bodies: self
                .messages
                .iter()
                .filter(|message| !message.acquired)
                .map(|message| (message.key.clone(), Ok(body.into())))
                .collect(),
            pane: self.pane.as_ref().map(|_| {
                pane.map_or(recent_responses::Observation::Unavailable, |text| {
                    recent_responses::Observation::Observed(vec![recent_responses::Item {
                        provider: "codex".into(),
                        text: text.into(),
                        ended_ms: crate::status::now_ms().saturating_sub(60_000),
                        truncated: false,
                    }])
                })
            }),
        }
    }

    pub(super) fn read(
        &self,
        core: &Core,
        sent_texts: &mut SentTexts,
        cancellation: &Cancellation,
    ) -> Projection {
        let finals = self
            .finals
            .then(|| self.key.read(core).map_err(|error| error.to_string()));
        let mut documents = BTreeMap::new();
        let mut show = |request: &str| {
            documents
                .entry(request.to_owned())
                .or_insert_with(|| crate::requests::show_request(core, request))
                .clone()
        };
        let sent = (self.sent && !cancellation.cancelled()).then(|| {
            SentKey {
                sender: self.key.sender.clone(),
                agent: self.key.agent.clone(),
            }
            .read_with(core, sent_texts, &mut show)
            .map_err(|error| error.to_string())
        });
        let mut messages = self.messages.clone();
        if let Some(Ok(finals)) = &finals {
            for reply in finals {
                let Some(request) = reply["requestId"].as_str() else {
                    continue;
                };
                let key = MessageKey {
                    sender: self.key.sender.clone(),
                    lead: self.key.agent.clone(),
                    squad: self.squad.clone(),
                    request: request.into(),
                    kind: Kind::Reply,
                };
                if !messages.iter().any(|message| message.key.same_source(&key)) {
                    messages.push(Message {
                        key,
                        at_ms: reply["submittedAtMs"].as_u64(),
                        acquired: false,
                    });
                }
            }
        }
        // Match the visible finals window after merging the new targeted page with
        // held sources. Questions have their independent bounded inbox ownership.
        messages.sort_by(|a, b| (a.at_ms, &a.key.request).cmp(&(b.at_ms, &b.key.request)));
        let mut finals_left = super::row_detail::FINALS;
        messages.reverse();
        messages.retain(|message| {
            if message.key.kind != Kind::Reply {
                return true;
            }
            if finals_left == 0 {
                return false;
            }
            finals_left -= 1;
            true
        });
        let mut bodies = Vec::new();
        for message in messages.into_iter().filter(|message| !message.acquired) {
            if cancellation.cancelled() {
                break;
            }
            let body = show(&message.key.request)
                .and_then(|document| message.key.body(&document))
                .map_err(|error| error.to_string());
            bodies.push((message.key, body));
        }
        let pane = self
            .pane
            .as_ref()
            .filter(|_| !cancellation.cancelled())
            .map(|key| recent_responses::read(core, &key.1));
        Projection {
            finals,
            sent,
            bodies,
            pane,
        }
    }
}

impl App {
    pub(super) fn conversation_read(&self) -> Option<Plan> {
        let key = self.conversation_finals()?;
        let detail = self.detail(&Target::Row(self.conversation_target()?))?;
        let finals = self.finals_read().is_some();
        let sent = finals || self.sent_read().is_some();
        let messages: Vec<_> = detail
            .messages()
            .into_iter()
            .filter_map(|reply| {
                Some(Message {
                    key: reply.key.clone()?,
                    at_ms: reply.at_ms,
                    acquired: reply.acquired,
                })
            })
            .collect();
        let pane = self.pane_output_read();
        if !finals && !sent && pane.is_none() && messages.iter().all(|message| message.acquired) {
            return None;
        }
        Some(Plan {
            key,
            squad: self
                .target_squad(&self.conversation_target()?)
                .unwrap_or_default(),
            finals,
            sent,
            messages,
            pane,
        })
    }

    pub(super) fn apply_conversation(&mut self, plan: Plan, projection: Projection) {
        // The event owner checks the entire plan, revision and cancellation before
        // admission. Apply without painting between parts, under the same source checks
        // used by compact details and readers.
        if let Some(result) = projection.finals {
            self.apply_finals(plan.key.clone(), result);
        }
        if let Some(result) = projection.sent {
            self.apply_sent(
                SentKey {
                    sender: plan.key.sender,
                    agent: plan.key.agent,
                },
                result,
            );
        }
        for (key, body) in projection.bodies {
            self.apply_message(&key, body);
        }
        if let Some((key, observation)) = plan.pane.zip(projection.pane) {
            self.apply_pane_output(key, observation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{sync::mpsc, time::Duration};

    fn message(request: String, kind: Kind, at_ms: u64) -> Message {
        Message {
            key: MessageKey {
                sender: "owner".into(),
                lead: "agent".into(),
                squad: "product".into(),
                request,
                kind,
            },
            at_ms: Some(at_ms),
            acquired: false,
        }
    }

    #[test]
    fn a_typical_refresh_reads_each_request_once_and_publishes_one_complete_projection() {
        let dir = std::env::temp_dir().join(format!(
            "ops-conversation-refresh-{}",
            crate::id::new_v4().unwrap()
        ));
        std::fs::create_dir(&dir).unwrap();
        let calls = dir.join("calls");
        let page: Vec<_> = (0..8)
            .map(|i| {
                json!({"requestId":format!("final-{i}"),"kind":"request",
                "sender":{"identityId":"owner"},"recipientId":"agent",
                "preparedAtMs":i,"preview":"prompt preview",
                "final":{"status":"retained","submittedAtMs":i}})
            })
            .collect();
        let mut script = format!(
            "#!/bin/sh\nif [ \"$1\" != api ]; then printf '%s\\n' '{{}}'; exit 0; fi\nrequest=$(cat)\nprintf '%s\\n' \"$request\" >> '{}'\ncase \"$request\" in\n*requests.list*) printf '%s\\n' '{}';;\n",
            calls.display(),
            json!({"items":page,"nextBefore":null})
        );
        for (prefix, count) in [("final", 8), ("question", 10)] {
            for i in 0..count {
                let question = prefix == "question";
                let detail = json!({"requestId":format!("{prefix}-{i}"),"kind":"request",
                    "sender":{"identityId":if question {"agent"} else {"owner"}},
                    "recipientId":if question {"owner"} else {"agent"},
                    "prompt":{"status":"retained","message":format!("exact prompt {prefix}-{i}\r\n")},
                    "final":{"status":"retained","response":format!("exact reply {i}\r\n")}});
                script.push_str(&format!("*{prefix}-{i}*) printf '%s\\n' '{}';;\n", detail));
            }
        }
        script.push_str("*) printf '%s\\n' '{}';;\nesac\n");
        let fake = dir.join("tmt");
        crate::test_support::write_ready_executable(&fake, &script);
        let plan = Plan {
            key: FinalsKey {
                sender: "owner".into(),
                agent: "agent".into(),
            },
            squad: "product".into(),
            finals: true,
            sent: true,
            messages: (0..10)
                .map(|i| message(format!("question-{i}"), Kind::Question, i + 10))
                .collect(),
            pane: None,
        };
        let (events, input) = mpsc::channel();
        let worker = super::super::refresh::Worker::spawn(Core::at(fake), false, events, None);
        worker.selected(
            23,
            Some(super::super::refresh::SelectedRead::Conversation(
                plan.clone(),
            )),
        );
        let event = input.recv_timeout(Duration::from_secs(30)).unwrap();
        let super::super::BoardEvent::Conversation {
            plan: delivered,
            projection,
            revision,
            cancellation,
        } = event
        else {
            panic!("one conversation completion, no intermediate body/list events")
        };
        assert_eq!(delivered, plan);
        assert_eq!(revision, 23);
        assert!(!cancellation.cancelled());
        assert_eq!(projection.finals.unwrap().unwrap().len(), 8);
        let sent = projection.sent.unwrap().unwrap();
        assert_eq!(sent.len(), 8);
        assert_eq!(
            sent.iter()
                .filter(|send| send["truncated"] == false)
                .count(),
            5
        );
        assert_eq!(projection.bodies.len(), 18);
        for (key, body) in projection.bodies {
            let text = body.unwrap();
            assert!(text.ends_with("\r\n"), "exact source bytes");
            assert_eq!(
                text,
                if key.kind == Kind::Question {
                    format!("exact prompt {}\r\n", key.request)
                } else {
                    format!(
                        "exact reply {}\r\n",
                        key.request.strip_prefix("final-").unwrap()
                    )
                }
            );
        }
        drop(worker);
        assert_eq!(input.into_iter().count(), 0, "exactly one publication");
        let calls: Vec<Value> = std::fs::read_to_string(&calls)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let shows: Vec<_> = calls
            .iter()
            .filter(|call| call["operation"] == "requests.show")
            .collect();
        assert_eq!(shows.len(), 18);
        let ids: std::collections::BTreeSet<_> = shows
            .iter()
            .map(|call| call["input"]["requestId"].as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), 18, "no prompt/body double reads");
        assert_eq!(
            calls
                .iter()
                .filter(|call| call["operation"] == "requests.list")
                .count(),
            2,
            "targeted list remains: no coverage proof was supplied"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn a_reused_document_still_verifies_source_and_an_expired_body_is_not_retried() {
        let dir = std::env::temp_dir().join(format!(
            "ops-conversation-source-{}",
            crate::id::new_v4().unwrap()
        ));
        std::fs::create_dir(&dir).unwrap();
        let fake = dir.join("tmt");
        let calls = dir.join("calls");
        let detail = json!({"requestId":"same","kind":"request",
            "sender":{"identityId":"agent"},"recipientId":"owner",
            "prompt":{"status":"expired"},
            "final":{"status":"retained","response":"not this direction"}});
        crate::test_support::write_ready_executable(
            &fake,
            &format!(
                "#!/bin/sh\nrequest=$(cat)\nprintf '%s\\n' \"$request\" >> '{}'\nprintf '%s\\n' '{}'\n",
                calls.display(),
                detail
            ),
        );
        let plan = Plan {
            key: FinalsKey {
                sender: "owner".into(),
                agent: "agent".into(),
            },
            squad: "product".into(),
            finals: false,
            sent: false,
            pane: None,
            messages: vec![
                message("same".into(), Kind::Reply, 1),
                message("same".into(), Kind::Question, 2),
            ],
        };
        let projection = plan.read(
            &Core::at(fake.clone()),
            &mut SentTexts::default(),
            &Cancellation::default(),
        );
        assert_eq!(projection.bodies.len(), 2);
        for (key, body) in projection.bodies {
            let error = body.unwrap_err();
            assert!(error.contains(if key.kind == Kind::Question {
                "expired"
            } else {
                "no longer matches"
            }));
        }
        let calls = std::fs::read_to_string(calls).unwrap();
        assert_eq!(
            calls.lines().count(),
            1,
            "a refused source is never fetched twice in one refresh"
        );
        let cancelled = Cancellation::default();
        cancelled.cancel();
        let cancelled_projection =
            plan.read(&Core::at(fake), &mut SentTexts::default(), &cancelled);
        assert!(cancelled_projection.bodies.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
