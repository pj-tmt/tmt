//! Disposable theme preview. The opening Config is the save baseline;
//! refresh snapshots update the board, never this unsaved draft.

use super::picker_surface;
use crate::{config::Config, core::SquadError, look::Look, theme::ThemeScope};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode},
    layout::Rect,
};
use serde_json::json;
use std::{cell::RefCell, sync::OnceLock};
use tmt_cli_style::Base;
use tmt_tui::components::{ListRow, PickerEvent, PickerField, PickerInput, surface};

const FILE: &str = "squad.theme-picker.xml";
const MARKUP: &str = r#"<tmt-view version="1"><tmt-modal id="theme-picker" title="theme · all boards" placement="center" class="w-72"><tmt-scroll id="choices-body"><tmt-table id="choices" bind="$.rows"><tmt-row class="grid grid-cols-[10_1_1fr] gap-0"><tmt-cell bind="row.name" token="text"/><tmt-cell bind="row.cursor" token="text"/><tmt-cell bind="row.description" wrap="true" token="text"/></tmt-row></tmt-table><tmt-repeat each="$.notes" as="note"><tmt-text bind="note.text" token="muted" wrap="true"/></tmt-repeat></tmt-scroll><tmt-text id="scope" slot="query" bind="$.query" token="accent"/><tmt-text slot="status" bind="$.status" token="blocked"/><tmt-text slot="footer" bind="$.footer" token="muted"/></tmt-modal></tmt-view>"#;
fn template(squad: bool) -> &'static surface::Template<()> {
    static BOARD: OnceLock<surface::Template<()>> = OnceLock::new();
    static SQUAD: OnceLock<surface::Template<()>> = OnceLock::new();
    let template = if squad { &SQUAD } else { &BOARD };
    template.get_or_init(|| {
        picker_surface::compile(
            FILE,
            &if squad {
                MARKUP.replace("theme · all boards", "theme · this squad")
            } else {
                MARKUP.into()
            },
            picker_surface::schema(&["name", "cursor", "description"]),
        )
    })
}

pub(super) struct Picker {
    config: Config,
    squad: Option<String>,
    pub scope: ThemeScope,
    pub(super) surface: RefCell<picker_surface::State>,
    pub notice: Option<String>,
    preview: tmt_cli_style::Theme,
}

pub(super) enum Input {
    Preview,
    Cancel,
    Save,
}

impl Picker {
    pub fn open(config: Config, squad: Option<String>) -> Result<Self, SquadError> {
        let name = squad.as_deref().unwrap_or("");
        let selected = config.theme(name)?.0.base;
        let scope = ThemeScope::Board;
        let preview = config.preview_theme_base(&scope, selected, name)?;
        Ok(Self {
            config,
            squad,
            scope,
            surface: RefCell::new(picker_surface::State::new(
                None,
                Base::ALL
                    .into_iter()
                    .map(|base| ListRow {
                        id: base.name().into(),
                        disabled: false,
                    })
                    .collect(),
                Some(selected.name()),
            )),
            notice: None,
            preview,
        })
    }

    pub fn preview(&self, depth: tmt_cli_style::Depth) -> Look {
        self.preview_with_background(depth, crate::look::background())
    }

    fn preview_with_background(
        &self,
        depth: tmt_cli_style::Depth,
        signal: Option<tmt_cli_style::theme::background::Background>,
    ) -> Look {
        Look {
            theme: self.preview.resolve(signal),
            depth,
        }
    }

    pub fn masked(&self) -> Option<String> {
        let name = self.squad.as_deref()?;
        if self.scope != ThemeScope::Board || self.config.theme_source(name).ok()? != "squad" {
            return None;
        }
        Some(format!(
            "squad {name} keeps {} (its own setting) · r reset in ,",
            self.config.theme(name).ok()?.0.base.name()
        ))
    }

    pub fn selected(&self) -> Base {
        Base::parse(
            self.surface
                .borrow()
                .picker
                .list
                .selected()
                .expect("theme choice"),
        )
        .expect("built-in base")
    }
    pub fn input(&mut self, event: &Event) -> Option<Input> {
        if matches!(event, Event::Key(_)) {
            self.notice = None;
        }
        let previous = (self.scope.clone(), self.selected());
        if matches!(event, Event::Key(key) if key.code == KeyCode::Tab) {
            if let Some(name) = &self.squad {
                self.scope = match self.scope {
                    ThemeScope::Board => ThemeScope::Squad(name.clone()),
                    ThemeScope::Squad(_) => ThemeScope::Board,
                };
            }
        } else {
            let input = self.surface.borrow_mut().input(event, PickerField::List);
            if input.is_none() && !matches!(event, Event::Key(_)) {
                return None;
            }
            match input {
                Some(PickerInput::Event(PickerEvent::Cancel)) => return Some(Input::Cancel),
                Some(PickerInput::Event(PickerEvent::Confirm(_))) => return Some(Input::Save),
                _ => {}
            }
        }
        if previous != (self.scope.clone(), self.selected()) {
            self.preview = self
                .config
                .preview_theme_base(
                    &self.scope,
                    self.selected(),
                    self.squad.as_deref().unwrap_or(""),
                )
                .expect("opening validates the layers and built-in bases are valid");
        }
        Some(Input::Preview)
    }

