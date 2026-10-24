//! Destination popovers name the held target before any action. Web/mail links offer
//! Open, Copy and Show full; local files offer only Copy full path and Show full.
use super::{Hit, Target, menu_area};
use crate::board::app::{App, Choice, CopyFeedback, Effect, Menu, MenuEntry, Request};
use crate::effects;
use ratatui::layout::Rect;
use std::cell::RefCell;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Cells a popover line holds: the menu is 48 wide, less borders, key and cursor columns.
const LINE: usize = 40;

/// The address up to and including its host, so a look-alike label cannot hide where it goes.
fn origin(url: &str) -> &str {
    let after = url.find("://").map_or(0, |at| at + 3);
    let end = url[after..]
        .find(['/', '?', '#'])
        .map_or(url.len(), |at| after + at);
    &url[..end]
}

/// `text` cut into lines of at most `width` cells, between characters.
fn chunks(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut used = 0;
    for character in text.chars() {
        let size = character.width().unwrap_or(0);
        if used + size > width {
            lines.push(String::new());
            used = 0;
        }
        lines.last_mut().expect("one line").push(character);
        used += size;
    }
    lines
}

/// The popover's rows: the address up to its host, then what can be done with it.
fn actions(url: &str, can_open: bool, anchor: Rect) -> Vec<MenuEntry> {
    let shown = origin(url);
    let mut entries = vec![MenuEntry {
        key: String::new(),
        label: if shown.width() > LINE {
            crate::board::view::fit(shown, LINE)
        } else {
            shown.to_owned()
        },
        choice: Choice::Heading,
    }];
    entries.push(if can_open {
        MenuEntry {
            key: "o".into(),
            label: "Open".into(),
            choice: Choice::LinkOpen(url.to_owned()),
        }
    } else {
        MenuEntry {
            key: "o".into(),
            label: "Open (no browser here)".into(),
            choice: Choice::Unavailable("No browser here: copy the link".into()),
        }
    });
    entries.push(MenuEntry {
        key: "y".into(),
        label: "Copy".into(),
        choice: Choice::LinkCopy(url.to_owned()),
    });
    entries.push(MenuEntry {
        key: "f".into(),
        label: "Show full".into(),
        choice: Choice::LinkShow {
            url: url.to_owned(),
            anchor,
        },
    });
    entries
}

impl App {
    /// The opener this board uses: the configured one, else what this machine offers.
    fn link_opener(&self) -> Option<Vec<String>> {
        self.view
            .as_ref()
            .and_then(|view| view.opener.clone())
            .or_else(|| effects::detect_opener(&effects::OpenHost::current(), &effects::on_path))
    }

    pub(super) fn open_url_link(&mut self, hit: &Hit) -> Effect {
        let Target::Url(url) = &hit.target else {
            return Effect::None;
        };
        self.link_popover(url, hit.area)
    }

    /// The popover for `url` under `anchor`: where it goes, then what to do with it.
    pub(in crate::board) fn link_popover(&mut self, url: &str, anchor: Rect) -> Effect {
        let can_open = self.link_opener().is_some();
        let entries = actions(url, can_open, anchor);
        if menu_area(
            self.member_links.borrow().screen,
            anchor,
            entries.len() as u16 + 4,
        )
        .is_none()
        {
            return self.say("Not enough room beside this link; resize to open its actions.");
        }
        self.menu = Some(Menu {
            row_send: None,
            link: None,
            prefill: String::new(),
            title: "Link".into(),
            entries,
            // Open is the default; without a browser the default is Copy.
            selected: if can_open { 1 } else { 2 },
            anchor: Some(anchor),
            surface: RefCell::new(None),
        });
        Effect::None
    }

    pub(in crate::board) fn file_popover(&mut self, path: &str, anchor: Rect) -> Effect {
        let entries = vec![
            MenuEntry {
                key: String::new(),
                label: crate::board::view::fit(path, LINE),
                choice: Choice::Heading,
            },
            MenuEntry {
                key: "y".into(),
                label: "Copy full path".into(),
                choice: Choice::FileCopy(path.to_owned()),
            },
            MenuEntry {
                key: "f".into(),
                label: "Show full".into(),
                choice: Choice::FileShow {
                    path: path.to_owned(),
                    anchor,
                },
            },
        ];
        if menu_area(
            self.member_links.borrow().screen,
            anchor,
            entries.len() as u16 + 4,
        )
        .is_none()
        {
            return self.say("Not enough room beside this file; resize to open its actions.");
        }
        self.menu = Some(Menu {
            row_send: None,
            link: None,
            prefill: String::new(),
            title: "File".into(),
            entries,
            selected: 1,
            anchor: Some(anchor),
            surface: RefCell::new(None),
        });
        Effect::None
    }

