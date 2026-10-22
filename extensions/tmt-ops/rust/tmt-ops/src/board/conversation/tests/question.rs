use super::*;
use crate::board::app::workspace::tests::MEMBER;

/// The squads tree builds the thread from the finals to the user's own requests. An
/// agent that answers by sending the user new requests must show every one of them,
/// each at the time it was sent, or a later answer hides behind an earlier one.
#[test]
fn every_open_request_from_the_agent_is_in_the_squads_thread_by_send_time() {
    crate::status::with_now_ms(3_600_000, || {
        let mut app = workspace_fixture();
        assert!(!app.flat_agents(), "the Squads chip is the default");
        app.view.as_mut().unwrap().replies = vec![
            json!({
                "recipientId":MEMBER,"requestId":"older-request",
                "submittedAtMs":600_000,"response":"Older final"
            }),
            json!({
                "recipientId":MEMBER,"requestId":"middle-request",
                "submittedAtMs":3_520_000,"response":"Middle final"
            }),
        ];
        let target = app.workspace.selected.clone().unwrap();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .rows
            .get_mut(&target)
            .unwrap()["waitingOnYou"] = json!([
            {"requestId":"req_a0cc1d41-e7cf-4489-8d76-1884e01f271a",
             "preview":"Checked the two largest sessions","preparedAtMs":3_500_000},
            {"requestId":"req_b1dd2e52-f8d0-4590-9e95-2995f12f382b",
             "preview":"A second question after the first","preparedAtMs":3_550_000}
        ]);
        app.workspace_reconcile();
        draw(&app, 120, 40);
        let state = app.conversation.borrow();
        let session = state.sessions.get(state.active.as_ref().unwrap()).unwrap();
        let lines: Vec<_> = session
            .rendered
            .as_ref()
            .unwrap()
            .mapped
            .lines
            .iter()
            .map(Line::to_string)
            .collect();
        let at = |text: &str| {
            lines
                .iter()
                .position(|line| line.contains(text))
                .unwrap_or_else(|| panic!("missing {text}: {lines:#?}"))
        };
        let order = [
            at("Older final"),
            at("Checked the two largest sessions"),
            at("Middle final"),
            at("A second question after the first"),
        ];
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{lines:#?}");
    });
}
