use super::*;
use std::collections::BTreeSet;

fn lead(id: &str, squad: &str) -> Lead {
    Lead {
        squad: squad.into(),
        row: json!({"id":id,"name":format!("lead-{id}"),"waitingOnYou":[]}),
        exchange: None,
        failure: None,
    }
}
fn ask(id: &str, recipient: &str, at: u64) -> Value {
    json!({"requestId":id,"recipientId":recipient,"kind":"request","sender":{"identityId":"me"},"preparedAtMs":at,"preview":"my question","recipientAcknowledged":true,"final":{"status":"not_submitted"}})
}
fn reply(id: &str, recipient: &str, at: u64, preview: Option<&str>) -> Value {
    let mut value = ask(id, recipient, 1);
    value["final"] = json!({"status":"retained","submittedAtMs":at});
    if let Some(preview) = preview {
        value["responsePreview"] = json!(preview);
    }
    value
}
fn fetch(leads: Vec<Lead>) -> Fetch {
    Fetch {
        sender: Some(Me {
            id: "me".into(),
            name: "user".into(),
        }),
        leads,
    }
}

fn occurrences(leads: &[Lead]) -> Vec<(&str, &str)> {
    leads
        .iter()
        .map(|lead| (lead.id(), lead.squad.as_str()))
        .collect()
}

fn observation_home(leads: Vec<Lead>) -> super::super::home::Home {
    super::super::home::Home {
        workspace: Default::default(),
        window: crate::config::TokenWindow::MINUTE,
        windows: crate::config::TokenWindow::DEFAULTS,
        reduced_motion: false,
        summary: Default::default(),
        squads: leads
            .into_iter()
            .map(|lead| super::super::home::SquadLine {
                squad: lead.squad,
                lead: Some(lead.row),
                counts: Default::default(),
            })
            .collect(),
        failures: vec![],
        incomplete: false,
    }
}

#[test]
fn an_empty_exchange_set_requires_an_admitted_read() {
    let mut state = State::default();
    state.set_sender(Some("me"));
    assert!(!state.observations_complete(), "never read is unavailable");
    state.replace(fetch(vec![]).read(|_| Ok(json!({"items":[]})), 100));
    assert!(
        state.observations_complete(),
        "a successful empty result is known"
    );
    assert!(state.leads.is_empty() && state.replies.is_empty());
    state.set_sender(None);
    state.replace(Fetch::results(None).read(|_| panic!("no sender must not read"), 100));
    assert!(!state.observations_complete());
}

#[test]
fn failed_result_or_lead_pages_are_not_complete_observations() {
    for failed_results in [true, false] {
        let mut state = State::default();
        state.reconcile(&observation_home(vec![lead("a", "squad")]), Some("me"));
        state.replace(fetch(vec![lead("a", "squad")]).read(|_| Ok(json!({"items":[]})), 100));
        assert!(state.observations_complete());
        let read = fetch(vec![lead("a", "squad")]).read(
            |input| {
                if (input["view"] == "results") == failed_results {
                    Err(SquadError::new("READ_FAILED", "source unavailable"))
                } else {
                    Ok(json!({"items":[]}))
                }
            },
            200,
        );
        state.replace(read);
        assert!(!state.observations_complete());
        assert_eq!(
            state.lead_observed.get(&("squad".into(), "a".into())),
            Some(&true)
        );
        state.replace(fetch(vec![lead("a", "squad")]).read(|_| Ok(json!({"items":[]})), 300));
        assert!(state.observations_complete());
    }
}

#[test]
fn sender_changes_and_late_reads_cannot_mark_another_users_sources() {
    let home = observation_home(vec![lead("a", "squad")]);
    for via_reconcile in [false, true] {
        let mut state = State::default();
        state.reconcile(&home, Some("me"));
        state.replace(fetch(vec![lead("a", "squad")]).read(|_| Ok(json!({"items":[]})), 100));
        assert!(state.observations_complete());
        let late = fetch(vec![lead("a", "squad")]).read(|_| Ok(json!({"items":[]})), 200);
        if via_reconcile {
            state.reconcile(&home, Some("other"));
        } else {
            state.set_sender(Some("other"));
        }
        assert!(!state.results_observed);
        assert!(!state.observations_complete());
        state.replace(late);
        assert!(!state.results_observed);
        assert!(!state.observations_complete());
    }
}