    /// The destination and anchor whose full-view Esc returns to its popover.
    pub(in crate::board) fn destination_full_menu(&self) -> Option<(Target, Rect)> {
        let menu = self.menu.as_ref()?;
        let anchor = menu.anchor?;
        match menu.entries.first()?.choice.clone() {
            Choice::LinkCopy(url)
                if menu
                    .entries
                    .iter()
                    .all(|entry| entry.choice == Choice::LinkCopy(url.clone())) =>
            {
                Some((Target::Url(url), anchor))
            }
            Choice::FileCopy(path)
                if menu
                    .entries
                    .iter()
                    .all(|entry| entry.choice == Choice::FileCopy(path.clone())) =>
            {
                Some((Target::File(path), anchor))
            }
            _ => None,
        }
    }

    pub(in crate::board) fn link_open(&mut self, url: String) -> Effect {
        match self.link_opener() {
            Some(opener) => Effect::Act(Request::Open {
                link: url,
                opener: Some(opener),
            }),
            None => self.say("No browser here: copy the link"),
        }
    }

    pub(in crate::board) fn link_copy(&mut self, url: String) -> Effect {
        Effect::Act(Request::Copy {
            text: url,
            program: self.view.as_ref().and_then(|view| view.clipboard.clone()),
            feedback: CopyFeedback::Destination,
        })
    }

    /// The whole address, wrapped; Enter on any line copies it, Esc closes.
    pub(in crate::board) fn link_show(&mut self, url: &str, anchor: Rect) -> Effect {
        self.show_destination(Target::Url(url.to_owned()), anchor)
    }

    pub(in crate::board) fn file_show(&mut self, path: &str, anchor: Rect) -> Effect {
        self.show_destination(Target::File(path.to_owned()), anchor)
    }

    fn show_destination(&mut self, target: Target, anchor: Rect) -> Effect {
        let (text, title, copy) = match target {
            Target::Url(url) => (url.clone(), "Full link", Choice::LinkCopy(url)),
            Target::File(path) => (path.clone(), "Full path", Choice::FileCopy(path)),
            Target::Member(_) => return Effect::None,
        };
        let entries: Vec<_> = chunks(&text, LINE)
            .into_iter()
            .map(|label| MenuEntry {
                key: String::new(),
                label,
                choice: copy.clone(),
            })
            .collect();
        if menu_area(
            self.member_links.borrow().screen,
            anchor,
            entries.len().min(8) as u16 + 4,
        )
        .is_none()
        {
            return self.say("Not enough room beside this link; copy it instead.");
        }
        self.menu = Some(Menu {
            row_send: None,
            link: None,
            prefill: String::new(),
            title: title.into(),
            entries,
            selected: 0,
            anchor: Some(anchor),
            surface: RefCell::new(None),
        });
        Effect::None
    }
}

/// The oracle's link scenes: the popover for a link in the chat, without a browser, and the
/// whole address. The address is fixed and synthetic.
#[cfg(test)]
pub(in crate::board) const SCENES: &[&str] = &["link-popover", "link-no-browser", "link-full"];

#[cfg(test)]
pub(in crate::board) const FILE_SCENES: &[&str] = &[
    "file-message",
    "file-popover",
    "file-full",
    "file-inert",
    "file-sent",
];

#[cfg(test)]
pub(in crate::board) fn capture_file_hits(app: &App) -> String {
    format!("{:?}", app.member_links.borrow().hits)
}

