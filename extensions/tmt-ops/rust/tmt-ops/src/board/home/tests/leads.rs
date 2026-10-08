use super::interaction::{board, press};
use super::*;
use crate::board::{
    app::{App, Compose, Effect, Request},
    home_leads::{Kind, LeadPreview},
};
use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyCode::*};

fn fixture() -> App {
    let mut question = row("A", "lead-a", "working");
    question["waitingOnYou"] =
        json!([{"requestId":"question","preparedAtMs":20,"preview":"Approve this change?"}]);
    let mut app = board(&[
        ("a", document("a", question, vec![])),
        ("b", document("b", row("B", "lead-b", "working"), vec![])),
    ]);
    let view = app.view.as_mut().unwrap();
    view.me_id = Some("user".into());
    app.home_leads
        .reconcile(view.home.as_ref().unwrap(), view.me_id.as_deref());
    app.home_leads.leads[0].exchange = Some(LeadPreview {
        kind: Kind::Question,
        request: Some("question".into()),
        since_ms: Some(20),
        preview: "Approve this change?".into(),
        status: "retained".into(),
    });
    app.home_leads.leads[1].exchange = Some(LeadPreview {
        kind: Kind::Reply,
        request: Some("reply".into()),
        since_ms: Some(10),
        preview: "The work is ready.".into(),
        status: "retained".into(),
    });
    select(&mut app, "leads", "a");
    app
}
fn select(app: &mut App, section: &str, squad: &str) {
    let index = app
        .home_entries()
        .iter()
        .position(|entry| entry.target.section == section && entry.target.squad == squad)
        .unwrap();
    app.select(index);
}
fn draw(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| crate::board::view::render(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}
fn lines(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
    buffer
        .content
        .chunks(usize::from(buffer.area.width))
        .map(|cells| cells.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect()
}

#[test]
fn home_leads_collapse_by_default_and_expand_without_an_exchange() {
    for width in [80, 160] {
        let mut app = fixture();
        let initial = lines(&draw(&app, width, 40)).join("\n");
        assert!(
            !initial.contains("asks: Approve")
                && !initial.contains("The work is ready")
                && !initial.contains("hides replies")
        );
        app.home_leads.leads[0].exchange = None;
        press(&mut app, Char('e'));
        let expanded = lines(&draw(&app, width, 40)).join("\n");
        assert!(expanded.contains("no reply yet"));
        assert!(app.input.is_none());
        press(&mut app, Char('e'));
        assert!(
            !lines(&draw(&app, width, 40))
                .join("\n")
                .contains("no reply yet")
        );
    }
}

#[test]
fn home_lead_and_member_use_the_same_reply_renderer_and_v_reader() {
    use crate::board::row_detail::Target;
    let mut app = fixture();
    select(&mut app, "leads", "b");
    press(&mut app, Char('e'));
    let lead_target = app.row_target(app.selected).unwrap();
    let key = app.detail_read().unwrap();
    app.apply_message(&key, Ok("Full **reply** body\n".repeat(40)));
    let buffer = draw(&app, 80, 40);
    let shown = lines(&buffer).join("\n");
    assert!(shown.contains("from lead-b") && shown.contains("more lines · v view"));
    press(&mut app, Char('v'));
    assert!(app.row_details.reader.is_some());
    press(&mut app, Esc);
    select(&mut app, "needs-you", "a");
    app.home_leads.replies =
        vec![json!({"recipientId":"A","requestId":"member-reply","submittedAtMs":10})];
    press(&mut app, Char('e'));
    let member_target = app.row_target(app.selected).unwrap();
    let key = app.detail_read().unwrap();
    app.apply_message(&key, Ok("A member reply".into()));
    assert!(
        lines(&draw(&app, 80, 40))
            .join("\n")
            .contains("A member reply")
    );
    assert!(app.row_details.contains(&Target::Row(lead_target)));
    assert!(app.row_details.contains(&Target::Row(member_target)));
    press(&mut app, Char('e'));
    app.apply_message(&key, Ok("late collapsed".into()));
    assert!(
        !app.row_details
            .contains(&Target::Row(app.row_target(app.selected).unwrap()))
    );
}

#[test]
fn audience_controls_use_one_composer_and_refuse_sender_or_lead_changes() {
    let mut app = fixture();
    press(&mut app, Char('@'));
    press(&mut app, Down);
    press(&mut app, Enter);
    assert!(app.menu.is_none());
    assert!(
        matches!(&app.input.as_ref().unwrap().compose, Compose::Leads { all: false, recipients, .. } if recipients[0].id == "B")
    );
    press(&mut app, Char('x'));
    assert!(
        matches!(press(&mut app, Enter), Effect::Act(Request::Leads { all: false, text, .. }) if text == "x")
    );
    for actor in [true, false] {
        let mut app = fixture();
        press(&mut app, Char('A'));
        press(&mut app, Char('x'));
        if actor {
            app.view.as_mut().unwrap().me_id = Some("other".into());
        } else {
            app.home_leads.leads[0].row["id"] = json!("replacement");
        }
        assert_eq!(press(&mut app, Enter), Effect::None);
        assert!(app.notice.as_ref().unwrap().contains("nothing sent"));
    }
}

#[test]
fn home_attention_and_boxed_heading_selection_preserve_blanks_and_acquired_previews() {
    crate::status::with_now_ms(1_900_000_000_000, || {
        for width in [80, 100, 160, 180] {
            for (base, depth) in [
                (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::TrueColor),
                (
                    tmt_cli_style::Base::TmtLight,
                    tmt_cli_style::Depth::TrueColor,
                ),
                (tmt_cli_style::Base::Terminal, tmt_cli_style::Depth::Ansi16),
                (tmt_cli_style::Base::Tmt, tmt_cli_style::Depth::None),
            ] {
                let mut app = fixture();
                app.view.as_mut().unwrap().look = crate::look::Look {
                    theme: tmt_cli_style::Theme::new(base),
                    depth,
                };
                let document = app.view.as_ref().unwrap().document.clone();
                let selected = draw(&app, width, 40);
                let hits = format!("{:?}", app.hits.borrow());
                let starts = app.row_starts.borrow().clone();
                let heading = app
                    .hits
                    .borrow()
                    .iter()
                    .find(|hit| hit.row().unwrap() == app.selected)
                    .unwrap()
                    .y;
                assert_eq!(selected[(1, heading)].symbol(), " ");
                assert_eq!(selected[(2, heading)].symbol(), "◆");
                select(&mut app, "needs-you", "a");
                let attention = draw(&app, width, 40);
                assert_eq!(format!("{:?}", app.hits.borrow()), hits);
                assert_eq!(*app.row_starts.borrow(), starts);
                let y = app
                    .hits
                    .borrow()
                    .iter()
                    .find(|hit| hit.row().unwrap() == app.selected)
                    .unwrap()
                    .y;
                assert_eq!(attention[(0, y)].symbol(), " ");
                assert_eq!(attention[(1, y)].symbol(), "◆");
                assert_eq!(
                    attention[(0, y)].bg,
                    app.look().selection().bg.unwrap_or_default()
                );
                assert_eq!(
                    attention[(0, y)]
                        .modifier
                        .contains(ratatui::style::Modifier::REVERSED),
                    app.look().selection().bg.is_none()
                );
                for y in 2..39 {
                    for x in 0..width {
                        assert_eq!(selected[(x, y)].symbol(), attention[(x, y)].symbol());
                    }
                }
                assert_eq!(app.view.as_ref().unwrap().document, document);
            }
        }
    });
}

#[test]
fn a_newer_incoming_question_does_not_hide_the_leads_latest_submitted_reply() {
    let mut app = fixture();
    app.home_leads.replies =
        vec![json!({"recipientId":"A","requestId":"prior-reply","submittedAtMs":10})];
    press(&mut app, Char('e'));
    let key = app
        .detail_read()
        .expect("the submitted reply remains independently readable");
    assert_eq!(key.request, "prior-reply");
    app.apply_message(
        &key,
        Ok("Earlier reply retained beside the new decision".into()),
    );
    let shown = lines(&draw(&app, 80, 40)).join("\n");
    assert!(
        shown.contains("Earlier reply retained") && shown.contains("pending"),
        "{shown}"
    );
    assert!(app.reply_hint());
}
