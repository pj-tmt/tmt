//! The live screen: the selected agent's visible pane in place of the chat, read about once a
//! second while it is shown and never otherwise. It is output to look at; the composer is the
//! only way in.
use super::{ConversationKey, RowTarget, Session};
use crate::board::{
    app::{App, Effect},
    screen_view::{self, Observation, Row},
};
use crate::{config::Pane, look::Look};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use std::time::Duration;
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthChar;

/// Milliseconds between two reads of a shown screen.
const EVERY_MS: u64 = 1000;

impl App {
    pub(in crate::board) fn screen_mode(&self) -> bool {
        self.conversation.borrow().screen_mode
    }

    /// `l`: the chat and the live screen swap places in the Talk pane.
    pub(in crate::board) fn toggle_screen(&mut self) -> Effect {
        let mut state = self.conversation.borrow_mut();
        state.screen_mode = !state.screen_mode;
        // A text selection belongs to the chat; the screen is not selectable.
        state.selection = None;
        if let Some(key) = state.active.clone()
            && let Some(session) = state.sessions.get_mut(&key)
        {
            session.screen_due_ms = 0;
            session.screen_pin = true;
        }
        Effect::None
    }

    /// The read the shown screen wants next: the selected agent while the mode is on, the pane
    /// is visible and the last read is a second old.
    pub(in crate::board) fn pane_screen_read(&self) -> Option<(ConversationKey, u64)> {
        if !self.screen_mode() || !self.scrolls.visible(Pane::Replies) {
            return None;
        }
        let target = self.conversation_target()?;
        let id = self.target_row(&target)?["id"].as_str()?;
        if !crate::board::recent_responses::canonical_identity(id) {
            return None;
        }
        let key = (self.view.as_ref()?.me_id.clone(), id.to_owned());
        let (due, round) = self
            .conversation
            .borrow()
            .sessions
            .get(&key)
            .map_or((0, 0), |session| {
                (session.screen_due_ms, session.screen_round)
            });
        (crate::status::now_ms() >= due).then_some((key, round))
    }

    pub(in crate::board) fn apply_pane_screen(
        &self,
        key: ConversationKey,
        round: u64,
        observation: Observation,
    ) {
        if self.pane_screen_read() != Some((key.clone(), round)) {
            return;
        }
        let mut state = self.conversation.borrow_mut();
        let session = state.sessions.entry(key).or_default();
        session.apply_screen(observation, crate::status::now_ms());
    }

    /// How long until the shown screen is due for its next read.
    pub(in crate::board) fn screen_wait(&self) -> Option<Duration> {
        if !self.screen_mode() || !self.scrolls.visible(Pane::Replies) {
            return None;
        }
        let state = self.conversation.borrow();
        let due = state
            .active
            .as_ref()
            .and_then(|key| state.sessions.get(key))
            .map_or(0, |session| session.screen_due_ms);
        Some(Duration::from_millis(
            due.saturating_sub(crate::status::now_ms()),
        ))
    }
}

impl Session {
    /// A failed read after a good one keeps the screen it had, marked stale; a pane that is
    /// gone replaces it.
    fn apply_screen(&mut self, observation: Observation, now_ms: u64) {
        self.screen_round += 1;
        self.screen_failed = None;
        // Each failed read doubles the wait, from 2 s to 8 s; a good one resets it.
        let failed = matches!(observation, Observation::Unavailable(_));
        self.screen_failures = if failed { self.screen_failures + 1 } else { 0 };
        self.screen_due_ms = now_ms
            + if failed {
                EVERY_MS << self.screen_failures.min(3)
            } else {
                EVERY_MS
            };
        match observation {
            Observation::Unavailable(reason) if matches!(self.screen, Observation::Screen(_)) => {
                self.screen_failed = Some(reason);
            }
            other => self.screen = other,
        }
    }

    /// The rule above the screen: what it is, its size, and how old.
    pub(super) fn screen_title(&self, width: usize, now_ms: u64) -> String {
        let mut title = match &self.screen {
            Observation::Pending => "Live screen · reading".to_owned(),
            Observation::Gone => "Live screen · pane gone".to_owned(),
            Observation::Unavailable(_) => "Live screen · unavailable".to_owned(),
            Observation::Screen(screen) => format!(
                "Live screen {}×{}{} · updated {} ago",
                screen.width,
                screen.rows.len(),
                if screen.width > width {
                    " · clipped"
                } else {
                    ""
                },
                crate::requests::age(now_ms, screen.read_ms),
            ),
        };
        if self.screen_failed.is_some() {
            title.push_str(" · stale");
        }
        title
    }
}

