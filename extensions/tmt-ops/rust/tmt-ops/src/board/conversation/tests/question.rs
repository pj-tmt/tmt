use super::*;
use crate::board::app::workspace::tests::MEMBER;

/// The squads tree builds the thread from the finals to the user's own requests. An
/// agent that answers by sending the user a new request must still show up there.
#[test]
fn an_open_request_from_the_agent_is_the_last_message_of_the_squads_thread() {
    crate::status::with_now_ms(3_600_000, || {
        let mut app = workspace_fixture();
        assert!(!app.flat_agents(), "the Squads chip is the default");
        app.view.as_mut().unwrap().replies = vec![json!({
            "recipientId":MEMBER,"requestId":"older-request",
            "submittedAtMs":600_000,"response":"Older final"
        })];
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
            .unwrap()["waitingOnYou"] = json!([{
            "requestId":"req_a0cc1d41-e7cf-4489-8d76-1884e01f271a",
            "preview":"Checked the two largest sessions",
            "preparedAtMs":3_500_000
        }]);
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
        assert!(
            at("Older final") < at("Checked the two largest sessions"),
            "{lines:#?}"
        );
    });
}
