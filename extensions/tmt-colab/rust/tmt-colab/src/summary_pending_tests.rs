use super::*;
use crate::export::conversations::{Anchor, Ask, Head, Reply};
use serde_json::{Value, json};

fn string(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or("").to_owned()
}
fn view(case: &Value) -> Conversations {
    let threads = case["threads"].as_array().unwrap().iter().map(|row| {
        let writer = string(row, "writer");
        let id = string(row, "id");
        let comments = row["comments"].as_array().unwrap().iter().map(|comment| Comment {
            writer: string(comment, "writer"), id: string(comment, "id"), revision: "1".into(),
            sequence: string(comment, "sequence"), deleted: comment["deleted"] == true,
            body: string(comment, "body"), device_name: "Fixture".into(), at: "1".into(), attachments: Vec::new(),
        }).collect();
        let anchor = row.get("anchor").filter(|anchor| !anchor.is_null()).map(|anchor| Anchor {
            exact: string(anchor, "exact"), prefix: string(anchor, "prefix"), suffix: string(anchor, "suffix"),
        });
        let status = row.get("status").map(|status| crate::threads::status::Status {
            action: serde_json::from_value(json!({
                "version":1,"kind":"thread-status","spaceId":"fixture","pageId":id,"epoch":"1",
                "senderDevice":writer,"revision":"1","deleted":false,"deviceName":"Fixture","at":"1",
                "actionId":id,"thread":{"writer":writer,"id":id},"previous":null,
                "resolved":status["resolved"],"actor":status["actor"],"agentName":null,"recipients":[]
            })).unwrap(),
            reference: crate::threads::status::Reference { writer: writer.clone(), id: id.clone() }, depth: 1,
        });
        let decision = row.get("decided").map(|value| serde_json::from_value(json!({
            "version":1,"kind":"proposal-decision","spaceId":"fixture","pageId":id,"epoch":"1",
            "senderDevice":writer,"revision":"1","deleted":value == false,"deviceName":"Fixture","at":"1",
            "actionId":id,"thread":{"writer":writer,"id":id},"previous":null,"decision":"approved"
        })).unwrap());
        Thread {
            writer, id, revision:"1".into(), anchor, resolved:row["resolved"] == true,
            deleted:row["deleted"] == true, device_name:"Fixture".into(), at:"1".into(), comments,
            proposal:row.get("proposal").map(|proposal| serde_json::from_value(proposal.clone()).unwrap()),
            decision, status, notifications:Vec::new(),
        }
    }).collect();
    let asks = case["asks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| Ask {
            writer: string(row, "writer"),
            operation_id: "fixture".into(),
            device_name: "Fixture".into(),
            agent_name: "Agent 1".into(),
            agent: "fixture".into(),
            machine: "fixture".into(),
            thread: string(row, "thread"),
            message_ids: row["messageIds"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap().into())
                .collect(),
            issued_at: 0,
            expires_at: 1,
            message: String::new(),
            state: "accepted".into(),
            reason: None,
            request_id: None,
            reply: (row["replied"] == true).then(|| Reply {
                request_id: "fixture".into(),
                agent_id: "fixture".into(),
                body: "Reply".into(),
            }),
        })
        .collect();
    Conversations {
        format: "fixture",
        version: 1,
        space_id: "fixture".into(),
        page_id: "fixture".into(),
        title: "Page".into(),
        epoch: "1".into(),
        membership_head: Head {
            revision: "1".into(),
            statement_hash: String::new(),
        },
        threads,
        asks,
    }
}

#[test]
fn native_counts_match_the_shared_browser_pending_vectors() {
    let vectors: Value =
        serde_json::from_str(include_str!("../../../contracts/vectors/needs-you-v1.json")).unwrap();
    for case in vectors["cases"].as_array().unwrap() {
        let view = view(case);
        let writers = case["deviceWriters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        let (report, anchors) =
            crate::page_check::summary(case["source"].as_str().unwrap(), &view.threads).unwrap();
        assert_eq!(
            serde_json::to_value(items(&report, &anchors, &view, &writers)).unwrap(),
            case["expected"],
            "{}",
            case["name"]
        );
    }
}

#[test]
fn a_quote_budget_failure_is_unavailable_not_a_false_empty_summary() {
    let vectors: Value =
        serde_json::from_str(include_str!("../../../contracts/vectors/needs-you-v1.json")).unwrap();
    let mut case = vectors["cases"][5].clone();
    let source = format!(
        "{}{}",
        "<p>Read plan</p>".repeat(65),
        case["source"].as_str().unwrap()
    );
    case["source"] = json!(source);
    let conversations = view(&case);
    assert!(crate::page_check::summary(&source, &conversations.threads).is_err());
}

#[test]
fn top_level_text_and_empty_sources_need_no_element_parent() {
    for source in [
        "",
        "hello",
        "before <div>inside</div> after",
        "plain 🐈 text",
    ] {
        let (report, anchors) = crate::page_check::summary(source, &[]).unwrap();
        assert!(report.questions.is_empty());
        assert!(!anchors.contains("missing"));
    }
}

#[test]
fn a_page_without_admitted_blocks_keeps_its_summary_above_the_quote_text_budget() {
    let vectors: Value =
        serde_json::from_str(include_str!("../../../contracts/vectors/needs-you-v1.json")).unwrap();
    let case = &vectors["cases"][0];
    let conversations = view(case);
    let source = "plain text ".repeat(30_000);
    let (report, anchors) = crate::page_check::summary(&source, &conversations.threads).unwrap();
    let pending = items(&report, &anchors, &conversations, &Default::default());
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].kind, "proposal");
    assert_eq!(pending[0].id, case["expected"][0]["id"].as_str().unwrap());
}

#[test]
fn complete_reading_descriptors_match_the_shared_browser_cases() {
    let inputs: Value =
        serde_json::from_str(include_str!("../../../contracts/vectors/needs-you-v1.json")).unwrap();
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../contracts/vectors/reading-details-v1.json"
    ))
    .unwrap();
    for vector in vectors["cases"].as_array().unwrap() {
        let case = inputs["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == vector["pendingCase"])
            .expect("named input exists");
        let conversations = view(case);
        let writers = case["deviceWriters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect();
        let (report, anchors) =
            crate::page_check::summary(case["source"].as_str().unwrap(), &conversations.threads)
                .unwrap();
        let pending = items(&report, &anchors, &conversations, &writers);
        let actual = crate::summary_readings::descriptors(&report, &anchors, &pending);
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            vector["expected"],
            "{}",
            vector["pendingCase"]
        );
    }
}
