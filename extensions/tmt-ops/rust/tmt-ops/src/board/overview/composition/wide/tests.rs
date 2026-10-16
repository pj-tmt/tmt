use super::*;
use crate::board::{
    app::{
        Compose, Effect, Request,
        workspace::tests::{board, observe_waiting_member},
    },
    overview::capture,
    shell::OpsTab,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
};
fn board_app() -> App {
    let mut app = board();
    observe_waiting_member(&mut app);
    app.shell.tab = OpsTab::Overview;
    capture::populate_lower_band(&mut app, "squads");
    capture::populate_final(&mut app, "squads");
    app
}
fn paint(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
    terminal.backend().buffer().clone()
}
fn text(buffer: &ratatui::buffer::Buffer) -> String {
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
fn key(app: &mut App, code: KeyCode) -> Effect {
    app.key(KeyEvent::new(code, KeyModifiers::NONE))
}
#[test]
fn final_wide_composition_has_joined_divider_direct_matrix_and_filled_reply_field() {
    let _commands = crate::runner::forbid_commands();
    let mut app = board_app();
    let buffer = paint(&mut app, 148, 51);
    let shown = text(&buffer);
    assert!(shown.contains("% of input") && shown.contains("≈ $ avoided"));
    assert!(!shown.contains("Auto · timed · Off") && !shown.contains("Conversation ·"));
    let band = app.input_band.get().unwrap();
    assert_eq!(band.height, 3);
    assert_eq!(band.x, 90);
    assert_eq!(band.width, 57);
    assert!(shown.contains("⏎ to reply to worker"));
    assert!(!shown.contains("[ Send ]"));
    let rule = app.conversation.borrow().pane.unwrap().y;
    assert_eq!(buffer[(88, rule)].symbol(), "┬");
    assert_eq!(buffer[(88, rule + 1)].symbol(), "│");
    assert_eq!(
        buffer[(band.x, band.y)].bg,
        crate::board::input_field::style(app.look(), false)
            .bg
            .unwrap()
    );
    for y in band.y..band.bottom() {
        assert_eq!(buffer[(89, y)].bg, ratatui::style::Color::Reset);
        assert_eq!(buffer[(147, y)].bg, ratatui::style::Color::Reset);
        for x in band.x..band.right() {
            assert_eq!(buffer[(x, y)].bg, buffer[(band.x, band.y)].bg);
        }
    }
    assert_eq!(buffer[(92, band.y + 1)].symbol(), "⏎");
    assert!(shown.contains("worker → Owner"));
}

#[test]
fn input_follows_natural_exchange_growth_and_pins_only_when_it_overflows() {
    let _commands = crate::runner::forbid_commands();
    let mut app = board_app();
    paint(&mut app, 148, 51);
    let read = app.detail_read().expect("existing exact-question reader");
    app.apply_message(&read, Ok("One observed line.".into()));
    paint(&mut app, 148, 51);
    let pane = app.conversation.borrow().pane.unwrap();
    let short = conversation::exchange::desired_height(&app, pane.width);
    assert!(
        short < 6,
        "a short observation must not reserve six rows: {short}"
    );
    assert_eq!(app.input_band.get().unwrap().y, pane.y + 1 + short + 1);

    let body = (0..14)
        .map(|n| format!("Observed line {n}."))
        .collect::<Vec<_>>()
        .join("\n\n");
    app.apply_message(&read, Ok(body));
    paint(&mut app, 148, 51);
    let grown = conversation::exchange::desired_height(&app, pane.width);
    assert!(
        grown > 12,
        "the transcript must grow beyond the retired cap"
    );
    assert_eq!(app.input_band.get().unwrap().y, pane.y + 1 + grown + 1);

    let body = (0..100)
        .map(|n| format!("Observed line {n}."))
        .collect::<Vec<_>>()
        .join("\n\n");
    app.apply_message(&read, Ok(body));
    let shown = text(&paint(&mut app, 148, 51));
    assert_eq!(app.input_band.get().unwrap().bottom(), pane.bottom());
    assert!(shown.contains(" earlier · PgUp"), "{shown}");
    assert!(!shown.contains("↑ 0 earlier"));
    assert!(!shown.contains("── Input") && !shown.contains("[ Send ]"));
    crate::board::app::tests::configure_workspace(
        &mut app,
        "[bind]\npageup='menu'\nu='scroll-page-up'\n",
    );
    let authored = text(&paint(&mut app, 148, 51));
    assert!(authored.contains(" earlier · u"));
    assert!(!authored.contains(" earlier · PgUp"));
    key(&mut app, KeyCode::Enter);
    let typing = text(&paint(&mut app, 148, 51));
    assert!(
        !typing.contains(" earlier · u"),
        "the editor owns typing keys"
    );
}

#[test]
fn a_squad_uses_its_observed_lead_and_never_borrows_a_waiting_members_question() {
    use crate::board::workspace::OpsTarget;
    let mut app = board_app();
    paint(&mut app, 148, 51);
    let member_request = app.detail_read().unwrap().request;
    let lead_id = "77777777-7777-4777-8777-777777777777";
    let snapshot = &mut app.view.as_mut().unwrap().home.as_mut().unwrap().workspace;
    let mut lead = snapshot.roster.members[crate::board::app::workspace::tests::MEMBER].clone();
    lead.id = lead_id.into();
    lead.name = "observed-lead".into();
    lead.waiting = false;
    snapshot.roster.members.insert(lead_id.into(), lead);
    snapshot.roster.squads[0].lead = Some(lead_id.into());
    snapshot.roster.squads[0].members.push(lead_id.into());
    snapshot.roster.squads[0].member_states.insert(
        lead_id.into(),
        crate::board::workspace::OpsMembershipState {
            state: Some("working".into()),
            waiting: false,
            rank: None,
        },
    );
    snapshot.rows.insert(OpsTarget::Member { room_id: Some(crate::board::app::workspace::tests::ROOM.into()), identity_id: lead_id.into() }, serde_json::json!({"id":lead_id,"name":"observed-lead","state":"working","presence":"active","pane":{"location":{"host":"tmux"}}}));
    app.overview_rows.borrow_mut().clear();
    let squad = app.workspace_snapshot().unwrap().roster.squads[0].clone();
    let lead = squad.lead.clone().unwrap();
    app.overview_view = crate::board::overview::monitor::View::Squads;
    app.workspace.selected = Some(OpsTarget::Squad(squad.room_id));
    app.workspace_reconcile();
    paint(&mut app, 148, 51);
    let target = app.conversation_target().unwrap();
    assert_eq!(
        app.target_row(&target).unwrap()["id"].as_str(),
        Some(lead.as_str())
    );
    assert_ne!(lead, crate::board::app::workspace::tests::MEMBER);
    assert!(
        app.detail_read()
            .is_none_or(|read| read.request != member_request)
    );
    let selected = app.workspace.selected.clone();
    let target = app.workspace_message_target().unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .rows
        .get_mut(&target)
        .unwrap()["waitingOnYou"] = serde_json::json!([{
        "requestId":"req_88888888-8888-4888-8888-888888888888",
        "preparedAtMs":crate::status::now_ms(),
        "preview":"The observed lead asks this exact question."
    }]);
    paint(&mut app, 148, 51);
    assert!(
        app.input_band.get().is_some(),
        "the passive field must admit the exact lead reply"
    );
    key(&mut app, KeyCode::Enter);
    assert!(matches!(app.input.as_ref().unwrap().compose,
        Compose::Reply { ref request, .. } if request == "req_88888888-8888-4888-8888-888888888888"));
    app.paste("Reply to the lead only.");
    assert!(matches!(key(&mut app, KeyCode::Enter),
        Effect::Act(Request::Reply { request, text, .. })
        if request == "req_88888888-8888-4888-8888-888888888888" && text == "Reply to the lead only."));
    assert_eq!(app.workspace.selected, selected);
    key(&mut app, KeyCode::Esc);
    crate::board::app::tests::configure_workspace(
        &mut app,
        "[bind.tmux]\nenter='menu'\nt='talk'\n",
    );
    assert_eq!(
        app.workspace_bindings()["enter"].verb,
        crate::action::Verb::Menu
    );
    let shown = text(&paint(&mut app, 148, 51));
    assert!(shown.contains("t to message observed-lead"), "{shown}");
    key(&mut app, KeyCode::Enter);
    assert!(app.menu.is_some() && app.input.is_none());
}
#[test]
fn enter_and_pointer_reply_to_the_exact_question_and_an_unrelated_result_never_replaces_it() {
    let _commands = crate::runner::forbid_commands();
    let mut app = board_app();
    app.view.as_mut().unwrap().replies.push(serde_json::json!({"recipientId":crate::board::app::workspace::tests::MEMBER,"requestId":"unrelated","response":"UNRELATED_RESPONSE"}));
    paint(&mut app, 148, 51);
    let read = app
        .detail_read()
        .expect("existing selected-message read lane");
    assert_eq!(read.kind, crate::board::home_leads::Kind::Question);
    let mut wrong = read.clone();
    wrong.request = "unrelated".into();
    app.apply_message(&wrong, Ok("WRONG_BODY".into()));
    app.apply_message(&read, Ok("Exact acquired question body".into()));
    let shown = text(&paint(&mut app, 148, 51));
    assert!(shown.contains("Exact acquired question body"));
    assert!(!shown.contains("WRONG_BODY") && !shown.contains("UNRELATED_RESPONSE"));
    assert!(
        !app.conversation_reply().unwrap().source,
        "question bytes are not response attachments"
    );
    key(&mut app, KeyCode::Tab);
    assert_eq!(
        app.workspace_scope(),
        crate::config::OpsKeyScope::OverviewReader
    );
    assert_eq!(key(&mut app, KeyCode::Enter), Effect::None);
    assert!(
        matches!(&app.input.as_ref().unwrap().compose,Compose::Reply {request,..} if request.as_str()==read.request.as_str())
    );
    let shown = text(&paint(&mut app, 148, 51));
    assert!(shown.contains("⏎ send · Esc cancel"));
    let band = app.input_band.get().unwrap();
    let hint = crate::board::input_field::send_hint(band).unwrap();
    assert_eq!(hint.right(), band.right() - 1);
    assert_eq!(
        app.composer.send_hit.borrow().as_ref().unwrap().area.x,
        hint.x
    );
    assert_eq!(app.input_viewport.get().unwrap().0.x, band.x + 2);
    app.paste("Approved");
    assert!(
        matches!(key(&mut app,KeyCode::Enter),Effect::Act(Request::Reply {request,text,..}) if request==read.request && text=="Approved")
    );
    // A separate fresh passive field has the same pointer-to-request route.
    let mut pointer = board_app();
    paint(&mut pointer, 148, 51);
    let band = pointer.input_band.get().unwrap();
    pointer.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: band.x + 1,
            row: band.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    assert!(
        matches!(&pointer.input.as_ref().unwrap().compose,Compose::Reply {request,..} if request.as_str()==read.request.as_str())
    );
}
#[test]
fn read_only_missing_identity_has_no_input_and_resize_preserves_active_uuid_draft() {
    let mut app = board_app();
    app.view.as_mut().unwrap().me_id = None;
    let shown = text(&paint(&mut app, 148, 51));
    assert!(shown.contains("read-only"));
    assert!(app.input_band.get().is_none());
    assert!(app.composer.send_hit.borrow().is_none());
    let mut app = board_app();
    paint(&mut app, 148, 51);
    key(&mut app, KeyCode::Enter);
    app.paste("retained exact draft");
    let identity = app.input.as_ref().unwrap().draft_key.clone();
    paint(&mut app, 148, 51);
    let old = app.composer.send_hit.borrow().clone().unwrap().area;
    paint(&mut app, 103, 51);
    assert_eq!(app.shell.tab, OpsTab::Agents);
    assert_eq!(app.input.as_ref().unwrap().draft_key, identity);
    assert_eq!(
        app.input.as_ref().unwrap().text.as_str(),
        "retained exact draft"
    );
    assert!(
        app.composer
            .send_hit
            .borrow()
            .as_ref()
            .is_none_or(|hit| hit.area != old)
    );
}

#[test]
fn authored_start_and_host_keys_keep_their_meaning_in_the_final_reader() {
    let _commands = crate::runner::forbid_commands();
    let mut app = board_app();
    crate::board::app::tests::configure_workspace(
        &mut app,
        "[bind]\nenter='menu'\nt='talk'\n[bind.tmux]\nleft='jump'\n",
    );
    let shown = text(&paint(&mut app, 148, 51));
    assert!(shown.contains("t to message worker"));
    assert!(!shown.contains("⏎ to reply"));
    key(&mut app, KeyCode::Enter);
    assert!(app.menu.is_some() && app.input.is_none());
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Tab);
    assert!(
        matches!(key(&mut app,KeyCode::Left),Effect::Act(Request::Jump(id)) if id==crate::board::app::workspace::tests::MEMBER)
    );
    key(&mut app, KeyCode::Char('t'));
    assert!(matches!(
        app.input.as_ref().unwrap().compose,
        Compose::Talk { .. }
    ));
    let shown = text(&paint(&mut app, 148, 51));
    assert!(
        !shown.contains("c timers")
            && !shown.contains("3 Digest for all")
            && !shown.contains("o open")
    );
    assert_eq!(app.input_band.get().unwrap().height, 3);
}
#[test]
fn final_input_keeps_quote_provenance_and_masks_geometry_under_a_blocking_overlay() {
    use crate::board::{
        home_leads::{Kind, MessageKey},
        response::Quote,
    };
    let mut app = board_app();
    paint(&mut app, 148, 51);
    key(&mut app, KeyCode::Enter);
    app.input.as_mut().unwrap().response_quotes.push(Quote {
        key: MessageKey {
            sender: app.view.as_ref().unwrap().me_id.clone().unwrap(),
            lead: crate::board::app::workspace::tests::MEMBER.into(),
            squad: "product".into(),
            request: "prior-response".into(),
            kind: Kind::Reply,
        },
        range: 0..6,
        text: "quoted".into(),
    });
    app.paste("retained text");
    let shown = text(&paint(&mut app, 148, 51));
    assert!(shown.contains("Quote") && shown.contains("quoted"));
    let quote = app.composer.quote_hit.borrow().clone().unwrap();
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: quote.area.x,
            row: quote.area.y,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    assert!(app.input.as_ref().unwrap().response_quotes.is_empty());
    assert_eq!(app.input.as_ref().unwrap().text.as_str(), "retained text");
    app.help = true;
    paint(&mut app, 148, 51);
    assert!(app.composer.send_hit.borrow().is_none() && app.composer.quote_hit.borrow().is_none());
    assert!(app.input_viewport.get().is_none());
}