/// `row` cut to `width` cells.
fn clip<'a>(row: &Row, width: usize, look: Look) -> Line<'a> {
    let mut used = 0;
    let mut spans = Vec::new();
    for run in row {
        let mut text = String::new();
        for character in run.text.chars() {
            let size = character.width().unwrap_or(0);
            if used + size > width {
                break;
            }
            used += size;
            text.push(character);
        }
        let cut = text.len() < run.text.len();
        if !text.is_empty() {
            spans.push(Span::styled(text, screen_view::style(run.attr, look)));
        }
        if cut {
            break;
        }
    }
    Line::from(spans)
}

fn key_of(app: &App, target: &RowTarget) -> Option<ConversationKey> {
    let id = app.target_row(target)?["id"].as_str()?;
    Some((app.view.as_ref()?.me_id.clone(), id.to_owned()))
}

/// The rule above `target`'s live screen.
pub(in crate::board) fn title(app: &App, target: &RowTarget, width: usize) -> String {
    let state = app.conversation.borrow();
    let session = key_of(app, target).and_then(|key| state.sessions.get(&key));
    session.map_or_else(
        || "Live screen · reading".to_owned(),
        |session| session.screen_title(width, crate::status::now_ms()),
    )
}

/// Paints `target`'s live screen into `area`, in place of its exchanges.
pub(in crate::board) fn render(frame: &mut Frame, app: &App, area: Rect, target: &RowTarget) {
    let Some(key) = key_of(app, target) else {
        return;
    };
    let mut state = app.conversation.borrow_mut();
    let switched = state.active.as_ref() != Some(&key);
    state.active = Some(key.clone());
    state.response_area = None;
    let session = state.sessions.entry(key).or_default();
    if switched {
        session.screen_pin = true;
    }
    paint(frame, app, session, area, app.look());
}

