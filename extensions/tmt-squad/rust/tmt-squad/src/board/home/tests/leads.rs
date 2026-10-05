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
fn lead_rows_and_footer_share_one_cursor_and_hidden_previews_remove_separators() {
    for width in [160, 100, 80] {
        let mut app = fixture();
        assert_eq!(
            app.home_entries()
                .iter()
                .map(|entry| entry.target.section.as_str())
                .collect::<Vec<_>>(),
            [
                "needs-you",
                "leads",
                "leads",
                "all-leads",
                "squads",
                "squads"
            ]
        );
        let shown = draw(&app, width, 40);
        let text = lines(&shown);
        let at = |needle: &str| text.iter().position(|line| line.contains(needle)).unwrap();
        assert!(at("leads · latest") < at("asks: Approve"));
        assert!(at("asks: Approve") < at("→ all leads"));
        assert!(at("→ all leads") < at("── squads"));
        let row = app.selected;
        assert_eq!(
            app.hits
                .borrow()
                .iter()
                .filter(|hit| hit.row == row)
                .count(),
            2
        );
        assert!(text[at("→ all leads") - 1].starts_with('└'));
        assert_eq!(glyph_error(&text.join("\n")), None);
        assert!(
            text.last().unwrap().contains("? more"),
            "help, where the toggle is listed, stays discoverable at 80 columns"
        );
        let target = app.home_target.clone();
        assert_eq!(press(&mut app, Char('t')), Effect::HomeReplies(false));
        app.view.as_mut().unwrap().home_replies = false;
        let hidden = lines(&draw(&app, width, 40));
        assert!(hidden.iter().any(|line| line.contains("t shows replies")));
        assert!(
            !hidden
                .iter()
                .any(|line| line.contains("asks:") || line.contains("The work is ready"))
        );
        assert_eq!(
            app.hits
                .borrow()
                .iter()
                .filter(|hit| hit.row == row)
                .count(),
            1
        );
        assert_eq!(app.home_target, target);
        assert_eq!(
            press(&mut app, Enter),
            Effect::Act(Request::Jump("lead-a".into()))
        );
        press(&mut app, Down);
        press(&mut app, Down);
        assert_eq!(app.home_target.as_ref().unwrap().section, "all-leads");
        assert_eq!(press(&mut app, Enter), Effect::None);
        assert!(
            matches!(&app.input.as_ref().unwrap().compose, Compose::Leads { all: true, recipients, .. } if recipients.len() == 2)
        );
    }
}

#[test]
fn leads_without_exchanges_have_one_hit_line_and_one_blank_after_the_last_exchange() {
    for width in [160, 100, 80] {
        let mut app = fixture();
        app.home_leads.leads[1].exchange = None;
        let text = lines(&draw(&app, width, 40));
        let preview = text
            .iter()
            .position(|line| line.contains("asks: Approve"))
            .unwrap();
        // One blank boxed line keeps `no reply yet` / the question apart from the
        // first lead without an exchange; nothing separates lead from lead below it.
        assert!(
            text[preview + 1].starts_with('│')
                && text[preview + 1].trim_matches(['│', ' ']).is_empty()
        );
        assert!(text[preview + 2].contains("lead-b"));
        assert!(text[preview + 3].starts_with('└'));
        let target = app
            .home_entries()
            .iter()
            .position(|entry| entry.target.section == LEADS && entry.target.squad == "b")
            .unwrap();
        assert_eq!(
            app.hits
                .borrow()
                .iter()
                .filter(|hit| hit.row == target)
                .count(),
            1
        );
        app.home_leads.leads[0].exchange = None;
        let text = lines(&draw(&app, width, 40));
        let heading = text
            .iter()
            .position(|line| line.contains("lead-a") && line.starts_with('│'))
            .unwrap();
        assert!(text[heading + 1].contains("lead-b"));
        assert!(text[heading + 2].starts_with('└'));
        for entry in app
            .home_entries()
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.target.section == LEADS)
        {
            assert_eq!(
                app.hits
                    .borrow()
                    .iter()
                    .filter(|hit| hit.row == entry.0)
                    .count(),
                1
            );
        }
    }
}

