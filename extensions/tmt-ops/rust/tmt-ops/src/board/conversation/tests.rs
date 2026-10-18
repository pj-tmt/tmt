use super::*;
use crate::board::{
    app,
    home_leads::{Kind, MessageKey},
};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
use unicode_width::UnicodeWidthStr;

pub(in crate::board) mod performance;
mod recent_output_packet;

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
        at_ms: None,
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
fn intentional_wheel_or_page_reaching_the_painted_bottom_resumes_follow() {
    for height in [5, 18, 36] {
        for down in [Step::Lines(isize::MAX), Step::Pages(isize::MAX / 100)] {
            let mut app = fixture();
            draw(&app, 80, height);
            app.scroll_pane(Pane::Replies, Step::Top);
            assert!(draw(&app, 80, height).contains("alpha line 000"));
            app.scroll_pane(Pane::Replies, Step::Lines(1));
            assert!(
                !app.conversation
                    .borrow()
                    .sessions
                    .values()
                    .next()
                    .unwrap()
                    .follow
            );
            draw(&app, 80, height);
            app.view.as_mut().unwrap().replies[0] =
                json!({"recipientId":"a","requestId":"new","response":"new correlated source"});
            draw(&app, 80, height);
            assert!(
                !app.conversation
                    .borrow()
                    .sessions
                    .values()
                    .next()
                    .unwrap()
                    .follow
            );
            app.scroll_pane(Pane::Replies, down);
            assert!(
                app.conversation
                    .borrow()
                    .sessions
                    .values()
                    .next()
                    .unwrap()
                    .follow
            );
            assert!(draw(&app, 80, height).contains("new correlated source"));
        }
    }
}