    pub fn save(&mut self) -> Result<bool, SquadError> {
        self.config.set_theme_base(&self.scope, self.selected())
    }

    pub fn saved_message(&self, changed: bool) -> String {
        format!(
            "{} theme {} for {} (board only)",
            if changed { "Set" } else { "Kept" },
            self.selected().name(),
            self.scope.label()
        )
    }
}

pub(super) fn render(frame: &mut Frame, picker: &Picker, look: Look, body: Rect) {
    let mut notes =
        vec![json!({"id":"board-only", "text":"Board only; CLI colors stay unchanged"})];
    if let Some(masked) = picker.masked() {
        notes.insert(0, json!({"id":"masked", "text":masked}));
    }
    let query = if picker.squad.is_some() {
        if picker.scope == ThemeScope::Board {
            "all boards · Tab: this squad"
        } else {
            "this squad · Tab: all boards"
        }
    } else {
        "all boards"
    };
    let selected = picker
        .surface
        .borrow()
        .picker
        .list
        .selected()
        .map(str::to_owned);
    let value = json!({
        "rows": Base::ALL.into_iter().map(|base| json!({"id":base.name(), "disabled":false,"name":base.name(),"cursor":if selected.as_deref() == Some(base.name()) {"›"} else {""},"description":base.description()})).collect::<Vec<_>>(),
        "query":query, "notes":notes, "status":picker.notice.as_deref().unwrap_or(""),
        "footer":"Enter save · Esc cancel · Tab scope",
    });
    picker.surface.borrow_mut().render(
        FILE,
        template(picker.scope != ThemeScope::Board),
        value,
        frame,
        look,
        body,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::app::{App, Effect};
    use ratatui::style::Modifier;
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    };
    use serde_json::json;
    use tmt_cli_style::Role;

    fn fixture(name: &str, text: &str) -> (std::path::PathBuf, Config) {
        let directory =
            std::env::temp_dir().join(format!("tmt-picker-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("ops.toml");
        std::fs::write(&path, text).unwrap();
        let config = Config::read(path.clone()).unwrap();
        (path, config)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn auto_is_first_previews_detection_and_saves_the_requested_base() {
        use tmt_cli_style::{Depth, theme::background::Background};
        let (path, config) = fixture("auto", "# keep me\n");
        let mut picker = Picker::open(config, None).unwrap();
        assert_eq!(picker.selected(), Base::Auto);
        assert_eq!(Base::ALL[0], Base::Auto);
        assert_eq!(
            Base::Auto.description(),
            "match your terminal (light or dark)"
        );
        assert_eq!(
            picker
                .preview_with_background(Depth::TrueColor, Some(Background::Light))
                .theme
                .base,
            Base::TmtLight
        );
        assert_eq!(
            picker
                .preview_with_background(Depth::TrueColor, Some(Background::Dark))
                .theme
                .base,
            Base::Tmt
        );
        assert_eq!(
            picker
                .preview_with_background(Depth::TrueColor, None)
                .theme
                .base,
            Base::Tmt
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# keep me\n");
        assert!(picker.save().unwrap());
        assert_eq!(
            Config::read(path.clone())
                .unwrap()
                .theme("")
                .unwrap()
                .0
                .base,
            Base::Auto
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("# keep me\n")
        );
        assert!(!picker.save().unwrap());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn preview_scope_mask_and_cancel_follow_the_actual_layers_without_writes() {
        let original = "[board.theme]\nbase = \"tmt\"\n[squad.product.theme]\nbase = \"mono\"\n";
        let (path, config) = fixture("preview-mask", original);
        let mut app = App::new(Some("product".into()));
        app.apply(crate::board::app::tests::snapshot("product", json!([])));
        assert_eq!(
            app.key(key(KeyCode::Char('T'))),
            Effect::PickTheme,
            "no selected row is needed"
        );
        app.theme_picker = Some(Picker::open(config, Some("product".into())).unwrap());
        assert_eq!(app.theme_picker.as_ref().unwrap().scope, ThemeScope::Board);
        assert_eq!(
            app.theme_picker.as_ref().unwrap().masked().as_deref(),
            Some("squad product keeps mono (its own setting) · r reset in ,")
        );
        app.key(key(KeyCode::Tab));
        assert_eq!(
            app.theme_picker.as_ref().unwrap().scope,
            ThemeScope::Squad("product".into())
        );
        app.key(key(KeyCode::Up));
        assert_eq!(app.look().theme.base, Base::Terminal);
        app.key(key(KeyCode::Tab));
        assert_eq!(
            app.look().theme.base,
            Base::Mono,
            "the saved squad base masks the board draft"
        );
        assert_eq!(
            app.theme_picker.as_ref().unwrap().masked().as_deref(),
            Some("squad product keeps mono (its own setting) · r reset in ,")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        let mut snapshot = crate::board::app::tests::snapshot("product", json!([]));
        snapshot.view.as_mut().unwrap().look.theme.base = Base::TmtLight;
        app.apply(snapshot);
        assert_eq!(
            app.look().theme.base,
            Base::Mono,
            "a refresh does not erase the draft"
        );
        assert_eq!(app.key(key(KeyCode::Esc)), Effect::None);
        assert!(app.theme_picker.is_none());
        assert_eq!(
            app.look().theme.base,
            Base::TmtLight,
            "cancel uses the latest saved snapshot"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn picker_isolates_underlying_keys_mouse_and_scope_on_builtin_tabs() {
        let (path, config) = fixture("isolation", "");
        let mut app = App::new(Some(crate::board::ALL.into()));
        app.apply(crate::board::app::tests::snapshot(
            crate::board::ALL,
            json!([]),
        ));
        app.theme_picker = Some(Picker::open(config, None).unwrap());
        let focus = app.focus;
        let current = app.current.clone();
        for code in [
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Char('t'),
            KeyCode::Char('/'),
            KeyCode::Char('?'),
            KeyCode::Char('s'),
            KeyCode::Tab,
        ] {
            assert_eq!(app.key(key(code)), Effect::None);
        }
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(app.mouse(mouse, std::time::Instant::now()), Effect::None);
        assert_eq!(app.focus, focus);
        assert_eq!(app.current, current);
        assert!(!app.searching && !app.help && app.switcher.is_none() && app.input.is_none());
        assert_eq!(app.theme_picker.as_ref().unwrap().scope, ThemeScope::Board);
        let refresh = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);
        assert_eq!(app.key(refresh), Effect::None, "picker isolates ctrl-r");
        assert_eq!(app.key(key(KeyCode::Esc)), Effect::None);
        assert_eq!(app.key(refresh), Effect::Refresh, "normal refresh resumes");
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Effect::Quit
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn stale_save_after_board_reload_preserves_the_file_and_unsaved_picker() {
        let (path, config) = fixture("stale-save", "[board.theme]\nbase = \"tmt\"\n");
        let mut app = App::new(Some("product".into()));
        app.apply(crate::board::app::tests::snapshot("product", json!([])));
        app.theme_picker = Some(Picker::open(config, Some("product".into())).unwrap());
        app.key(key(KeyCode::Down));
        std::fs::write(&path, "# newer file\n[board.theme]\nbase = \"mono\"\n").unwrap();
        app.apply(crate::board::app::tests::snapshot("product", json!([])));
        assert_eq!(app.key(key(KeyCode::Enter)), Effect::SaveTheme);
        let picker = app.theme_picker.as_mut().unwrap();
        let error = picker.save().unwrap_err();
        assert_eq!(error.code, "SQUAD_CONFIG_CHANGED");
        assert!(error.message.contains("retry"));
        assert_eq!(picker.selected(), Base::TmtLight);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# newer file\n[board.theme]\nbase = \"mono\"\n"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn preview_restyles_tabs_and_summary_and_retains_explicit_tokens() {
        use crate::attention::Attention;
        use tmt_cli_style::{Base, Depth, Theme};
        use unicode_width::UnicodeWidthStr;
        for override_text in ["", "waiting = \"#010203\"\n"] {
            let text = format!("[board.theme]\nbase = \"tmt\"\n{override_text}");
            let (path, config) = fixture("header-preview", &text);
            let mut app = App::new(Some("product".into()));
            let mut snapshot = crate::board::app::tests::snapshot("product", json!([]));
            snapshot.attention.insert(
                "product".into(),
                Attention {
                    waiting: 1,
                    blocked: 0,
                },
            );
            snapshot.view.as_mut().unwrap().look = Look {
                theme: Theme::new(Base::Tmt),
                depth: Depth::TrueColor,
            };
            app.apply(snapshot);
            app.theme_picker = Some(Picker::open(config, Some("product".into())).unwrap());
            for _ in 0..3 {
                app.key(key(KeyCode::Down));
            }
            assert_eq!(app.look().theme.base, Base::Mono);
            let mut terminal = Terminal::new(TestBackend::new(108, 28)).unwrap();
            terminal
                .draw(|frame| super::super::view::render(frame, &app))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let expected = app.look().role(Role::Waiting).fg.unwrap_or_default();
            let find = |row: u16, text: &str| {
                let line: String = (0..108).map(|x| buffer[(x, row)].symbol()).collect();
                line[..line.find(text).expect("header text")].width() as u16
            };
            assert_eq!(
                buffer[(find(0, "◆"), 0)].fg,
                expected,
                "tab mark uses the preview, including retained tokens"
            );
            assert_eq!(
                buffer[(find(1, " · 1 waiting"), 1)].fg,
                app.look().role(Role::Text).fg.unwrap_or_default(),
                "summary uses previewed text while the mark keeps its retained token"
            );
            if override_text.is_empty() {
                assert_eq!(expected, ratatui::style::Color::Reset);
            } else {
                assert_eq!(expected, ratatui::style::Color::Rgb(1, 2, 3));
            }
            app.key(key(KeyCode::Esc));
            assert_eq!(app.look().theme.base, Base::Tmt);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
    }

    #[test]
    fn overlay_draws_descriptions_scope_note_footer_and_selection_at_each_depth() {
        let (path, config) = fixture("render", "[squad.product.theme]\nbase = \"mono\"\n");
        let picker = Picker::open(config, Some("product".into())).unwrap();
        for depth in [
            tmt_cli_style::Depth::TrueColor,
            tmt_cli_style::Depth::Ansi16,
            tmt_cli_style::Depth::None,
        ] {
            let look = picker.preview(depth);
            let mut terminal = Terminal::new(TestBackend::new(90, 22)).unwrap();
            terminal
                .draw(|frame| render(frame, &picker, look, frame.area()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let content: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            for text in [
                "soft truecolor for dark terminals",
                "bold and dim only",
                "all boards",
                "this squad",
                "Board only; CLI colors stay unchanged",
                "Enter save · Esc cancel · Tab scope",
                "squad product keeps mono (its own setting)",
            ] {
                assert!(content.contains(text), "missing {text}");
            }
            let (x, y) = (0..22)
                .find_map(|y| {
                    let line: String = (0..90).map(|x| buffer[(x, y)].symbol()).collect();
                    line.find("mono      ")
                        .map(|x| (unicode_width::UnicodeWidthStr::width(&line[..x]) as u16, y))
                })
                .expect("mono preset row");
            let selected = &buffer[(x, y)];
            assert_eq!(selected.symbol(), "m");
            assert_eq!(selected.bg, look.selection().bg.unwrap_or_default());
            assert_eq!(
                selected.modifier.contains(Modifier::REVERSED),
                look.selection().add_modifier.contains(Modifier::REVERSED)
            );
            for (width, height) in [(1, 1), (20, 5), (40, 10)] {
                let mut narrow = Terminal::new(TestBackend::new(width, height)).unwrap();
                narrow
                    .draw(|frame| render(frame, &picker, look, frame.area()))
                    .unwrap();
            }
        }
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn theme_cursor_uses_only_the_gap_and_keeps_description_start() {
        let original = "[board.theme]\nbase = 'tmt'\n";
        let (path, config) = fixture("cursor-track", original);
        let picker = Picker::open(config, None).unwrap();
        for (variant, look) in picker_surface::evidence::looks().into_iter().enumerate() {
            for (width, height) in [(80, 30), (100, 30), (160, 30), (180, 30), (80, 8)] {
                for selected in ["tmt", "tmt-light"] {
                    picker.surface.borrow_mut().select(selected);
                    let mut screen = Terminal::new(TestBackend::new(width, height)).unwrap();
                    screen
                        .draw(|frame| render(frame, &picker, look, frame.area()))
                        .unwrap();
                    let state = picker.surface.borrow();
                    let buffer = screen.backend().buffer();
                    for id in ["tmt", "tmt-light"] {
                        let row = picker_surface::evidence::row(&state, id);
                        if row.height == 0 {
                            continue;
                        }
                        assert_eq!(buffer[(row.x, row.y)].symbol(), "t");
                        assert_eq!(
                            buffer[(row.x + 10, row.y)].symbol(),
                            if id == selected { "›" } else { " " }
                        );
                        assert_eq!(
                            buffer[(row.x + 11, row.y)].symbol(),
                            if id == "tmt" { "s" } else { "t" }
                        );
                        for y in row.y + 1..row.bottom() {
                            assert_ne!(buffer[(row.x + 10, y)].symbol(), "›");
                        }
                    }
                    picker_surface::evidence::capture(
                        &format!("theme-{width}x{height}-{variant}-{selected}"),
                        buffer,
                        &state,
                    );
                }
            }
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