/// Paints the screen, or the one line that says why there is none, into `body`.
pub(super) fn paint(frame: &mut Frame, app: &App, session: &mut Session, body: Rect, look: Look) {
    let say = |frame: &mut Frame, text: String| {
        tmt_tui::components::strip::paint_left(
            frame.buffer_mut(),
            Rect::new(body.x, body.y, body.width, 1),
            Line::styled(
                crate::board::view::fit(&text, usize::from(body.width)),
                look.role(Role::Muted),
            ),
        );
    };
    match &session.screen {
        Observation::Pending => say(frame, "Reading the screen…".into()),
        Observation::Gone => say(frame, "This pane is gone. Pick the agent again.".into()),
        Observation::Unavailable(reason) => say(
            frame,
            format!("The screen cannot be read: {reason}. Trying again."),
        ),
        Observation::Screen(screen) => {
            let lines: Vec<Line> = screen
                .rows
                .iter()
                .map(|row| clip(row, usize::from(body.width), look))
                .collect();
            if session.screen_pin {
                // Opening the screen starts where a terminal's prompt is.
                app.scrolls.restore([(Pane::Replies, lines.len())]);
                session.screen_pin = false;
            }
            app.scrolls
                .show_with(frame, Pane::Replies, body, &lines, look, |_, line| {
                    line.clone()
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{
        app::workspace::tests::{MEMBER, board},
        refresh::SelectedRead,
        screen_view::Screen,
    };
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    };

    fn reader() -> App {
        let mut app = board();
        app.focus = 1;
        draw(&app);
        app
    }
    fn draw(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        (0..40)
            .map(|y| {
                (0..160)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn press_l(app: &mut App) {
        app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
    }
    fn screen(text: &str, read_ms: u64) -> Observation {
        let rows = screen_view::parse(text);
        let width = rows.iter().map(screen_view::width).max().unwrap_or(0);
        Observation::Screen(Screen {
            rows,
            width,
            read_ms,
        })
    }
    fn key_of(app: &App) -> ConversationKey {
        (app.view.as_ref().unwrap().me_id.clone(), MEMBER.to_owned())
    }

    #[test]
    fn l_swaps_the_chat_for_the_live_screen_and_back() {
        let mut app = reader();
        assert!(!app.screen_mode());
        assert!(!matches!(
            app.selected_read(),
            Some(SelectedRead::PaneScreen(..))
        ));
        press_l(&mut app);
        assert!(app.screen_mode());
        assert_eq!(
            app.selected_read(),
            Some(SelectedRead::PaneScreen(key_of(&app), 0))
        );
        assert!(draw(&app).contains("Live screen"));
        press_l(&mut app);
        assert!(!app.screen_mode());
        assert!(!draw(&app).contains("Live screen"));
    }

    #[test]
    fn a_bound_l_keeps_its_binding() {
        let mut app = reader();
        let path =
            std::env::temp_dir().join(format!("ops-screen-bind-{}.toml", std::process::id()));
        std::fs::write(&path, "[bind]\nl = 'run recorder {id}'\n").unwrap();
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
        std::fs::remove_file(path).unwrap();
        press_l(&mut app);
        assert!(!app.screen_mode(), "an authored l wins");
    }

    #[test]
    fn a_read_is_due_once_a_second_and_only_while_the_screen_is_shown() {
        let mut app = reader();
        crate::status::with_now_ms(10_000, || {
            press_l(&mut app);
            let key = key_of(&app);
            app.apply_pane_screen(key.clone(), 0, screen("hello", 10_000));
            // Read, not yet due again: nothing to ask for, and a wait of a second.
            assert_eq!(app.pane_screen_read(), None);
            assert_eq!(app.screen_wait(), Some(Duration::from_millis(1000)));
            // A stale answer for round 0 cannot be applied twice.
            app.apply_pane_screen(key.clone(), 0, Observation::Gone);
            assert!(draw(&app).contains("hello"));
        });
        crate::status::with_now_ms(11_000, || {
            assert_eq!(
                app.pane_screen_read(),
                Some((key_of(&app), 1)),
                "due a second after the last read"
            );
        });
        press_l(&mut app);
        assert_eq!(app.pane_screen_read(), None);
        assert_eq!(
            app.screen_wait(),
            None,
            "no wake-ups when the chat is shown"
        );
    }

    #[test]
    fn colours_survive_to_the_painted_cells_and_depth_none_keeps_attributes() {
        let mut app = reader();
        press_l(&mut app);
        app.apply_pane_screen(
            key_of(&app),
            0,
            screen("\u{1b}[31mred\u{1b}[0m and \u{1b}[1mbold\u{1b}[0m", 1),
        );
        let mut terminal = Terminal::new(TestBackend::new(160, 40)).unwrap();
        terminal
            .draw(|frame| crate::board::view::render(frame, &app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let cell = (0..40)
            .flat_map(|y| (0..160).map(move |x| (x, y)))
            .find(|(x, y)| {
                buffer[(*x, *y)].symbol() == "r"
                    && buffer[(*x + 1, *y)].symbol() == "e"
                    && buffer[(*x + 2, *y)].symbol() == "d"
            })
            .expect("the word on screen");
        assert_eq!(
            buffer[cell].fg,
            ratatui::style::Color::Indexed(1),
            "the screen's red, not the board's text colour"
        );
    }

    #[test]
    fn a_wide_screen_is_clipped_to_the_pane_and_says_so() {
        let mut app = reader();
        press_l(&mut app);
        app.apply_pane_screen(key_of(&app), 0, screen(&"x".repeat(400), 1));
        let text = draw(&app);
        assert!(text.contains("clipped"), "{text}");
        assert!(text.contains("400×1"), "{text}");
        assert!(!text.contains(&"x".repeat(200)));
    }

    #[test]
    fn a_gone_pane_and_an_unreadable_screen_each_say_what_to_do() {
        let mut app = reader();
        press_l(&mut app);
        app.apply_pane_screen(key_of(&app), 0, Observation::Gone);
        let text = draw(&app);
        assert!(
            text.contains("This pane is gone. Pick the agent again."),
            "{text}"
        );
        let mut app = reader();
        press_l(&mut app);
        app.apply_pane_screen(
            key_of(&app),
            0,
            Observation::Unavailable("the host did not answer in time".into()),
        );
        assert!(draw(&app).contains("the host did not answer in time"));
    }

    #[test]
    fn a_failed_read_keeps_the_last_screen_marked_stale_and_backs_off() {
        let mut app = reader();
        press_l(&mut app);
        let key = key_of(&app);
        let mut round = 0;
        let mut at = 10_000;
        let mut wait = Vec::new();
        crate::status::with_now_ms(at, || {
            app.apply_pane_screen(key.clone(), round, screen("last good", at));
        });
        for _ in 0..5 {
            round += 1;
            at = {
                let due = app.conversation.borrow().sessions[&key].screen_due_ms;
                due
            };
            crate::status::with_now_ms(at, || {
                assert_eq!(app.pane_screen_read(), Some((key.clone(), round)));
                app.apply_pane_screen(key.clone(), round, Observation::Unavailable("down".into()));
            });
            wait.push(app.conversation.borrow().sessions[&key].screen_due_ms - at);
        }
        assert_eq!(wait, [2000, 4000, 8000, 8000, 8000]);
        let text = crate::status::with_now_ms(at + 1, || draw(&app));
        assert!(text.contains("last good"), "{text}");
        assert!(text.contains("stale"), "{text}");
        // A good read resets the back-off.
        round += 1;
        crate::status::with_now_ms(at + 8000, || {
            app.apply_pane_screen(key.clone(), round, screen("fresh", at + 8000));
        });
        let due = app.conversation.borrow().sessions[&key].screen_due_ms;
        assert_eq!(due - (at + 8000), 1000);
    }
}
