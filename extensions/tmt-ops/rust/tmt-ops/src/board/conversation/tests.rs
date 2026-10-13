use super::*;
use crate::board::{
    app,
    home_leads::{Kind, MessageKey},
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
use unicode_width::UnicodeWidthStr;

mod performance;

fn fixture() -> App {
    let mut app = App::new(Some("product".into()));
    app.apply(app::tests::snapshot(
        "product",
        json!([{"rows":[
            {"id":"a","name":"alpha","state":"working","fields":{}},
            {"id":"b","name":"beta","state":"waiting","fields":{}}
        ]}]),
    ));
    app.view.as_mut().unwrap().me_id = Some("owner".into());
    app.view.as_mut().unwrap().me = Some("Owner".into());
    app.view.as_mut().unwrap().replies = vec![
        json!({"recipientId":"a","requestId":"ra","response":body("alpha", 90)}),
        json!({"recipientId":"b","requestId":"rb","response":body("beta", 60)}),
    ];
    app
}

fn body(prefix: &str, count: usize) -> String {
    (0..count)
        .map(|n| format!("{prefix} line {n:03}  \n"))
        .collect()
}

fn draw(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render(frame, app, frame.area()))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn reply(request: &str, body: &str) -> Reply {
    Reply {
        from: "alpha".into(),
        age: "now".into(),
        body: body.into(),
        key: Some(MessageKey {
            sender: "owner".into(),
            lead: "a".into(),
            squad: "product".into(),
            request: request.into(),
            kind: Kind::Reply,
        }),
        acquired: true,
        source: true,
    }
}

#[test]
fn scrolling_is_independent_and_follow_only_resumes_explicitly() {
    let app = fixture();
    assert!(draw(&app, 80, 18).contains("alpha line 089"));
    app.scrolls.restore([(Pane::Rows, 7)]);
    app.scroll_pane(Pane::Replies, Step::Top);
    assert!(draw(&app, 80, 18).contains("alpha line 000"));
    app.scroll_pane(Pane::Replies, Step::Pages(1));
    let older = app.scrolls.offset(Pane::Replies);
    assert!(older > 0);
    assert_eq!(app.scrolls.offset(Pane::Rows), 7);
    assert!(draw(&app, 80, 18).contains("reading"));
    app.scroll_pane(Pane::Rows, Step::Top);
    assert_eq!(app.scrolls.offset(Pane::Replies), older);
    app.scroll_pane(Pane::Replies, Step::Bottom);
    assert!(draw(&app, 80, 18).contains("alpha line 089"));
    assert!(draw(&app, 80, 18).contains("following"));
}

#[test]
fn switching_identity_and_renaming_preserves_each_reading_anchor() {
    let mut app = fixture();
    draw(&app, 80, 18);
    app.scroll_pane(Pane::Replies, Step::Top);
    app.scroll_pane(Pane::Replies, Step::Lines(20));
    let alpha = draw(&app, 80, 18);
    assert_eq!(app.scrolls.offset(Pane::Replies), 20);
    assert!(alpha.contains("alpha line 020"));
    app.selected = 1;
    assert!(draw(&app, 80, 18).contains("beta line 059"));
    app.scroll_pane(Pane::Replies, Step::Top);
    app.selected = 0;
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["name"] = json!("renamed");
    let restored = draw(&app, 80, 18);
    assert!(restored.contains("Conversation · renamed"));
    assert!(restored.contains("alpha line 020"));
    app.selected = 1;
    assert!(draw(&app, 80, 18).contains("beta line 000"));
    assert_eq!(app.conversation.borrow().sessions.len(), 2);
}

#[test]
fn new_output_does_not_replace_a_paused_request_and_end_reads_latest() {
    let mut app = fixture();
    draw(&app, 80, 18);
    app.scroll_pane(Pane::Replies, Step::Top);
    app.view.as_mut().unwrap().replies[0] =
        json!({"recipientId":"a","requestId":"next","response":"new final"});
    let paused = draw(&app, 80, 18);
    assert!(paused.contains("new output"));
    assert!(paused.contains("alpha line 000"));
    assert!(!paused.contains("new final"));
    app.scroll_pane(Pane::Replies, Step::Bottom);
    let followed = draw(&app, 80, 18);
    assert!(followed.contains("new final"));
    assert!(!followed.contains("new output"));
    app.scroll_pane(Pane::Replies, Step::Top);
    app.view.as_mut().unwrap().replies.clear();
    let unavailable = draw(&app, 80, 18);
    assert!(unavailable.contains("source unavailable"));
    assert!(unavailable.contains("new final"));
    assert!(!unavailable.contains("new output"));
    app.scroll_pane(Pane::Replies, Step::Bottom);
    assert!(draw(&app, 80, 18).contains("no response in loaded history"));
}

#[test]
fn append_and_resize_keep_source_anchor_without_reparsing_unchanged_text() {
    let mut session = Session::default();
    let source = format!(
        "# Header\n\n{}",
        "界 e\u{301} 👩‍💻 long wrapped source ".repeat(20)
    );
    session.update(Some(reply("r", &source)));
    let look = Look::default();
    assert!(session.prepare(60, look));
    let cached = session.rendered.as_ref().unwrap().mapped.lines.as_ptr();
    session.update(Some(reply("r", &source)));
    assert!(!session.prepare(60, look));
    assert_eq!(
        cached,
        session.rendered.as_ref().unwrap().mapped.lines.as_ptr()
    );
    session.follow = false;
    session.anchor.select_visual(
        &session.text,
        &session.rendered.as_ref().unwrap().mapped.sources,
        4,
    );
    let source_line = session.anchor.source;
    session.update(Some(reply("r", &(source + "\n\nappended"))));
    assert!(session.change == Change::NewOutput);
    assert_eq!(session.anchor.source, source_line);
    for width in [60, 80, 100, 140, 160] {
        session.prepare(width, look);
        let mapped = &session.rendered.as_ref().unwrap().mapped;
        let visual = session.anchor.visual(&mapped.sources).unwrap();
        assert_eq!(mapped.sources[visual], source_line);
        assert!(mapped.lines.iter().all(|line| {
            line.spans
                .iter()
                .map(|span| span.content.width())
                .sum::<usize>()
                <= width
        }));
    }
}

#[test]
fn reacquiring_same_request_keeps_readable_source_but_explicit_failure_does_not() {
    let mut session = Session::default();
    session.update(Some(reply("r", "known body")));
    session.prepare(80, Look::default());
    let cache = session.rendered.as_ref().unwrap().mapped.lines.as_ptr();
    let mut loading = reply("r", "(reading reply…)");
    loading.acquired = false;
    loading.source = false;
    session.update(Some(loading.clone()));
    assert_eq!(session.text, "known body");
    assert_eq!(
        session.rendered.as_ref().unwrap().mapped.lines.as_ptr(),
        cache
    );
    loading.acquired = true;
    loading.body = "(reply expired)".into();
    session.update(Some(loading));
    assert_eq!(session.text, "(reply expired)");
    assert!(!session.reply.as_ref().unwrap().source);
}

#[test]
fn malicious_response_remains_inert_and_full_source_is_available() {
    let mut session = Session::default();
    let raw = "# \u{1b}]0;bad\u{7}Heading\u{202e}\n\n- **bold** and `code`\n\n```text\n\u{1b}[31mred\u{1b}[0m\n```\n[Colab](https://example.test/page)\nFINAL";
    session.update(Some(reply("r", raw)));
    for width in [40, 80, 120] {
        session.prepare(width, Look::default());
        let mapped = &session.rendered.as_ref().unwrap().mapped;
        let text = mapped
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        for expected in ["Heading", "bold", "code", "red", "Colab", "FINAL"] {
            assert!(text.contains(expected));
        }
        assert!(
            !text
                .chars()
                .any(|ch| ch.is_control() && ch != '\n' || ch == '\u{202e}')
        );
        let link = mapped
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .find(|span| span.content.contains("Colab"))
            .unwrap();
        assert!(
            link.style
                .add_modifier
                .contains(ratatui::style::Modifier::UNDERLINED)
        );
        assert_eq!(mapped.lines.len(), mapped.sources.len());
        assert_eq!(
            session.reply.as_ref().unwrap().body,
            raw,
            "escaping does not rewrite quote source"
        );
    }
}

#[test]
fn clicking_response_links_does_not_open_or_execute_destinations() {
    use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    for target in [
        "https://example.test/page",
        "tmt:jump/other",
        "javascript:alert(1)",
    ] {
        let mut app = workspace_fixture();
        app.view.as_mut().unwrap().replies[0]["response"] = json!(format!("[InertLink]({target})"));
        let selected = app.workspace.selected.clone();
        let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
        terminal
            .draw(|frame| {
                crate::board::view::prepare(&mut app, frame.area());
                crate::board::view::render(frame, &app);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let (x, y) = (0..buffer.area.height)
            .find_map(|y| {
                let line: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                line.find("InertLink")
                    .map(|x| (u16::try_from(line[..x].width()).unwrap(), y))
            })
            .expect("the response label is visibly painted");
        let pane = app.conversation.borrow().pane.expect("response pane");
        assert!(pane.contains((x, y).into()));
        assert_eq!(
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: x,
                    row: y,
                    modifiers: KeyModifiers::NONE,
                },
                std::time::Instant::now()
            ),
            app::Effect::None,
            "{target}"
        );
        assert_eq!(app.workspace.selected, selected);
        assert!(app.input.is_none());
        assert!(app.menu.is_none());
    }
}

#[test]
fn narrow_response_header_keeps_source_and_choice_label_together() {
    use crate::labels::{Choose, Label, Supplied};
    let choose = Choose::parse(
        "digest",
        &json!({
            "kind":"choose", "options":[{"label":"Auto","value":"auto"}],
            "current":"auto", "argv":["digest", "member", "{value}"]
        }),
    )
    .unwrap();
    for (width, visible) in [(12, false), (13, true), (40, true)] {
        let mut app = workspace_fixture();
        app.labels = Supplied::from_rows(vec![(
            "digest".into(),
            [(
                crate::board::app::workspace::tests::MEMBER.into(),
                vec![Label {
                    text: "Auto".into(),
                    role: Role::Text,
                    action: Some(choose.clone()),
                }],
            )]
            .into(),
        )]);
        let screen = draw(&app, width, 24);
        assert_eq!(
            screen.contains("digest Auto ▾"),
            visible,
            "{width}: {screen}"
        );
        if !visible {
            assert!(
                !screen.contains("digest"),
                "a source cannot appear without its label"
            );
        }
        assert_eq!(app.chip_hits.borrow().len(), usize::from(visible));
        for (area, row) in app.chip_hits.borrow().iter() {
            assert_eq!(*row, app.selected);
            assert!(area.right() <= width);
        }
    }
}

#[test]
fn sender_identity_and_empty_selection_do_not_reuse_another_conversation() {
    let mut app = fixture();
    draw(&app, 80, 18);
    app.scroll_pane(Pane::Replies, Step::Top);
    app.view.as_mut().unwrap().me_id = Some("other owner".into());
    assert!(draw(&app, 80, 18).contains("alpha line 089"));
    assert_eq!(app.conversation.borrow().sessions.len(), 2);
    app.selected = 100;
    assert!(draw(&app, 80, 18).contains("Select an agent"));
    assert!(app.conversation.borrow().active.is_none());
}

#[test]
fn source_reader_opens_the_paused_response_and_rejects_stale_annotation() {
    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    let mut app = fixture();
    draw(&app, 80, 18);
    app.scroll_pane(Pane::Replies, Step::Top);
    app.view.as_mut().unwrap().replies[0] =
        json!({"recipientId":"a","requestId":"next","response":"replacement source"});
    draw(&app, 80, 18);
    app.view_row_reply();
    let event = |code, modifiers| Event::Key(KeyEvent::new(code, modifiers));
    app.reader_event(&event(KeyCode::Char('s'), KeyModifiers::NONE));
    let selected = app
        .row_details
        .reader
        .as_ref()
        .unwrap()
        .selection
        .as_ref()
        .unwrap();
    assert_eq!(selected.key.request, "ra");
    assert!(selected.source.as_str().starts_with("alpha line"));
    app.reader_event(&event(KeyCode::Char('a'), KeyModifiers::CONTROL));
    assert_eq!(
        app.reader_event(&event(KeyCode::Char('a'), KeyModifiers::NONE)),
        app::Effect::None
    );
    assert!(app.input.is_none());
    assert!(app.notice.as_deref().unwrap().contains("source changed"));
}

#[test]
fn membership_context_changes_do_not_invent_unread_output_or_reparse_source() {
    let mut session = Session::default();
    session.update(Some(reply("r", "same immutable final")));
    session.prepare(80, Look::default());
    let cached = session.rendered.as_ref().unwrap().mapped.lines.as_ptr();
    session.follow = false;
    let mut moved = reply("r", "same immutable final");
    moved.key.as_mut().unwrap().squad = "another membership".into();
    session.update(Some(moved));
    assert!(session.change == Change::Current);
    assert!(!session.prepare(80, Look::default()));
    assert_eq!(
        cached,
        session.rendered.as_ref().unwrap().mapped.lines.as_ptr()
    );
}

#[test]
fn shared_header_uses_runtime_not_vendor_and_preserves_colab_reference() {
    let mut app = fixture();
    app.references = crate::board::inventory::Inventory::from_listed(
        &[],
        &Ok(json!({"identities":[
            {"id":"a","name":"alpha","runningDriver":"pi","resume":{"driver":"codex","model":"model-x","provider":"vendor-y"}}
        ]})),
    );
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["fields"]["link"] =
        json!("https://colab.example/page");
    let shown = draw(&app, 80, 24);
    assert!(shown.contains("pi · model-x"));
    assert!(shown.contains("https://colab.example/page"));
    assert!(!shown.contains("vendor-y") && !shown.contains("codex"));
    assert_eq!(app.references.agents["a"].runtime.as_deref(), Some("pi"));
    // The shared header uses the same precedence as the existing open action.
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["fields"]["pr_link"] =
        json!("https://colab.example/review");
    let shown = draw(&app, 80, 24);
    assert!(shown.contains("https://colab.example/review"));
    assert!(!shown.contains("https://colab.example/page"));
}

#[test]
fn conversation_prefers_latest_identity_result_across_room_contexts() {
    let mut app = fixture();
    app.view.as_mut().unwrap().replies[0]["submittedAtMs"] = json!(10);
    app.home_leads.replies = vec![json!({
        "requestId":"global-latest", "recipientId":"a", "submittedAtMs":20,
        "response":"latest across rooms"
    })];
    assert!(draw(&app, 80, 24).contains("latest across rooms"));
    let first = app.conversation_reply().unwrap();
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["squad"] = json!("new-room");
    assert!(draw(&app, 80, 24).contains("latest across rooms"));
    let second = app.conversation_reply().unwrap();
    assert!(first.key.unwrap().same_source(&second.key.unwrap()));
    app.view.as_mut().unwrap().replies[0]["submittedAtMs"] = json!(30);
    app.view.as_mut().unwrap().replies[0]["response"] = json!("newer room receipt");
    assert!(draw(&app, 80, 24).contains("newer room receipt"));
}

#[test]
fn timer_counts_use_owner_uuid_and_distinguish_unknown_zero_and_stale() {
    use crate::board::cronboard::{test_cron, test_view};
    use tmt_ops::cron::{ClockStatus, Pause};
    let mut app = fixture();
    assert!(draw(&app, 80, 24).contains("timers –"));
    let mut own = test_view("alpha", "first", None);
    own.job.owner_id = Some("a".into());
    let mut paused = own.clone();
    paused.job.pause = Some(Pause {
        by: "owner".into(),
        at_ms: 1,
    });
    let mut other = own.clone();
    other.job.owner_id = Some("b".into());
    app.cron.replace(Ok(test_cron(
        vec![own, paused, other],
        ClockStatus::NoClock,
    )));
    assert!(draw(&app, 80, 24).contains("timers 1/2"));
    app.cron.replace(Err("fixture acquisition failure".into()));
    assert!(draw(&app, 80, 24).contains("timers 1/2 stale"));
    app.cron
        .replace(Ok(test_cron(vec![], ClockStatus::NoClock)));
    assert!(draw(&app, 40, 24).contains("timers 0/0"));
}

fn workspace_fixture() -> App {
    let mut app = crate::board::app::workspace::tests::board();
    app.view.as_mut().unwrap().replies = vec![json!({
        "recipientId":crate::board::app::workspace::tests::MEMBER,
        "requestId":"req_44444444-4444-4444-8444-444444444444",
        "response":body("worker", 90)
    })];
    app
}

#[test]
fn link_hint_uses_effective_action_and_truncates_url_before_hint() {
    let mut app = workspace_fixture();
    let selected = app.workspace.selected.clone().unwrap();
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .rows
        .get_mut(&selected)
        .unwrap()["fields"]["link"] =
        json!(format!("https://example.test/{}", "long-path/".repeat(20)));
    assert!(draw(&app, 60, 24).contains("(press o - open $link)"));
    let path = std::env::temp_dir()
        .join(format!("ops-conversation-link-{}", std::process::id()))
        .join("ops.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    for (text, expected) in [
        ("[bind]\no = 'copy custom'\n", "(press o - copy custom)"),
        ("", "(press o - open $link)"),
    ] {
        std::fs::write(&path, text).unwrap();
        let config = crate::config::Config::read(path.clone()).unwrap();
        app.view
            .as_mut()
            .unwrap()
            .home
            .as_mut()
            .unwrap()
            .workspace
            .configure(&config)
            .unwrap();
        let shown = draw(&app, 60, 24);
        assert!(shown.contains(expected), "{shown}");
        assert!(
            !shown.contains("long-path/long-path/long-path/long-path/long-path/"),
            "URL must truncate before the effective action hint"
        );
    }
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

fn workspace(app: &mut App, width: u16, height: u16) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(app, frame.area());
            crate::board::view::render(frame, app);
        })
        .unwrap();
}