#[test]
fn each_current_lead_occurrence_needs_its_own_page_observation() {
    let mut state = State::default();
    state.reconcile(&observation_home(vec![lead("a", "one")]), Some("me"));
    state.replace(fetch(vec![lead("a", "one")]).read(|_| Ok(json!({"items":[]})), 100));
    assert!(state.observations_complete());
    let late_old_roster = fetch(vec![lead("a", "one")]).read(|_| Ok(json!({"items":[]})), 200);
    state.reconcile(
        &observation_home(vec![lead("a", "one"), lead("b", "two")]),
        Some("me"),
    );
    assert!(
        !state.observations_complete(),
        "a newly added lead is unobserved"
    );
    state.replace(late_old_roster);
    assert!(
        !state.observations_complete(),
        "an older page cannot mark the new lead"
    );
    state.reconcile(
        &observation_home(vec![lead("a", "one"), lead("b", "two")]),
        Some("me"),
    );
    state.replace(
        fetch(vec![lead("a", "one"), lead("b", "two")]).read(|_| Ok(json!({"items":[]})), 300),
    );
    assert!(state.observations_complete());
    state.reconcile(&observation_home(vec![lead("b", "two")]), Some("me"));
    assert!(
        state.observations_complete(),
        "removing a lead preserves retained evidence"
    );
    state.reconcile(&observation_home(vec![lead("c", "two")]), Some("me"));
    assert!(
        !state.observations_complete(),
        "a replaced lead is unobserved"
    );
}

#[test]
fn a_lead_that_leaves_and_returns_does_not_reuse_its_old_mark() {
    let mut state = State::default();
    state.reconcile(&observation_home(vec![lead("a", "squad")]), Some("me"));
    state.replace(fetch(vec![lead("a", "squad")]).read(|_| Ok(json!({"items":[]})), 100));
    assert!(state.observations_complete());
    state.reconcile(&observation_home(vec![]), Some("me"));
    assert!(state.observations_complete());
    state.reconcile(&observation_home(vec![lead("a", "squad")]), Some("me"));
    assert!(!state.observations_complete());
}

#[test]
fn conversation_results_use_one_originator_page_without_room_or_lead_scans() {
    let mut calls = 0;
    let read = Fetch::results(Some(Me {
        id: "me".into(),
        name: "user".into(),
    }))
    .read(
        |input| {
            calls += 1;
            assert_eq!(input["originatorId"], "me");
            assert_eq!(input["limit"], 50);
            Ok(if input["view"] == "answers" {
                json!({"items":[],"nextBefore":null})
            } else {
                assert_eq!(input["view"], "results");
                json!({"items":[reply("cross-room", "a", 20, Some("latest"))],"nextBefore":null})
            })
        },
        30,
    );
    assert_eq!(
        calls, 2,
        "the results page, and the answers page for unread"
    );
    assert!(read.leads.is_empty());
    let mut state = State::default();
    state.set_sender(Some("me"));
    state.replace(read);
    assert_eq!(state.replies[0]["requestId"], "cross-room");
    state.set_sender(Some("other"));
    assert!(state.replies.is_empty());
    let read = Fetch::results(None).read(|_| panic!("no sender must not read history"), 30);
    assert_eq!(read.calls, 0);
}

#[test]
fn questions_precede_newer_exchanges_and_the_longest_wait_comes_first() {
    let mut oldest = lead("z", "oldest-question");
    oldest.row["waitingOnYou"] =
        json!([{"requestId":"oldest","preparedAtMs":10,"preview":"oldest ask"}]);
    let mut newer = lead("a", "newer-question");
    newer.row["waitingOnYou"] =
        json!([{"requestId":"newer","preparedAtMs":30,"preview":"newer ask"}]);
    let mut pending = lead("p", "undated-question");
    pending.row["pending"] = json!("undated ask");
    // The HOME mark follows the selected exchange, not every decision in the
    // source row: a newer reply can be selected over an older incoming ask.
    let mut replied = lead("r", "reply");
    replied.row["waitingOnYou"] =
        json!([{"requestId":"older","preparedAtMs":20,"preview":"older ask"}]);
    let read = fetch(vec![
        lead("b", "asked"),
        newer,
        replied,
        pending,
        oldest,
        lead("e", "empty"),
    ])
    .read(
        |input| {
            Ok(if input["view"] == "results" {
                json!({"items":[reply("reply", "r", 80, Some("newer reply"))]})
            } else {
                json!({"items":[ask("asked", "b", 90)]})
            })
        },
        100,
    );
    assert_eq!(
        occurrences(&read.leads),
        [
            ("z", "oldest-question"),
            ("a", "newer-question"),
            ("p", "undated-question"),
            ("b", "asked"),
            ("r", "reply"),
            ("e", "empty")
        ]
    );
    assert_eq!(read.leads[4].exchange.as_ref().unwrap().kind, Kind::Reply);
    assert!(crate::attention::waits_on_you(&read.leads[4].row));
}

