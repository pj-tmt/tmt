use super::*;

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
        windows: crate::config::TokenWindow::DEFAULTS,
        summary: Default::default(),
        sections: vec![],
        squads: retained
            .iter()
            .map(|lead| super::super::home::SquadLine {
                squad: lead.squad.clone(),
                lead: Some(lead.row.clone()),
                counts: Default::default(),
                members: Default::default(),
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
        Ok(if input["view"] == "results" {
            json!({"items":[reply("late", "b", 40, Some("answer"))],"nextBefore":{"submittedAtMs":40,"requestId":"late"}})
        } else {
            json!({"items":[ask("pending", "a", 50)],"nextBefore":null})
        })
    }, 100);
    assert_eq!(read.calls, 3);
    assert_eq!(inputs.len(), 3);
    assert!(inputs.iter().all(|input| input["limit"] == LIMIT
        && input.get("before").is_none()
        && input.get("roomId").is_none()));
    assert_eq!(inputs[0]["originatorId"], "me");
    assert_eq!(inputs[1]["recipientId"], "a");
    assert_eq!(inputs[2]["recipientId"], "b");
    assert!(read.incomplete);
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
            assert_eq!(
                input,
                json!({"originatorId":"me","view":"results","limit":50})
            );
            Ok(json!({"items":[reply("member-reply","member",20,Some("answer"))]}))
        },
        100,
    );
    assert_eq!(read.calls, 1);
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
    assert_eq!(read.calls, 2);
    state.replace(read);
    assert!(state.failure.is_some() && state.leads[0].failure.is_some());
    assert_eq!(state.leads[0].exchange, Some(prior));
    state.replace(Read {
        sender: Some("other".into()),
        leads: vec![lead("a", "squad")],
        replies: vec![],
        failure: None,
        incomplete: false,
        calls: 0,
        elapsed_ms: 0,
    });
    assert!(
        state.leads[0].exchange.is_some(),
        "late read from another user is ignored"
    );
    let home = super::super::home::Home {
        windows: crate::config::TokenWindow::DEFAULTS,
        summary: Default::default(),
        sections: vec![],
        squads: vec![super::super::home::SquadLine {
            squad: "squad".into(),
            lead: Some(lead("a", "squad").row),
            counts: Default::default(),
            members: Default::default(),
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
        message(Kind::Reply).body(detail.clone()).unwrap(),
        "first\nsecond\nthird"
    );
    assert_eq!(
        message(Kind::Asked).body(detail.clone()).unwrap(),
        "original ask"
    );
    detail["final"]["response"] = json!("");
    assert_eq!(message(Kind::Reply).body(detail.clone()).unwrap(), "");
    detail["sender"]["identityId"] = json!("a");
    detail["recipientId"] = json!("me");
    assert_eq!(
        message(Kind::Question).body(detail.clone()).unwrap(),
        "original ask"
    );
    assert!(message(Kind::Reply).body(detail).is_err());
}

#[test]
fn full_message_expiry_and_missing_body_are_honest() {
    let mut detail = reply("id", "a", 20, None);
    detail["final"]["status"] = json!("expired");
    assert_eq!(
        message(Kind::Reply).body(detail.clone()).unwrap(),
        "(message expired)"
    );
    detail["final"]["status"] = json!("retained");
    assert!(message(Kind::Reply).body(detail).is_err());
}