#[cfg(test)]
pub(in crate::board) fn capture_file_case(
    scene: &str,
    look: crate::look::Look,
    width: u16,
    height: u16,
) -> App {
    use crate::board::app::workspace::tests::{MEMBER, board};
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = board();
    app.focus = 1;
    let response = if scene == "file-inert" {
        "Code: `/tmp/file_name.rs`\nRelative: src/file_name.rs\nWord: prefix/tmp/file_name.rs\nURL: https://example.com/file_name.rs"
    } else if scene == "file-sent" {
        "A previous answer."
    } else {
        crate::board::markdown::reported::PATH
    };
    {
        let view = app.view.as_mut().unwrap();
        view.look = look;
        view.replies = vec![serde_json::json!({
            "recipientId": MEMBER, "requestId": "file-scene", "response": response,
        })];
        view.home.as_mut().unwrap().workspace.squad_looks.clear();
    }
    let draw = |app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, app))
            .unwrap();
    };
    draw(&app);
    if scene == "file-sent" {
        let owner = app.conversation_finals().unwrap();
        app.apply_sent(crate::board::home_leads::SentKey { sender: owner.sender, agent: owner.agent },
            Ok(vec![serde_json::json!({
                "requestId": "file-sent", "preparedAtMs": crate::status::now_ms(),
                "text": "Before '/tmp/my screenshot_file.png' after ", "truncated": false,
                "mode": "passthrough", "messageState": {"state": "delivered", "reason": {"text": "delivered"}},
                "passthrough": {"state": "pending"}
            })]));
        draw(&app);
    }
    if matches!(scene, "file-popover" | "file-full") {
        let hit = app
            .member_links
            .borrow()
            .hits
            .iter()
            .find(|hit| matches!(hit.target, Target::File(_)))
            .unwrap()
            .clone();
        let Target::File(path) = &hit.target else {
            unreachable!()
        };
        if scene == "file-full" {
            app.file_show(path, hit.area);
        } else {
            app.file_popover(path, hit.area);
        }
    }
    draw(&app);
    app
}