#[test]
fn exchanges_sort_before_empty_leads_through_read_reconcile_and_replace() {
    let mut question = lead("z", "dated-question");
    question.row["waitingOnYou"] =
        json!([{"requestId":"question","preparedAtMs":50,"preview":"question"}]);
    let mut pending = lead("d", "pending");
    pending.row["pending"] = json!("decision without a request timestamp");
    let input = vec![
        lead("a", "empty"),
        pending,
        lead("e", "future"),
        lead("c", "missing"),
        lead("x", "dated-ask"),
        lead("y", "dated-reply"),
        question,
        lead("b", "empty"),
    ];
    let read = fetch(input).read(|input| {
        let mut missing = ask("missing", "c", 1);
        missing["preparedAtMs"] = Value::Null;
        Ok(if input["view"] == "results" {
            json!({"items":[reply("reply", "y", 40, Some("answer")), reply("future", "e", 120, Some("future"))]})
        } else {
            json!({"items":[ask("ask", "x", 30), missing]})
        })
    }, 100);
    let expected = [
        ("z", "dated-question"),
        ("d", "pending"),
        ("y", "dated-reply"),
        ("x", "dated-ask"),
        ("c", "missing"),
        ("e", "future"),
        ("a", "empty"),
        ("b", "empty"),
    ];
    assert_eq!(occurrences(&read.leads), expected);
    assert!([1, 4, 5].into_iter().all(|index| {
        read.leads[index]
            .exchange
            .as_ref()
            .unwrap()
            .since_ms
            .is_none()
    }));

    let mut retained = read.leads.clone();
    retained.reverse();
    let home = super::super::home::Home {
        workspace: Default::default(),
        window: crate::config::TokenWindow::MINUTE,
        windows: crate::config::TokenWindow::DEFAULTS,
        reduced_motion: false,
        summary: Default::default(),
        squads: retained
            .iter()
            .map(|lead| super::super::home::SquadLine {
                squad: lead.squad.clone(),
                lead: Some(lead.row.clone()),
                counts: Default::default(),
            })
            .collect(),
        failures: vec![],
        incomplete: false,
    };
    let mut state = State {
        sender: Some("me".into()),
        leads: retained,
        ..Default::default()
    };
    state.reconcile(&home, Some("me"));
    assert_eq!(occurrences(&state.leads), expected);
    let mut shuffled = read;
    shuffled.leads.reverse();
    state.replace(shuffled);
    assert_eq!(occurrences(&state.leads), expected);
}

#[test]
fn equal_exchange_times_use_identity_ties_and_empty_leads_use_names() {
    let mut rows = vec![
        lead("b", "same"),
        lead("a", "z-squad"),
        lead("a", "a-squad"),
        lead("c", "empty"),
        lead("d", "empty"),
    ];
    rows[0].row["name"] = json!("aaa");
    rows[1].row["name"] = json!("zzz");
    rows[2].row["name"] = json!("zzz");
    rows[3].row["name"] = json!("zzz");
    rows[4].row["name"] = json!("aaa");
    let read = fetch(rows).read(|input| Ok(if input["view"] == "results" {
        json!({"items":[reply("reply-a", "a", 40, Some("answer")), reply("reply-b", "b", 40, Some("answer"))]})
    } else { json!({"items":[]}) }), 100);
    assert_eq!(
        occurrences(&read.leads),
        [
            ("a", "a-squad"),
            ("a", "z-squad"),
            ("b", "same"),
            ("d", "empty"),
            ("c", "empty")
        ]
    );
}