#[test]
fn resize_update_and_missing_frame_never_resume_a_paused_conversation() {
    let mut app = fixture();
    draw(&app, 80, 18);
    app.scroll_pane(Pane::Replies, Step::Top);
    app.scroll_pane(Pane::Replies, Step::Lines(70));
    draw(&app, 80, 18);
    for (width, height) in [(60, 36), (100, 18), (160, 44)] {
        app.view.as_mut().unwrap().replies[0]["response"] = json!(body("alpha", 100));
        draw(&app, width, height);
        assert!(
            !app.conversation
                .borrow()
                .sessions
                .values()
                .next()
                .unwrap()
                .follow
        );
        app.scroll_pane(Pane::Replies, Step::Lines(0));
        assert!(
            !app.conversation
                .borrow()
                .sessions
                .values()
                .next()
                .unwrap()
                .follow
        );
    }
    app.invalidate_frame_geometry();
    app.scroll_pane(Pane::Replies, Step::Lines(isize::MAX));
    assert!(
        !app.conversation
            .borrow()
            .sessions
            .values()
            .next()
            .unwrap()
            .follow
    );
    draw(&app, 100, 18);
    app.scroll_pane(Pane::Replies, Step::Lines(1));
    assert!(
        !app.conversation
            .borrow()
            .sessions
            .values()
            .next()
            .unwrap()
            .follow
    );
    // A missing frame did not move the source anchor to the bottom. A new
    // deliberate move must reach the newly painted viewport's actual limit.
    app.scroll_pane(Pane::Replies, Step::Lines(isize::MAX));
    assert!(
        app.conversation
            .borrow()
            .sessions
            .values()
            .next()
            .unwrap()
            .follow
    );
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
    assert!(session.prepare(60, look, &Default::default()));
    let cached = session.rendered.as_ref().unwrap().mapped.lines.as_ptr();
    session.update(Some(reply("r", &source)));
    assert!(!session.prepare(60, look, &Default::default()));
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
        session.prepare(width, look, &Default::default());
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
    session.prepare(80, Look::default(), &Default::default());
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
        session.prepare(width, Look::default(), &Default::default());
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
    for (width, visible) in [(14, false), (15, true), (40, true)] {
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
    session.prepare(80, Look::default(), &Default::default());
    let cached = session.rendered.as_ref().unwrap().mapped.lines.as_ptr();
    session.follow = false;
    let mut moved = reply("r", "same immutable final");
    moved.key.as_mut().unwrap().squad = "another membership".into();
    session.update(Some(moved));
    assert!(session.change == Change::Current);
    assert!(!session.prepare(80, Look::default(), &Default::default()));
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
fn shared_header_marks_a_basic_driver_instead_of_hiding_the_runtime_line() {
    let mut app = fixture();
    let listed = |extra: serde_json::Value| {
        let mut row = json!({"id":"a","name":"alpha"});
        row.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        Ok(json!({"identities":[row]}))
    };
    app.references = crate::board::inventory::Inventory::from_listed(
        &[],
        &listed(json!({"driverSupport":"basic"})),
    );
    let shown = draw(&app, 80, 24);
    assert!(shown.contains("basic · –"), "{shown}");
    // Nothing observed and no basic marker: the line stays hidden, as before.
    app.references = crate::board::inventory::Inventory::from_listed(&[], &listed(json!({})));
    assert!(!draw(&app, 80, 24).contains("basic"));
    // An observed runtime and model always win over the marker.
    app.references = crate::board::inventory::Inventory::from_listed(
        &[],
        &listed(json!({"driverSupport":"basic","runningDriver":"pi","resume":{"model":"model-x"}})),
    );
    let shown = draw(&app, 80, 24);
    assert!(
        shown.contains("pi · model-x") && !shown.contains("basic ·"),
        "{shown}"
    );
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
    assert!(draw(&app, 80, 24).contains("◷ unknown"));
    let mut own = test_view("alpha", "first", None);
    own.job.owner_id = Some("a".into());
    let mut paused = crate::board::cronboard::test_views(2, "alpha", "first")[1].clone();
    paused.job.owner_id = Some("a".into());
    paused.job.pause = Some(Pause {
        by: "owner".into(),
        at_ms: 1,
    });
    let mut other = crate::board::cronboard::test_views(3, "alpha", "first")[2].clone();
    other.job.owner_id = Some("b".into());
    app.cron.replace(Ok(test_cron(
        vec![own, paused, other],
        ClockStatus::NoClock,
    )));
    assert!(draw(&app, 80, 24).contains("◷ 1/2"));
    app.cron.replace(Err("fixture acquisition failure".into()));
    assert!(draw(&app, 80, 24).contains("◷ unknown"));
    app.cron
        .replace(Ok(test_cron(vec![], ClockStatus::NoClock)));
    assert!(draw(&app, 40, 24).contains("◷ 0/0"));
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
fn link_preview_uses_effective_action_without_printing_inline_key_hints() {
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
    let first = draw(&app, 60, 24);
    assert!(first.contains("https://example.test/"));
    assert!(!first.contains("(press ") && !first.contains("$link"));
    let path = std::env::temp_dir()
        .join(format!("ops-conversation-link-{}", std::process::id()))
        .join("ops.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    for (text, expected) in [
        ("[bind]\no = 'copy custom'\n", "copy from the selected row"),
        ("", "https://example.test/"),
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
        assert!(!shown.contains("(press ") && !shown.contains("$link"));
        let open = app.workspace_bindings()["o"].clone();
        match app.perform(&open) {
            crate::board::app::Effect::Act(crate::board::app::Request::Open { link, .. }) => {
                assert_eq!(
                    link,
                    format!("https://example.test/{}", "long-path/".repeat(20))
                )
            }
            crate::board::app::Effect::Act(crate::board::app::Request::Copy { text, .. }) => {
                assert_eq!(text, "custom")
            }
            other => panic!("effective authored action: {other:?}"),
        }
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
fn switching_shell_keeps_uuid_and_link_while_overview_discusses_only_an_observed_question() {
    use crate::board::shell::OpsTab;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = workspace_fixture();
    crate::board::app::workspace::tests::observe_waiting_member(&mut app);
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
        .unwrap()["fields"]["link"] = json!("https://example.test/pr");
    let mut terminal = Terminal::new(TestBackend::new(160, 36)).unwrap();
    for (key, tab) in [
        ('2', OpsTab::Agents),
        ('1', OpsTab::Overview),
        ('3', OpsTab::Digest),
        ('2', OpsTab::Agents),
    ] {
        app.focus = 0;
        app.key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
        assert_eq!(app.shell.tab, tab);
        terminal
            .draw(|frame| {
                crate::board::view::prepare(&mut app, frame.area());
                crate::board::view::render(frame, &app);
            })
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        if tab == OpsTab::Digest {
            assert!(text.contains("Details"), "{text}");
            assert!(
                !text.contains("worker line 089"),
                "Digest shows status details"
            );
        } else if tab == OpsTab::Overview {
            assert!(text.contains("No question observed."), "{text}");
            assert!(!text.contains("Loading recent pane output"), "{text}");
            assert!(
                !text.contains("worker line 089"),
                "an unrelated outgoing result is not the incoming question"
            );
            assert_eq!(
                app.workspace_bindings()["o"].verb,
                crate::action::Verb::Open
            );
        } else {
            assert!(
                text.contains("https://example.test/pr")
                    && !text.contains("(press ")
                    && !text.contains("$link"),
                "{tab:?}: {text}"
            );
            assert!(text.contains("worker line 089"), "{tab:?}: {text}");
        }
        assert_eq!(app.workspace.selected.as_ref(), Some(&selected));
        assert!(app.conversation.borrow().pane.is_some());
    }
}

#[test]
fn response_wheel_uses_painted_viewport_independently_of_keyboard_focus() {
    use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
    for layout in [
        crate::config::OpsLayout::Side,
        crate::config::OpsLayout::Stack,
    ] {
        for focus in [0, 1] {
            let mut app = workspace_fixture();
            app.view
                .as_mut()
                .unwrap()
                .home
                .as_mut()
                .unwrap()
                .workspace
                .preferences
                .layout = layout;
            app.focus = focus;
            workspace(&mut app, 160, 36);
            let selected = app.workspace.selected.clone();
            let keyboard = app.focused_pane();
            let rows = app.scrolls.offset(Pane::Rows);
            let pane = app.conversation.borrow().pane.unwrap();
            let latest = app.scrolls.offset(Pane::Replies);
            assert!(latest > 0);
            let wheel = |app: &mut App, kind| {
                app.mouse(
                    MouseEvent {
                        kind,
                        column: pane.x + 1,
                        row: pane.y + 1,
                        modifiers: KeyModifiers::NONE,
                    },
                    std::time::Instant::now(),
                )
            };
            assert_eq!(wheel(&mut app, MouseEventKind::ScrollUp), app::Effect::None);
            workspace(&mut app, 160, 36);
            let older = app.scrolls.offset(Pane::Replies);
            assert!(older < latest);
            assert_eq!(app.scrolls.offset(Pane::Rows), rows);
            assert_eq!(app.workspace.selected, selected);
            assert_eq!(app.focused_pane(), keyboard);
            assert_eq!(
                wheel(&mut app, MouseEventKind::ScrollDown),
                app::Effect::None
            );
            workspace(&mut app, 160, 36);
            assert!(app.scrolls.offset(Pane::Replies) > older);
            assert_eq!(app.workspace.selected, selected);
            assert_eq!(app.focused_pane(), keyboard);
        }
    }
}

#[test]
fn resize_keeps_one_editor_and_wheel_scrolls_response_without_changing_recipient() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
    let mut app = workspace_fixture();
    let selected = app.workspace.selected.clone();
    workspace(&mut app, 160, 36);
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
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

#[test]
fn focus_uses_held_identity_stat_tiles_and_uuid_fenced_details_action() {
    use ratatui::crossterm::event::{
        KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    let mut app = workspace_fixture();
    app.references = crate::board::inventory::Inventory::from_listed(
        &[],
        &Ok(json!({"identities":[{
            "id":crate::board::app::workspace::tests::MEMBER,"name":"worker","runningDriver":"runtime-long-界","resume":{"model":"model-long-value"}
        }]})),
    );
    let mut terminal = Terminal::new(TestBackend::new(160, 44)).unwrap();
    terminal
        .draw(|frame| {
            crate::board::view::prepare(&mut app, frame.area());
            crate::board::view::render(frame, &app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let screen: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(screen.contains("product · member"));
    assert!(screen.contains("runtime-long-界") && screen.contains("model-long-value"));
    for caption in ["TOK 1h", "STATE", "DIGEST", "TIMERS"] {
        assert!(screen.contains(caption));
    }
    assert!(!screen.contains(" Details   Links "));
    assert!(
        app.conversation.borrow().header_hits.is_empty(),
        "retired inline controls grant no hits"
    );
    let selected = app.workspace.selected.clone().unwrap();
    let action = app
        .workspace_bindings()
        .into_iter()
        .find(|(_, action)| {
            action.verb == crate::action::Verb::Ui(crate::action::OpsUiAction::MonitorDetails)
        })
        .unwrap()
        .1;
    app.perform(&action);
    assert_eq!(app.monitor_details.as_ref().unwrap().target, selected);
    assert!(app.input.is_none());
    assert_eq!(
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        crate::board::app::Effect::None
    );
    assert!(app.monitor_details.is_none());
    assert_eq!(app.selected, 2);
    app.invalidate_frame_geometry();
    assert!(app.conversation.borrow().header_hits.is_empty());
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 78,
        row: 9,
        modifiers: KeyModifiers::NONE,
    };
    app.mouse(click, std::time::Instant::now());
    assert!(
        app.monitor_details.is_none(),
        "an unpainted metadata cell grants no details authority"
    );
}

#[test]
fn shortened_or_missing_header_rows_never_publish_timer_or_details_hits() {
    for width in [1, 10, 13, 20, 40, 60, 100] {
        for height in [0, 1, 3, 6, 8, 18, 24, 44] {
            let app = workspace_fixture();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    app.cron_scope_hits.borrow_mut().clear();
                    render(frame, &app, frame.area());
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            for hit in &app.conversation.borrow().header_hits {
                let painted: String = (hit.area.x..hit.area.right())
                    .map(|x| buffer[(x, hit.area.y)].symbol())
                    .collect();
                assert!(
                    painted == " Details " || painted == " Links ",
                    "{width}x{height}: {painted}"
                );
            }
            for hit in app.cron_scope_hits.borrow().iter() {
                let painted: String = (hit.area.x..hit.area.right())
                    .map(|x| buffer[(x, hit.area.y)].symbol())
                    .collect();
                assert!(
                    painted.contains('◷'),
                    "{width}x{height}: a replaced Response row cannot inherit timer authority: {painted}"
                );
            }
        }
    }
}

#[test]
fn compact_identity_hides_uuid_and_selected_host_action_survives_pane_focus() {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = workspace_fixture();
    for focused in [false, true] {
        if focused {
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
        workspace(&mut app, 100, 18);
        let mut terminal = Terminal::new(TestBackend::new(100, 18)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let shown: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(shown.contains("tmux"));
        assert!(!shown.contains(crate::board::app::workspace::tests::MEMBER));
        assert_eq!(
            app.workspace_bindings()["j"].verb,
            crate::action::Verb::Jump
        );
        assert_eq!(
            app.workspace_bindings()["pageup"].verb,
            if focused {
                crate::action::Verb::Ui(crate::action::OpsUiAction::ScrollPageUp)
            } else {
                crate::action::Verb::Ui(crate::action::OpsUiAction::PageUp)
            }
        );
        if !focused {
            assert!(shown.contains("j host"));
        }
    }
}

#[test]
fn response_position_counts_retained_lines_rather_than_empty_viewport_cells() {
    for width in [160, 100] {
        let mut app = workspace_fixture();
        app.view.as_mut().unwrap().replies.clear();
        app.home_leads.replies.clear();
        let mut terminal = Terminal::new(TestBackend::new(width, 18)).unwrap();
        terminal
            .draw(|frame| {
                crate::board::view::prepare(&mut app, frame.area());
                crate::board::view::render(frame, &app);
            })
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        let state = app.conversation.borrow();
        let identity = state.active.as_ref().expect("selected UUID session");
        let mapped = &state.sessions[identity].rendered.as_ref().unwrap().mapped;
        assert_eq!(
            mapped.lines.len(),
            1,
            "only the retained unavailable observation is content"
        );
        assert_eq!(app.scrolls.offset(Pane::Replies), 0);
        assert!(
            !screen.contains("earlier"),
            "empty viewport cells are not retained lines"
        );
        assert!(screen.contains("Exchange history unavailable"));
        assert!(!screen.contains("Loading recent pane output"));
    }
}

#[test]
fn member_projection_keeps_exact_response_and_refresh_invalidates_only_the_display_cache() {
    let text = |lines: &[Line<'_>]| {
        lines
            .iter()
            .flat_map(|line| &line.spans)
            .map(|span| span.content.as_ref())
            .collect::<String>()
    };
    let raw = "Ask #tmt:alpha; keep #tmt:missing literal.";
    let mut inventory = crate::board::inventory::Inventory::from_listed(
        &[],
        &Ok(json!({"identities":[{"id":"a","name":"Alpha","canonicalName":"alpha"}]})),
    );
    let mut session = Session::default();
    session.update(Some(reply("raw-mention-request", raw)));
    assert!(session.prepare(80, Look::default(), &inventory));
    let display = text(&session.rendered.as_ref().unwrap().mapped.lines);
    assert!(display.contains("Ask Alpha; keep #tmt:missing literal."));
    assert_eq!(
        session.reply.as_ref().unwrap().body,
        raw,
        "exact reply/copy source remains original markup"
    );
    assert_eq!(
        session.text, raw,
        "source line anchors are based on the original sanitized source"
    );
    assert_eq!(
        session
            .reply
            .as_ref()
            .unwrap()
            .key
            .as_ref()
            .unwrap()
            .request,
        "raw-mention-request"
    );
    assert!(
        !session.prepare(80, Look::default(), &inventory),
        "ordinary frames reuse the resolved render cache"
    );
    inventory = crate::board::inventory::Inventory::from_listed(
        &[],
        &Ok(json!({"identities":[{"id":"a","name":"Renamed","canonicalName":"renamed"}]})),
    );
    assert!(session.prepare(80, Look::default(), &inventory));
    assert!(
        text(&session.rendered.as_ref().unwrap().mapped.lines).contains("#tmt:alpha"),
        "a refreshed canonical name withdraws the old display link"
    );
    assert!(
        session
            .rendered
            .as_ref()
            .unwrap()
            .mapped
            .mentions
            .iter()
            .flatten()
            .next()
            .is_none()
    );
    assert_eq!(session.reply.as_ref().unwrap().body, raw);
}

#[test]
fn pane_output_is_neutral_bounded_and_never_a_request_source() {
    use crate::board::recent_responses::{Item, Observation};
    use ratatui::style::Modifier;
    let mut session = Session::default();
    session.update(Some(reply(
        "real-request",
        "real exchange\nlast source line",
    )));
    session.transcript_context = transcript::Context {
        enabled: true,
        name: "alpha".into(),
        times: vec![1000],
        exchanges_observed: true,
        output_ages: vec!["now".into()],
    };
    session.pane_output = Observation::Observed(vec![Item {
        text: "untrusted **pane**\r\nlast line\n".into(),
        ended_ms: 1000,
        truncated: true,
    }]);
    let original = match &session.pane_output {
        Observation::Observed(items) => items[0].text.clone(),
        _ => unreachable!(),
    };
    for width in [20, 60, 140] {
        session.prepare(width, Look::default(), &Default::default());
        let rendered = session.rendered.as_ref().unwrap();
        let labels: Vec<_> = rendered
            .mapped
            .lines
            .iter()
            .map(ToString::to_string)
            .collect();
        let header = labels
            .iter()
            .position(|line| line.contains("pane output"))
            .unwrap();
        assert!(
            labels[0].contains("real exchange"),
            "exchange wins equal-time tie"
        );
        assert!(header > 0);
        for (index, source) in rendered.mapped.sources.iter().enumerate() {
            if index >= header {
                assert!(
                    session.response_key_at(*source).is_none(),
                    "pane line {index} has no request authority"
                );
            } else {
                assert_eq!(
                    session.response_key_at(*source).unwrap().0.request,
                    "real-request"
                );
            }
        }
        let line = &rendered.mapped.lines[header];
        assert!(!line.style.add_modifier.contains(Modifier::BOLD));
        assert!(
            line.spans
                .iter()
                .all(|span| !span.style.add_modifier.contains(Modifier::BOLD))
        );
        assert_eq!(line.style.fg, Look::default().role(Role::Muted).fg);
        assert!(
            labels
                .iter()
                .any(|line| line.starts_with(" │ ") && line.contains("untrusted"))
        );
        assert_eq!(labels.last().unwrap(), "… truncated");
        assert!(
            rendered
                .mapped
                .mentions
                .iter()
                .skip(header)
                .all(Vec::is_empty)
        );
    }
    assert_eq!(
        match &session.pane_output {
            Observation::Observed(items) => &items[0].text,
            _ => unreachable!(),
        },
        &original,
        "display never rewrites captured bytes"
    );
}

#[test]
fn pane_output_pending_unavailable_and_observed_empty_are_distinct() {
    use crate::board::recent_responses::Observation;
    let mut session = Session {
        transcript_context: transcript::Context {
            enabled: true,
            name: "alpha".into(),
            times: vec![],
            exchanges_observed: true,
            output_ages: vec![],
        },
        ..Default::default()
    };
    session.update(Some(reply("known", "known exchange")));
    session.prepare(60, Look::default(), &Default::default());
    let pending_lines: Vec<_> = session
        .rendered
        .as_ref()
        .unwrap()
        .mapped
        .lines
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(
        pending_lines
            .iter()
            .any(|line| line.contains("known exchange"))
    );
    assert!(
        !pending_lines
            .iter()
            .any(|line| line.contains("recent pane output"))
    );
    session.pane_output = Observation::Observed(vec![]);
    session.rendered = None;
    session.prepare(60, Look::default(), &Default::default());
    assert_eq!(
        pending_lines,
        session
            .rendered
            .as_ref()
            .unwrap()
            .mapped
            .lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        "pending and observed-empty add nothing to known exchanges"
    );
    session.update(None);
    session.prepare(60, Look::default(), &Default::default());
    let empty_lines: Vec<_> = session
        .rendered
        .as_ref()
        .unwrap()
        .mapped
        .lines
        .iter()
        .map(ToString::to_string)
        .collect();
    assert!(
        empty_lines
            .iter()
            .any(|line| line.contains("(no response in loaded history)")),
        "{empty_lines:?}"
    );
    assert!(
        !empty_lines
            .iter()
            .any(|line| line.contains("Exchange history unavailable"))
    );
    session = Session {
        transcript_context: transcript::Context {
            enabled: true,
            name: "alpha".into(),
            exchanges_observed: true,
            ..Default::default()
        },
        ..Default::default()
    };
    session.pane_output = Observation::Unavailable;
    session.prepare(60, Look::default(), &Default::default());
    assert_eq!(
        session
            .rendered
            .as_ref()
            .unwrap()
            .mapped
            .lines
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["Recent pane output unavailable"]
    );
    assert!(session.response_key_at(0).is_none());
    session.pane_output = Observation::Observed(vec![]);
    session.transcript_context.exchanges_observed = false;
    session.rendered = None;
    session.prepare(60, Look::default(), &Default::default());
    assert_eq!(
        session.rendered.as_ref().unwrap().mapped.lines[0].to_string(),
        "Exchange history unavailable"
    );
}

#[test]
fn ten_long_pane_outputs_share_scroll_follow_and_leave_real_exchange_reachable() {
    use crate::board::recent_responses::{Item, Observation};
    let mut app = fixture();
    app.view.as_mut().unwrap().document["sections"][0]["rows"][0]["id"] =
        json!("11111111-1111-4111-8111-111111111111");
    app.view.as_mut().unwrap().replies[0]["recipientId"] =
        json!("11111111-1111-4111-8111-111111111111");
    draw(&app, 80, 18);
    let key = app.pane_output_read().expect("visible exact UUID");
    app.apply_pane_output(
        key.clone(),
        Observation::Observed(
            (0..10)
                .map(|index| Item {
                    text: format!("pane {index}\n{}\nend {index}", "long line\n".repeat(600)),
                    ended_ms: 1000 + index,
                    truncated: false,
                })
                .collect(),
        ),
    );
    assert!(
        app.pane_output_read().is_none(),
        "one read per held observation"
    );
    let bottom = draw(&app, 80, 18);
    assert!(bottom.contains("end 9"));
    app.scroll_pane(Pane::Replies, Step::Top);
    assert!(draw(&app, 80, 18).contains("alpha line 000"));
    let offset = app.scrolls.offset(Pane::Replies);
    draw(&app, 60, 18);
    assert_eq!(
        app.scrolls.offset(Pane::Replies),
        offset,
        "resize does not resume follow"
    );
    app.scroll_pane(Pane::Replies, Step::Bottom);
    assert!(draw(&app, 60, 18).contains("end 9"));
    app.conversation.borrow_mut().refresh_pane_output();
    assert_eq!(app.pane_output_read(), Some(key));
}

#[test]
fn digest_details_do_not_admit_recent_output_even_with_the_shared_scroll_pane() {
    let mut app = workspace_fixture();
    workspace(&mut app, 160, 36);
    let key = app
        .pane_output_read()
        .expect("Agents admits the selected UUID");
    app.apply_pane_output(
        key,
        crate::board::recent_responses::Observation::Observed(vec![
            crate::board::recent_responses::Item {
                text: "Agents-only pane capture".into(),
                ended_ms: 1000,
                truncated: false,
            },
        ]),
    );
    workspace(&mut app, 160, 36);
    assert!(draw(&app, 160, 36).contains("Agents-only pane capture"));
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
        .unwrap()["waitingOnYou"] = json!([{"requestId":"req_55555555-5555-4555-8555-555555555555","preparedAtMs":1000,"preview":"A real waiting question keeps Overview's response viewport visible."}]);
    app.shell.tab = crate::board::shell::OpsTab::Overview;
    workspace(&mut app, 160, 36);
    assert!(
        app.scrolls.visible(Pane::Replies),
        "a waiting Overview question really paints the shared viewport"
    );
    assert!(
        app.pane_output_read().is_none(),
        "Overview keeps its existing question-only owner"
    );
    assert!(
        !draw(&app, 160, 36).contains("Agents-only pane capture"),
        "cached Agents captures never leak into Overview"
    );
    assert!(
        !app.conversation
            .borrow()
            .sessions
            .values()
            .any(|session| session.transcript_context.enabled)
    );
    app.view
        .as_mut()
        .unwrap()
        .home
        .as_mut()
        .unwrap()
        .workspace
        .rows
        .get_mut(&selected)
        .unwrap()["waitingOnYou"] = json!([]);
    app.shell.tab = crate::board::shell::OpsTab::Digest;
    workspace(&mut app, 160, 36);
    assert!(
        app.scrolls.visible(Pane::Replies),
        "Digest Details is genuinely painted"
    );
    assert!(
        app.pane_output_read().is_none(),
        "Details is not a conversation read owner"
    );
    assert!(app.selected_read().is_none());
}

#[test]
fn pane_output_uses_the_exchange_header_time_format() {
    use crate::board::recent_responses::{Item, Observation};
    for (age, expected) in [
        ("now", "now"),
        ("–", "—"),
        ("—", "—"),
        ("1m", "1m ago"),
        ("6h", "6h ago"),
    ] {
        assert_eq!(exchange::age_stamp(age), expected);
    }
    crate::status::with_now_ms(30_000_000, || {
        let app = workspace_fixture();
        let row = json!({"id":"11111111-1111-4111-8111-111111111111","name":"alpha"});
        let mut session = Session {
            pane_output: Observation::Observed(vec![Item {
                text: "pane body".into(),
                ended_ms: 8_400_000,
                truncated: false,
            }]),
            ..Default::default()
        };
        session.transcript_context(&app, &row);
        session.prepare(60, Look::default(), &Default::default());
        assert!(
            session
                .rendered
                .as_ref()
                .unwrap()
                .mapped
                .lines
                .iter()
                .any(|line| line.width() == 60
                    && line.to_string().trim_end() == "alpha · pane output · 6h ago")
        );
    });
}

#[test]
fn grouped_wrap_keeps_the_full_body_and_invalidates_only_layout_not_source() {
    let mut session = Session::default();
    let raw = "abcdefghijklmnopqrstuvwxyz0123456789";
    session.update(Some(reply("grouped-width", raw)));
    for width in [8, 16, 40] {
        assert!(session.prepare_grouped(width, Look::default(), &Default::default()));
        let rendered = session.rendered.as_ref().unwrap();
        let text: String = rendered.mapped.lines.iter().map(Line::to_string).collect();
        assert_eq!(text, raw);
        assert!(
            rendered
                .mapped
                .lines
                .iter()
                .all(|line| line.width() + 3 <= width)
        );
        assert_eq!(session.reply.as_ref().unwrap().body, raw);
        assert_eq!(
            session
                .response_key_at(rendered.mapped.sources[0])
                .unwrap()
                .0
                .request,
            "grouped-width"
        );
        assert!(!session.prepare_grouped(width, Look::default(), &Default::default()));
        assert!(session.prepare(width, Look::default(), &Default::default()));
    }
}

/// Whether every session follows new output, for tests that drive an App.
pub(in crate::board) fn set_follow(app: &App, follow: bool) {
    for session in app.conversation.borrow_mut().sessions.values_mut() {
        session.follow = follow;
    }
}

/// Overwrites the age string every held reply was built with.
pub(in crate::board) fn set_built_age(app: &App, age: &str) {
    for session in app.conversation.borrow_mut().sessions.values_mut() {
        if let Some(reply) = session.reply.as_mut() {
            reply.age = age.into();
        }
    }
}
