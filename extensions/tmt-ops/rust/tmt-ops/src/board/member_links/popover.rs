//! Destination popovers name the held target before any action. Web/mail links offer
//! Open, Copy and Show full, plus Add to Notes in a conversation. Local files offer
//! only Copy full path and Show full.
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

/// A URL's last path segment, without query/fragment; a bare host remains readable.
fn link_title(url: &str) -> String {
    let address = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim_end_matches('/');
    let title = address
        .rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(address);
    crate::board::notes::single_line(title)
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
        let note = self.note_source_at(hit.area.as_position()).map(|source| {
            self.conversation_notes_key().map_or_else(
                || Choice::Unavailable("Select a squad member to add to Notes.".into()),
                |key| {
                    Choice::AddToNotes(crate::board::conversation_notes::Seed {
                        key,
                        actor: self.view.as_ref().and_then(|view| view.me_id.clone()),
                        kind: crate::board::conversation_notes::NoteKind::Link,
                        title: link_title(url),
                        reference: url.into(),
                        source,
                    })
                },
            )
        });
        self.member_links.borrow_mut().link_note = note.map(|choice| (url.to_owned(), choice));
        self.link_popover(url, hit.area)
    }

    /// The popover for `url` under `anchor`: where it goes, then what to do with it.
    pub(in crate::board) fn link_popover(&mut self, url: &str, anchor: Rect) -> Effect {
        let can_open = self.link_opener().is_some();
        let entries = self.link_entries(url, can_open, anchor);
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
            selected: if can_open { 1 } else { 2 },
            anchor: Some(anchor),
            surface: RefCell::new(None),
        });
        Effect::None
    }

    fn link_entries(&self, url: &str, can_open: bool, anchor: Rect) -> Vec<MenuEntry> {
        let mut entries = actions(url, can_open, anchor);
        if let Some((_, choice)) = self
            .member_links
            .borrow()
            .link_note
            .as_ref()
            .filter(|(held, _)| held == url)
        {
            entries.push(MenuEntry {
                key: "n".into(),
                label: "Add to Notes".into(),
                choice: choice.clone(),
            });
        }
        entries
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
                choice: Choice::FileCopy(path.into()),
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
                Some((Target::File(path.to_string()), anchor))
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
            Target::File(path) => (path.clone(), "Full path", Choice::FileCopy(path.into())),
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
        let entries = app.link_entries(url, false, area);
        let menu = app.menu.as_mut().unwrap();
        menu.entries = entries;
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
    use crate::board::member_links::tests::capture_file_case;
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
    fn n_on_a_link_opens_the_link_draft_without_losing_the_exact_url_or_message() {
        let mut app = reader("Read [the docs](https://example.com/a_b?query=1#frag) now.");
        let (area, url) = url_hits(&app).remove(0);
        click(&mut app, area);
        app.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        let key = app.conversation_notes_key().unwrap();
        let draft = app.notes.draft(&key).expect("n opens Add to Notes");
        assert_eq!(draft.text(0), "a_b");
        assert_eq!(draft.text(1), url);
        let seed = draft.seed.as_ref().unwrap();
        assert_eq!(seed.kind, crate::board::conversation_notes::NoteKind::Link);
        assert!(
            seed.source.body().contains("request link"),
            "{:?}",
            seed.source
        );
        assert!(app.menu.is_none());
        assert!(app.notes_overlay.is_some());
    }

    #[test]
    fn a_stale_add_to_notes_choice_cannot_change_the_selected_conversation() {
        let mut app = reader("Read [the docs](https://example.com/a) now.");
        let (area, _) = url_hits(&app).remove(0);
        click(&mut app, area);
        let choice = app
            .menu
            .take()
            .unwrap()
            .entries
            .into_iter()
            .find(|entry| entry.key == "n")
            .unwrap()
            .choice;
        app.select(0);
        assert_eq!(app.choose(choice), Effect::None);
        assert!(app.notes_overlay.is_none());
    }

    #[test]
    fn full_link_back_keeps_the_opened_messages_source_after_the_same_url_is_refreshed() {
        let mut app = reader("Read [the docs](https://example.com/a) now.");
        let (area, _) = url_hits(&app).remove(0);
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let original = app
            .menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.key == "n")
            .unwrap()
            .choice
            .clone();
        let show = app
            .menu
            .take()
            .unwrap()
            .entries
            .into_iter()
            .find(|entry| entry.key == "f")
            .unwrap()
            .choice;
        app.choose(show);
        app.view.as_mut().unwrap().replies[0]["requestId"] =
            serde_json::json!("replacement-request");
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        assert!(
            app.note_source_at(area.as_position())
                .unwrap()
                .body()
                .contains("replacement-request"),
            "positive control: the background now holds the replacement message"
        );
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let restored = app
            .menu
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .find(|entry| entry.key == "n")
            .unwrap()
            .choice
            .clone();
        assert_eq!(
            restored, original,
            "back restores the held source, not the row now at the anchor"
        );
        app.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        let key = app.conversation_notes_key().unwrap();
        let source = &app.notes.draft(&key).unwrap().seed.as_ref().unwrap().source;
        assert!(source.body().contains("request link"));
        assert!(!source.body().contains("replacement-request"));

        app.notes.drop_draft(&key);
        app.notes_overlay = None;
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        app.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        app.key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        let fresh = &app.notes.draft(&key).unwrap().seed.as_ref().unwrap().source;
        assert!(
            fresh.body().contains("replacement-request"),
            "a new hit of the same URL replaces the old context"
        );
    }

    #[test]
    fn link_titles_use_the_last_path_segment_without_rewriting_the_url() {
        assert_eq!(
            link_title("https://example.com/a/review_123?x=y#z"),
            "review_123"
        );
        assert_eq!(link_title("https://example.com/a/"), "a");
        assert_eq!(link_title("https://example.com"), "example.com");
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
        assert_eq!(
            labels,
            [
                "https://example.com",
                "Open",
                "Copy",
                "Show full",
                "Add to Notes"
            ]
        );
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
        let show = menu
            .entries
            .iter()
            .find(|entry| entry.key == "f")
            .unwrap()
            .choice
            .clone();
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
            .iter()
            .find(|entry| entry.key == "f")
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
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let shown = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(shown.contains("Enter copies · Esc back"), "{shown}");
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

    #[test]
    fn a_long_full_path_keeps_all_its_text_and_copies_from_either_end() {
        let path = format!("/tmp/{}.txt", "file_name_".repeat(500));
        let mut app = reader(&format!("See '{path}' now."));
        let hit = app
            .member_links
            .borrow()
            .hits
            .iter()
            .find(|hit| matches!(hit.target, Target::File(_)))
            .unwrap()
            .clone();
        app.file_show(&path, hit.area);
        let menu = app.menu.as_ref().unwrap();
        assert!(menu.entries.len() > 8, "the full path scrolls");
        assert_eq!(
            menu.entries
                .iter()
                .map(|entry| entry.label.as_str())
                .collect::<String>(),
            path
        );
        let first = menu.entries.first().unwrap().choice.clone();
        let last = menu.entries.last().unwrap().choice.clone();
        for choice in [first, last] {
            assert!(
                matches!(app.choose(choice), Effect::Act(Request::Copy { text, .. }) if text == path)
            );
        }
    }
}