#[test]
fn late_acknowledged_reply_and_newer_question_use_event_times_not_request_order() {
    let mut one = lead("a", "squad");
    one.row["waitingOnYou"] =
        json!([{"requestId":"question","preparedAtMs":30,"preview":"asks now"}]);
    let final_item = reply("old-request", "a", 40, Some("late final"));
    let selected = latest(
        &one,
        "me",
        std::slice::from_ref(&final_item),
        &[ask("pending", "a", 20)],
        100,
    )
    .unwrap();
    assert_eq!(selected.kind, Kind::Reply);
    assert_eq!(selected.since_ms, Some(40));
    one.row["waitingOnYou"][0]["preparedAtMs"] = json!(50);
    let selected = latest(&one, "me", &[final_item], &[], 100).unwrap();
    assert_eq!(selected.kind, Kind::Question);
    assert_eq!(selected.request.as_deref(), Some("question"));
}

#[test]
fn acknowledged_unanswered_asks_remain_visible_and_unrelated_senders_do_not() {
    let one = lead("a", "squad");
    let mut unrelated = ask("unrelated", "a", 90);
    unrelated["sender"]["identityId"] = json!("someone-else");
    let selected = latest(
        &one,
        "me",
        &[],
        &[ask("mine", "a", 20), unrelated, ask("other-lead", "b", 99)],
        100,
    )
    .unwrap();
    assert_eq!(selected.kind, Kind::Asked);
    assert_eq!(selected.request.as_deref(), Some("mine"));
}

#[test]
fn withdrawn_history_never_replaces_an_open_ask_or_recipient_final() {
    let one = lead("a", "squad");
    let mut withdrawn = ask("withdrawn", "a", 90);
    withdrawn["final"] = json!({"status":"withdrawn", "reason":"obsolete", "withdrawnAtMs":99});
    let mut announcement = ask("announcement", "a", 95);
    announcement["kind"] = json!("announcement");
    announcement["final"] = json!({"status":"not_required"});
    let history = vec![withdrawn.clone(), announcement, ask("open", "a", 20)];
    let original = history.clone();
    let selected = latest(&one, "me", &[], &history, 100).unwrap();
    assert_eq!(selected.kind, Kind::Asked);
    assert_eq!(selected.request.as_deref(), Some("open"));
    assert!(latest(&one, "me", &[], &[withdrawn.clone()], 100).is_none());
    for (status, preview) in [
        ("retained", "recipient answer"),
        ("expired", "(reply expired)"),
        ("unavailable", "(reply preview unavailable)"),
    ] {
        let mut final_item = reply("final", "a", 40, None);
        final_item["final"]["status"] = json!(status);
        if status == "retained" {
            final_item["responsePreview"] = json!(preview);
        }
        // HOME joins the results view and ordinary recipient history.
        let selected = latest(&one, "me", &[final_item.clone()], &history, 100).unwrap();
        assert_eq!(selected.kind, Kind::Reply);
        assert_eq!(selected.request.as_deref(), Some("final"));
        assert_eq!(selected.since_ms, Some(40));
        assert_eq!(selected.status, status);
        assert_eq!(selected.preview, preview);
        let selected = latest(&one, "me", &[], &[withdrawn.clone(), final_item], 100).unwrap();
        assert_eq!(selected.kind, Kind::Reply);
        assert_eq!(selected.request.as_deref(), Some("final"));
    }
    let mut manual = one;
    manual.row["pending"] = json!("independent decision");
    let selected = latest(&manual, "me", &[], &[withdrawn], 100).unwrap();
    assert_eq!(selected.kind, Kind::Question);
    assert_eq!(selected.request, None);
    assert_eq!(selected.preview, "independent decision");
    assert_eq!(
        history, original,
        "HOME projections do not change Core history"
    );
}