#[test]
fn resize_keeps_one_editor_and_wheel_scrolls_response_without_changing_recipient() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
    let mut app = workspace_fixture();
    let selected = app.workspace.selected.clone();
    workspace(&mut app, 160, 36);
    app.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
    assert!(app.input.is_some());
    app.paste("draft 界 e\u{301}\nsecond line");
    let key = app.input.as_ref().unwrap().draft_key.clone();
    for (width, height) in [(160, 36), (100, 36), (80, 24), (60, 18), (140, 44)] {
        app.invalidate_frame_geometry();
        workspace(&mut app, width, height);
        let (editor, _) = app
            .input_viewport
            .get()
            .expect("visible editor after resize");
        assert!(!editor.is_empty());
        assert_eq!(editor.intersection(Rect::new(0, 0, width, height)), editor);
        assert_eq!(app.input.as_ref().unwrap().draft_key, key);
        assert_eq!(
            app.input.as_ref().unwrap().text,
            "draft 界 e\u{301}\nsecond line"
        );
    }
    let pane = app.conversation.borrow().pane.unwrap();
    let before = app.scrolls.offset(Pane::Replies);
    assert!(before > 0);
    app.mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: pane.x + 1,
            row: pane.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        std::time::Instant::now(),
    );
    assert!(app.scrolls.offset(Pane::Replies) < before);
    assert_eq!(app.input.as_ref().unwrap().draft_key, key);
    assert_eq!(app.workspace.selected, selected);
    assert_eq!(app.focused(), Pane::Replies);
}