#[test]
fn expanded_body_and_answer_share_the_full_inner_band_at_every_width_and_theme() {
    for width in [160, 100, 80] {
        for (base, depth) in [
            ("tmt", tmt_cli_style::Depth::TrueColor),
            ("tmt-light", tmt_cli_style::Depth::TrueColor),
            ("tmt", tmt_cli_style::Depth::None),
        ] {
            let mut app = fixture();
            app.view.as_mut().unwrap().look = crate::look::Look {
                theme: tmt_cli_style::Theme::new(tmt_cli_style::theme::Base::parse(base).unwrap()),
                depth,
            };
            press(&mut app, Char('e'));
            let Compose::ReadLead { key, .. } = app.input.as_ref().unwrap().compose.clone() else {
                panic!("expanded mode")
            };
            app.apply_message(
                &key,
                Ok("First complete line\n\nSecond complete line".into()),
            );
            let buffer = draw(&app, width, 30);
            let band = app.input_band.get().unwrap();
            assert_eq!((band.x, band.width), (1, width - 2));
            let text = lines(&buffer);
            assert!(text[usize::from(band.y - 1)].contains("lead-a"));
            assert!(!text.iter().any(|line| line.contains("asks: Approve")));
            assert!(text.iter().any(|line| line.contains("The work is ready.")));
            assert_eq!(
                app.hits
                    .borrow()
                    .iter()
                    .filter(|hit| hit.row == app.selected)
                    .count(),
                1
            );
            assert!(text[usize::from(band.y + 1)].contains("First complete line"));
            assert!(
                text[usize::from(band.y + 2)]
                    .chars()
                    .all(|c| c == ' ' || c == '│')
            );
            assert!(text[usize::from(band.y + 3)].contains("Second complete line"));
            assert!(!text.join("\n").contains("\\n"));
            assert!(
                text[usize::from(band.bottom() - 2)].contains("e collapse · a reply to lead-a")
            );
            assert!(text[usize::from(band.y)].starts_with("│┌"));
            assert!(
                app.hits
                    .borrow()
                    .iter()
                    .all(|hit| hit.y < band.y || hit.y >= band.bottom())
            );
            press(&mut app, Esc);
            let collapsed = lines(&draw(&app, width, 30));
            assert!(collapsed.iter().any(|line| line.contains("asks: Approve")));
            assert_eq!(
                app.hits
                    .borrow()
                    .iter()
                    .filter(|hit| hit.row == app.selected)
                    .count(),
                2
            );
            press(&mut app, Char('e'));
            press(&mut app, Char('a'));
            assert!(matches!(
                app.input.as_ref().unwrap().compose,
                Compose::Reply { .. }
            ));
            let answer = lines(&draw(&app, width, 30));
            let next = app.input_band.get().unwrap();
            assert_eq!((next.x, next.width), (band.x, band.width));
            assert!(answer[usize::from(next.y + 1)].contains("→ lead-a · answer"));
            assert!(answer[usize::from(next.y + 2)].contains("Approve this change?"));
            press(&mut app, Esc);
            assert!(app.input.is_none());
        }
    }
}

#[test]
fn long_expanded_message_scrolls_and_rejects_changed_exchange_or_actor() {
    let mut app = fixture();
    press(&mut app, Char('e'));
    let Compose::ReadLead { key, .. } = app.input.as_ref().unwrap().compose.clone() else {
        panic!("expanded mode")
    };
    app.apply_message(
        &key,
        Ok((0..50)
            .map(|i| format!("body line {i}"))
            .collect::<Vec<_>>()
            .join("\n")),
    );
    let first = lines(&draw(&app, 80, 12));
    assert!(
        app.input_band.get().is_some(),
        "the entire capped band must fit"
    );
    assert!(first.iter().any(|line| line.contains("body line 0")));
    press(&mut app, PageDown);
    let next = lines(&draw(&app, 100, 12));
    assert!(!next.iter().any(|line| line.contains("body line 0")));
    for _ in 0..50 {
        press(&mut app, PageDown);
    }
    let last = lines(&draw(&app, 80, 12));
    assert!(last.iter().any(|line| line.contains("body line 49")));
    app.home_leads.leads[0].exchange.as_mut().unwrap().request = Some("new-question".into());
    app.apply_message(&key, Ok("late old body".into()));
    assert!(!app.input.as_ref().unwrap().text.contains("late old body"));
    app.view.as_mut().unwrap().me_id = Some("other".into());
    assert!(!app.message_valid());
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