#[test]
fn one_global_results_page_and_one_recipient_page_per_distinct_lead_are_bounded() {
    let mut inputs = Vec::new();
    let read = fetch(vec![lead("a", "one"), lead("a", "two"), lead("b", "three")]).read(|input| {
        inputs.push(input.clone());
        Ok(match input["view"].as_str() {
            Some("results") => json!({"items":[reply("late", "b", 40, Some("answer"))],"nextBefore":{"submittedAtMs":40,"requestId":"late"}}),
            Some("answers") => json!({"items":[],"nextBefore":null}),
            _ => json!({"items":[ask("pending", "a", 50)],"nextBefore":null}),
        })
    }, 100);
    assert_eq!(read.calls, 4);
    assert_eq!(inputs.len(), 4);
    assert!(inputs.iter().all(|input| input["limit"] == LIMIT
        && input.get("before").is_none()
        && input.get("roomId").is_none()));
    assert_eq!(inputs[0]["originatorId"], "me");
    assert_eq!(inputs[1]["view"], "answers");
    assert_eq!(inputs[2]["recipientId"], "a");
    assert_eq!(inputs[3]["recipientId"], "b");
    assert_eq!(
        read.leads
            .iter()
            .map(|lead| lead.squad.as_str())
            .collect::<Vec<_>>(),
        ["one", "two", "three"]
    );
    assert!(read.leads[2].exchange.as_ref().unwrap().preview == "answer");
}

#[test]
fn missing_user_reads_nothing_but_home_members_without_leads_reuse_one_results_page() {
    let read = Fetch {
        sender: None,
        leads: vec![lead("a", "squad")],
    }
    .read(|_| panic!("no core call without a sender"), 100);
    assert_eq!(read.calls, 0);
    let read = fetch(vec![]).read(
        |input| {
            assert_eq!(input["originatorId"], "me");
            Ok(if input["view"] == "answers" {
                json!({"items":[],"nextBefore":null})
            } else {
                assert_eq!(input["view"], "results");
                json!({"items":[reply("member-reply","member",20,Some("answer"))]})
            })
        },
        100,
    );
    assert_eq!(read.calls, 2);
    assert!(read.leads.is_empty());
    assert_eq!(read.replies[0]["recipientId"], "member");
    assert_eq!(read.replies[0]["requestId"], "member-reply");
    assert!(
        read.replies[0].get("response").is_none(),
        "bodies stay on the selected-read worker lane"
    );
}

#[test]
fn results_preview_wins_over_duplicate_metadata_and_display_is_inert() {
    let one = lead("a", "squad");
    let result = reply(
        "id",
        "a",
        20,
        Some("\x1b[31manswer\x1b[0m\u{202e}\nignored"),
    );
    let selected = latest(&one, "me", &[result], &[reply("id", "a", 20, None)], 100).unwrap();
    assert_eq!(selected.preview, "answer");
    assert_eq!(
        selected.kind,
        Kind::Reply,
        "reply prose never becomes a blocked/question mark"
    );
    let selected = latest(&one, "me", &[reply("id", "a", 20, Some(""))], &[], 100).unwrap();
    assert!(
        selected.preview.is_empty(),
        "an exact empty reply is not unavailable"
    );
}

#[test]
fn expired_finals_keep_submission_evidence_and_future_times_have_no_age() {
    let one = lead("a", "squad");
    let mut item = reply("id", "a", 120, None);
    item["final"]["status"] = json!("expired");
    let selected = latest(&one, "me", &[item], &[], 100).unwrap();
    assert_eq!(selected.kind, Kind::Reply);
    assert_eq!(selected.since_ms, None);
    assert_eq!(selected.preview, "(reply expired)");
    assert_eq!(selected.status, "expired");
}

#[test]
fn failed_reads_are_not_empty_evidence_and_changed_sender_drops_observations() {
    let prior = LeadPreview {
        kind: Kind::Reply,
        request: Some("id".into()),
        since_ms: Some(20),
        preview: "answer".into(),
        status: "retained".into(),
    };
    let mut state = State {
        sender: Some("me".into()),
        leads: vec![Lead {
            exchange: Some(prior.clone()),
            ..lead("a", "squad")
        }],
        ..Default::default()
    };
    let read = fetch(vec![lead("a", "squad")])
        .read(|_| Err(SquadError::new("READ_FAILED", "cannot read")), 100);
    assert_eq!(read.calls, 3);
    state.replace(read);
    assert!(state.failure.is_some() && state.leads[0].failure.is_some());
    assert_eq!(state.leads[0].exchange, Some(prior));
    state.replace(Read {
        sender: Some("other".into()),
        leads: vec![lead("a", "squad")],
        replies: vec![],
        unread: None,
        failure: None,
        calls: 0,
        elapsed_ms: 0,
    });
    assert!(
        state.leads[0].exchange.is_some(),
        "late read from another user is ignored"
    );
    let home = super::super::home::Home {
        workspace: Default::default(),
        window: crate::config::TokenWindow::MINUTE,
        windows: crate::config::TokenWindow::DEFAULTS,
        reduced_motion: false,
        summary: Default::default(),
        squads: vec![super::super::home::SquadLine {
            squad: "squad".into(),
            lead: Some(lead("a", "squad").row),
            counts: Default::default(),
        }],
        failures: vec![],
        incomplete: false,
    };
    state.reconcile(&home, Some("other"));
    assert!(state.leads[0].exchange.is_none());
    assert!(state.failure.is_none());
}