#[cfg(test)]
pub(in crate::board) fn capture_case(
    scene: &str,
    look: crate::look::Look,
    width: u16,
    height: u16,
) -> App {
    use crate::board::app::workspace::tests::{MEMBER, board};
    use ratatui::{Terminal, backend::TestBackend};
    let url = "https://docs.example.com/guides/getting-started/installing-the-board?ref=chat&section=links";
    let mut app = board();
    {
        let view = app.view.as_mut().unwrap();
        view.look = look;
        view.opener = Some(vec!["oracle-open".into(), "{url}".into()]);
        view.replies = vec![serde_json::json!({
            "recipientId": MEMBER,
            "requestId": "link-scene",
            "response": format!("Read [the install guide]({url}) before you start."),
        })];
        let workspace = &mut view.home.as_mut().unwrap().workspace;
        for squad in &workspace.roster.squads {
            workspace.squad_looks.insert(squad.room_id.clone(), look);
        }
    }
    app.focus = 1;
    let draw = |app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, app))
            .unwrap();
    };
    draw(&app);
    let (area, _) = {
        let hits = app.member_links.borrow();
        hits.hits
            .iter()
            .find_map(|hit| matches!(hit.target, Target::Url(_)).then(|| (hit.area, ())))
            .expect("the link is painted at this width")
    };
    let hit = Hit {
        area,
        surface: area,
        target: Target::Url(url.into()),
        reader: true,
        occurrence: 0,
    };
    app.open_url_link(&hit);
    if scene == "link-no-browser" {
        let menu = app.menu.as_mut().unwrap();
        menu.entries = actions(url, false, area);
        menu.selected = 2;
    }
    if scene == "link-full" {
        app.menu = None;
        app.link_show(url, area);
    }
    draw(&app);
    app
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::app::workspace::tests::{MEMBER, board};
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{
            KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        },
    };

    const LINK: &str = "https://example.com/a?b=1;c='x y'";

    fn reader(response: &str) -> App {
        let mut app = board();
        app.view.as_mut().unwrap().replies =
            vec![serde_json::json!({"recipientId":MEMBER,"requestId":"link","response":response})];
        app.view.as_mut().unwrap().opener = Some(vec!["fake-open".into(), "{url}".into()]);
        app.focus = 1;
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        app
    }

    fn url_hits(app: &App) -> Vec<(Rect, String)> {
        app.member_links
            .borrow()
            .hits
            .iter()
            .filter_map(|hit| match &hit.target {
                Target::Url(url) => Some((hit.area, url.clone())),
                Target::Member(_) | Target::File(_) => None,
            })
            .collect()
    }

    fn click(app: &mut App, area: Rect) -> Effect {
        let at = |kind| MouseEvent {
            kind,
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        };
        app.mouse(
            at(MouseEventKind::Down(MouseButton::Left)),
            std::time::Instant::now(),
        );
        app.mouse(
            at(MouseEventKind::Up(MouseButton::Left)),
            std::time::Instant::now(),
        )
    }

    #[test]
    fn the_origin_is_the_scheme_and_host_before_any_path() {
        assert_eq!(
            origin("https://example.com:8080/a/b?c#d"),
            "https://example.com:8080"
        );
        assert_eq!(origin("https://example.com"), "https://example.com");
        assert_eq!(origin("https://example.com?q=1"), "https://example.com");
        assert_eq!(origin("mailto:a@example.com"), "mailto:a@example.com");
    }

    #[test]
    fn a_long_address_wraps_between_characters_and_loses_none() {
        let url = format!("https://example.com/{}", "界".repeat(30));
        let lines = chunks(&url, 10);
        assert!(lines.iter().all(|line| line.width() <= 10), "{lines:?}");
        assert_eq!(lines.concat(), url);
    }

    #[test]
    fn labels_and_bare_addresses_are_links_and_other_schemes_stay_plain() {
        let app = reader(&format!(
            "See [the docs]({}) and https://bare.example/x. Mail [me](mailto:a@example.com) \
             but not [this](javascript:alert) or `https://code.example`.",
            LINK.replace(' ', "%20")
        ));
        // A label wrapped over two lines is two hits of one link.
        let mut urls: Vec<_> = url_hits(&app).into_iter().map(|(_, url)| url).collect();
        urls.dedup();
        assert_eq!(
            urls,
            [
                LINK.replace(' ', "%20"),
                "https://bare.example/x".to_owned(),
                "mailto:a@example.com".to_owned(),
            ]
        );
    }

    #[test]
    fn clicking_a_link_opens_a_popover_that_names_its_target_before_any_action() {
        let mut app = reader("Read [the docs](https://example.com/a?b=1) now.");
        let (area, _) = url_hits(&app).remove(0);
        assert_eq!(click(&mut app, area), Effect::None);
        let menu = app.menu.as_ref().expect("a popover");
        let labels: Vec<_> = menu.entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["https://example.com", "Open", "Copy", "Show full"]);
        assert_eq!(menu.anchor, Some(area));
    }

    #[test]
    fn open_and_copy_carry_the_whole_address_as_one_argument() {
        let mut app = reader("Read [the docs](https://example.com/a?b=1) now.");
        let (area, url) = url_hits(&app).remove(0);
        click(&mut app, area);
        let menu = app.menu.take().unwrap();
        let choice = |key: &str| {
            menu.entries
                .iter()
                .find(|entry| entry.key == key)
                .unwrap()
                .choice
                .clone()
        };
        assert_eq!(
            app.choose(choice("o")),
            Effect::Act(Request::Open {
                link: url.clone(),
                opener: Some(vec!["fake-open".into(), "{url}".into()]),
            })
        );
        assert!(matches!(
            app.choose(choice("y")),
            Effect::Act(Request::Copy { text, .. }) if text == url
        ));
    }

    #[test]
    fn show_full_lists_every_character_and_enter_on_a_line_copies_it_all() {
        let mut app = reader(&format!(
            "Read [it]({}) now.",
            "https://example.com/".to_owned() + &"p".repeat(120)
        ));
        let (area, url) = url_hits(&app).remove(0);
        click(&mut app, area);
        let menu = app.menu.take().unwrap();
        let show = menu.entries.last().unwrap().choice.clone();
        app.choose(show);
        let full = app.menu.as_ref().expect("the full address");
        assert_eq!(
            full.entries
                .iter()
                .map(|e| e.label.as_str())
                .collect::<String>(),
            url
        );
        assert!(full.entries.len() > 1);
        assert!(
            full.entries
                .iter()
                .all(|entry| entry.choice == Choice::LinkCopy(url.clone()))
        );
    }

    #[test]
    fn the_origin_is_a_heading_the_cursor_never_visits_and_no_row_counts() {
        let mut app = reader("Read [the docs](https://example.com/a?b=1) now.");
        let (area, _) = url_hits(&app).remove(0);
        click(&mut app, area);
        let menu = app.menu.as_mut().unwrap();
        assert_eq!(menu.entries[0].choice, Choice::Heading);
        for _ in 0..6 {
            menu.step(-1);
        }
        assert_eq!(menu.selected, 1, "up saturates on Open, not on the heading");
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let text = (0..40)
            .map(|y| {
                (0..120)
                    .map(|x| terminal.backend().buffer()[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !text.contains(" of 4"),
            "no position line when rows fit:\n{text}"
        );
        assert!(text.contains("Enter runs · Esc closes"), "{text}");
    }

    #[test]
    fn esc_in_the_full_address_goes_back_to_the_popover_and_then_closes() {
        let mut app = reader("Read [it](https://example.com/a?b=1) now.");
        let (area, url) = url_hits(&app).remove(0);
        click(&mut app, area);
        let show = app
            .menu
            .take()
            .unwrap()
            .entries
            .last()
            .unwrap()
            .choice
            .clone();
        app.choose(show);
        assert_eq!(app.menu.as_ref().unwrap().title, "Full link");
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let menu = app.menu.as_ref().expect("back at the popover");
        assert_eq!(menu.title, "Link");
        assert_eq!(menu.entries[2].choice, Choice::LinkCopy(url));
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.menu.is_none());
    }

    #[test]
    fn without_a_browser_open_is_disabled_and_copy_stays() {
        let rows = actions("https://example.com/x", false, Rect::new(0, 0, 5, 1));
        assert_eq!(
            rows[1].choice,
            Choice::Unavailable("No browser here: copy the link".into())
        );
        assert!(matches!(rows[2].choice, Choice::LinkCopy(_)));
        let rows = actions("https://example.com/x", true, Rect::new(0, 0, 5, 1));
        assert!(matches!(rows[1].choice, Choice::LinkOpen(_)));
    }

    #[test]
    fn enter_on_a_keyboard_focused_link_opens_its_popover() {
        let mut app = reader("Read [the docs](https://example.com/a) now.");
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert!(app.member_links.borrow().focused.is_some());
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.menu.is_some(), "a popover for the focused link");
    }

    #[test]
    fn file_chips_copy_the_literal_path_and_full_view_returns_to_copy_only_actions() {
        let path = "/tmp/folder with space/file_name.png";
        let mut app = reader(&format!("See '{path}' now."));
        assert!(
            app.member_links.borrow().web_spans().is_empty(),
            "files never emit OSC 8 links"
        );
        let hit = app
            .member_links
            .borrow()
            .hits
            .iter()
            .find(|hit| hit.target == Target::File(path.into()))
            .unwrap()
            .clone();
        assert_eq!(click(&mut app, hit.area), Effect::None);
        let menu = app.menu.as_ref().expect("file popover");
        assert_eq!(menu.title, "File");
        assert_eq!(menu.selected, 1);
        assert_eq!(menu.entries[1].label, "Copy full path");
        assert_eq!(menu.entries[2].label, "Show full");
        assert_eq!(menu.entries.len(), 3, "configured opener does not add Open");
        let copy = menu.entries[1].choice.clone();
        let show = menu.entries[2].choice.clone();
        assert!(
            matches!(app.choose(copy), Effect::Act(Request::Copy { text, .. }) if text == path)
        );
        app.choose(show);
        let full = app.menu.as_ref().unwrap();
        assert_eq!(full.title, "Full path");
        assert_eq!(
            full.entries
                .iter()
                .map(|entry| entry.label.as_str())
                .collect::<String>(),
            path
        );
        assert!(
            full.entries
                .iter()
                .all(|entry| entry.choice == Choice::FileCopy(path.into()))
        );
        let copy = full.entries[0].choice.clone();
        assert!(
            matches!(app.choose(copy), Effect::Act(Request::Copy { text, .. }) if text == path)
        );
        app.file_show(path, hit.area);
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.menu.as_ref().unwrap().title, "File");
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.menu.is_none());
    }

    #[test]
    fn sent_file_chips_use_the_same_copy_only_popover_and_repaint_discards_stale_hits() {
        for width in [60, 80, 160] {
            let mut app = capture_file_case("file-sent", crate::look::Look::default(), width, 36);
            let hit = app
                .member_links
                .borrow()
                .hits
                .iter()
                .find(|hit| matches!(hit.target, Target::File(_)))
                .unwrap()
                .clone();
            assert_eq!(hit.area.intersection(hit.surface), hit.area);
            assert_eq!(click(&mut app, hit.area), Effect::None);
            let menu = app.menu.as_ref().unwrap();
            assert_eq!(menu.title, "File");
            let copy = menu.entries[1].choice.clone();
            assert!(
                matches!(app.choose(copy), Effect::Act(Request::Copy { text, .. }) if text == "/tmp/my screenshot_file.png")
            );
            app.menu = None;
            app.member_links
                .borrow_mut()
                .begin_frame(Rect::new(0, 0, width, 36));
            assert!(app.member_links.borrow().hits.is_empty());
            click(&mut app, hit.area);
            assert!(app.menu.is_none(), "an old chip is no longer actionable");
        }
    }
}