#[test]
fn a_focused_mention_owns_default_enter_while_tab_still_moves_panes() {
    let mut app = board_app();
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
        .unwrap()["waitingOnYou"][0]["preview"] = serde_json::json!("Ask #tmt:worker, please.");
    app.references = crate::board::inventory::Inventory::from_listed(
        &[],
        &Ok(
            serde_json::json!({"identities":[{"id":crate::board::app::workspace::tests::MEMBER,"name":"worker","canonicalName":"worker"}]}),
        ),
    );
    paint(&mut app, 148, 51);
    key(&mut app, KeyCode::Tab);
    paint(&mut app, 148, 51);
    key(&mut app, KeyCode::Right);
    let shown = text(&paint(&mut app, 148, 51));
    assert!(app.member_link_owns_enter());
    assert!(!shown.contains("⏎ to reply to"));
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focused(), Pane::Rows);
    key(&mut app, KeyCode::BackTab);
    paint(&mut app, 148, 51);
    assert_eq!(app.focused(), Pane::Replies);
    assert!(
        !app.member_link_owns_enter(),
        "pane changes clear stale mention geometry"
    );
    key(&mut app, KeyCode::Right);
    assert!(app.member_link_owns_enter());
    key(&mut app, KeyCode::Enter);
    assert!(app.menu.is_some() && app.input.is_none());
}

#[test]
fn overview_notices_and_errors_keep_the_neutral_palette() {
    use tmt_cli_style::Role;
    let mut app = board_app();
    for error in [false, true] {
        app.notice = (!error).then(|| "Nothing sent.".into());
        app.error = error.then(|| "Reply failed; draft kept.".into());
        let buffer = paint(&mut app, 148, 51);
        assert_eq!(buffer[(0, 50)].fg, app.look().role(Role::Muted).fg.unwrap());
    }
}