#[test]
fn malformed_pages_report_failure_instead_of_claiming_no_exchange() {
    let read = fetch(vec![lead("a", "squad")]).read(|_| Ok(json!({"nextBefore":null})), 100);
    assert!(read.failure.is_some());
    assert!(read.leads[0].failure.is_some());
    assert!(read.leads[0].exchange.is_none());
}

#[test]
fn partial_failure_does_not_replace_a_newer_reply_with_an_older_ask() {
    let prior = latest(
        &lead("a", "squad"),
        "me",
        &[reply("new", "a", 40, Some("answer"))],
        &[],
        100,
    );
    let mut state = State {
        sender: Some("me".into()),
        leads: vec![Lead {
            exchange: prior.clone(),
            ..lead("a", "squad")
        }],
        ..Default::default()
    };
    let read = fetch(vec![lead("a", "squad")]).read(
        |input| {
            if input["view"] == "results" {
                Err(SquadError::new("READ_FAILED", "results unavailable"))
            } else {
                Ok(json!({"items":[ask("old", "a", 20)],"nextBefore":null}))
            }
        },
        100,
    );
    state.replace(read);
    assert!(state.failure.is_some());
    assert_eq!(state.leads[0].exchange, prior);
}

fn message(kind: Kind) -> MessageKey {
    MessageKey {
        sender: "me".into(),
        lead: "a".into(),
        squad: "squad".into(),
        request: "id".into(),
        kind,
    }
}

#[test]
fn full_messages_use_the_chosen_body_and_verify_participants_without_acknowledging() {
    let mut detail = reply("id", "a", 20, Some("preview"));
    detail["final"]["response"] = json!("first\nsecond\n\x1b[31mthird\x1b[0m");
    detail["prompt"] = json!({"status":"retained","message":"original ask"});
    assert_eq!(
        message(Kind::Reply).body(&detail).unwrap(),
        "first\nsecond\n\x1b[31mthird\x1b[0m"
    );
    assert_eq!(message(Kind::Asked).body(&detail).unwrap(), "original ask");
    detail["final"]["response"] = json!("");
    assert_eq!(message(Kind::Reply).body(&detail).unwrap(), "");
    detail["sender"]["identityId"] = json!("a");
    detail["recipientId"] = json!("me");
    assert_eq!(
        message(Kind::Question).body(&detail).unwrap(),
        "original ask"
    );
    assert!(message(Kind::Reply).body(&detail).is_err());
}

#[test]
fn full_message_expiry_and_missing_body_are_honest() {
    let mut detail = reply("id", "a", 20, None);
    detail["final"]["status"] = json!("expired");
    assert!(
        message(Kind::Reply)
            .body(&detail)
            .unwrap_err()
            .to_string()
            .contains("expired")
    );
    detail["final"]["status"] = json!("retained");
    assert!(message(Kind::Reply).body(&detail).is_err());
}

// A test oracle over the retained read-state machine; no production Recent view remains.
impl State {
    pub fn observations_complete(&self) -> bool {
        let retained: BTreeSet<_> = self
            .leads
            .iter()
            .map(|lead| (lead.squad.as_str(), lead.id()))
            .collect();
        self.sender.is_some()
            && self.results_observed
            && self.failure.is_none()
            && self.lead_observed.iter().all(|((squad, id), observed)| {
                *observed && retained.contains(&(squad.as_str(), id.as_str()))
            })
            && self.leads.iter().all(|lead| {
                lead.failure.is_none()
                    && self
                        .lead_observed
                        .get(&(lead.squad.clone(), lead.id().to_owned()))
                        == Some(&true)
            })
    }
}

