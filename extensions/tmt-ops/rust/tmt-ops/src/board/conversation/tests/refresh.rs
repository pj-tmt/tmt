//! Loading/retained/single-publication scenes share the live display test owner.
use super::*;

pub(in crate::board) const SCENES: [&str; 3] = [
    "conversation-loading",
    "conversation-refreshing",
    "conversation-refreshed",
];

pub(in crate::board) fn capture_case(scene: &str, look: Look, width: u16, height: u16) -> App {
    assert!(SCENES.contains(&scene));
    let mut app = loading_workspace();
    app.view.as_mut().unwrap().look = look;
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .squad_looks
        .clear();
    if scene == "conversation-loading" {
        return app;
    }
    // Prime the actual completed projection at an admitted viewport, then lay it
    // out at the capture size before beginning another read. No fixture writes.
    workspace_frame(&app);
    let plan = app.conversation_read().unwrap();
    app.apply_conversation(plan.clone(), plan.test_projection(
        "The retained conversation is available.\nThis exact source stays visible during refresh.",
        Some("Retained provider output."),
    ));
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, &app))
        .unwrap();
    app.refresh_sent();
    app.conversation.borrow_mut().refresh_pane_output();
    if scene == "conversation-refreshed" {
        // The completed read includes a new final; it appends to, rather than
        // overwrites, the earlier held conversation.
        app.view.as_mut().unwrap().replies.push(json!({
            "recipientId":crate::board::app::workspace::tests::MEMBER,
            "requestId":"new-complete-final","submittedAtMs":crate::status::now_ms() - 1_000,
            "status":"retained"
        }));
        // Acquisition depends on an admitted viewport; the fixture can still
        // capture a clipped final frame after that complete read.
        workspace_frame(&app);
        let plan = app.conversation_read().unwrap();
        app.apply_conversation(plan.clone(), plan.test_projection(
            "The refreshed conversation arrived in one publication.\nAll of this source is available together.",
            Some("Fresh provider output."),
        ));
    }
    app
}