#[test]
fn unread_uses_only_the_originators_answers_page_and_holds_it_across_failed_or_late_reads() {
    let mut state = State::default();
    state.reconcile(&observation_home(vec![lead("a", "room")]), Some("me"));
    let read = fetch(vec![lead("a", "room")]).read(
        |input| match input["view"].as_str() {
            Some("results") => {
                Ok(json!({"items":[reply("new", "a", 100, Some("answer"))], "nextBefore":null}))
            }
            Some("answers") => {
                assert_eq!(
                    input,
                    json!({"originatorId":"me","view":"answers","limit":50})
                );
                Ok(json!({"items":[
                    {"requestId":"turn","kind":"request","sender":{"identityId":"me"},
                     "recipientId":"a","answeredAtMs":150,"answer":"turn"},
                    {"requestId":"new","kind":"request","sender":{"identityId":"me"},
                     "recipientId":"a","answeredAtMs":100,"answer":"final"}
                ], "nextBefore":null}))
            }
            _ => Ok(json!({"items":[reply("older", "a", 90, Some("older"))], "nextBefore":null})),
        },
        200,
    );
    assert_eq!(
        read.calls, 3,
        "results, one answers page for unread, the lead's history"
    );
    state.replace(read);
    assert_eq!(
        state.replies.len(),
        2,
        "conversation metadata still includes history"
    );
    assert_eq!(
        state.unread("me").unwrap().count("a", 0).total,
        2,
        "a captured turn counts beside the final"
    );
    let held = state.unread("me").unwrap().clone();
    state.replace(fetch(vec![]).read(|_| Err(SquadError::new("FAILED", "missing")), 200));
    assert_eq!(state.unread("me"), Some(&held));
    state.replace(fetch(vec![]).read(|_| Ok(json!({"items":[], "nextBefore":{}})), 200));
    assert_eq!(
        state.unread("me"),
        Some(&held),
        "malformed optional page cannot erase held evidence"
    );
    let late = fetch(vec![]).read(|_| Ok(json!({"items":[], "nextBefore":null})), 200);
    state.set_sender(Some("other"));
    state.replace(late);
    assert!(state.unread("me").is_none());
    assert!(
        state.unread("other").is_none(),
        "late actor result cannot mark another user's page"
    );
    let mut replacement = fetch(vec![]);
    replacement.sender.as_mut().unwrap().id = "other".into();
    state.replace(replacement.read(|_| Ok(json!({"items":[], "nextBefore":null})), 200));
    assert_eq!(state.unread("other").unwrap().count("a", 0).total, 0);
}

/// What Core's `answers` view says of a `results` page: each submitted final is an answer at
/// its submission time.
fn answers_of(results: &Value) -> Value {
    let items: Vec<_> = results["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["final"]["submittedAtMs"].is_u64())
        .map(|item| {
            json!({
                "requestId":item["requestId"], "kind":item["kind"], "sender":item["sender"],
                "recipientId":item["recipientId"], "answeredAtMs":item["final"]["submittedAtMs"],
                "answer":"final"
            })
        })
        .collect();
    let next = &results["nextBefore"];
    json!({"items":items,"nextBefore":if next.is_null() {
        Value::Null
    } else {
        json!({"answeredAtMs":next["submittedAtMs"],"requestId":next["requestId"]})
    }})
}

/// Native scenarios admit the same bounded originator documents as the worker.
impl State {
    /// A `results` page, with the `answers` page its finals make.
    pub(in crate::board) fn test_results(&mut self, actor: &str, document: Value) {
        let answers = answers_of(&document);
        self.test_pages(actor, document, answers);
    }

    /// A `results` page and an explicit `answers` page, as when a turn answered without a final.
    pub(in crate::board) fn test_pages(&mut self, actor: &str, results: Value, answers: Value) {
        self.set_sender(Some(actor));
        let read = Fetch::results(Some(Me {
            id: actor.into(),
            name: "Owner".into(),
        }))
        .read(
            |input| {
                let view = input["view"].as_str().unwrap_or_default().to_owned();
                assert_eq!(input, json!({"originatorId":actor,"view":view,"limit":50}));
                match view.as_str() {
                    "results" => Ok(results.clone()),
                    "answers" => Ok(answers.clone()),
                    other => panic!("unexpected view {other}"),
                }
            },
            crate::status::now_ms(),
        );
        assert_eq!(read.calls, 2);
        self.replace(read);
    }
}
