//! Disposable theme preview. The opening Config is the save baseline;
//! refresh snapshots update the board, never this unsaved draft.

use crate::{config::Config, core::SquadError, look::Look, theme::ThemeScope};
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph},
};
use tmt_cli_style::{Base, Role};

pub(super) struct Picker {
    config: Config,
    squad: Option<String>,
    pub scope: ThemeScope,
    pub selected: Base,
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
        let scope = if config.theme_source(name)? == "squad" {
            ThemeScope::Squad(squad.clone().expect("a squad theme has a squad"))
        } else {
            ThemeScope::Board
        };
        let preview = config.preview_theme_base(&scope, selected, name)?;
        Ok(Self {
            config,
            squad,
            scope,
            selected,
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
            "squad {name} keeps {} (its own setting)",
            self.config.theme(name).ok()?.0.base.name()
        ))
    }

    pub fn key(&mut self, key: KeyEvent) -> Input {
        self.notice = None;
        let previous = (self.scope.clone(), self.selected);
        let position = Base::ALL
            .iter()
            .position(|base| *base == self.selected)
            .expect("built-in base");
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Input::Cancel,
            KeyCode::Enter => return Input::Save,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = Base::ALL[position.saturating_sub(1)]
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = Base::ALL[(position + 1).min(Base::ALL.len() - 1)]
            }
            KeyCode::Tab => {
                if let Some(name) = &self.squad {
                    self.scope = match self.scope {
                        ThemeScope::Board => ThemeScope::Squad(name.clone()),
                        ThemeScope::Squad(_) => ThemeScope::Board,
                    };
                }
            }
            _ => {}
        }
        if previous != (self.scope.clone(), self.selected) {
            self.preview = self
                .config
                .preview_theme_base(
                    &self.scope,
                    self.selected,
                    self.squad.as_deref().unwrap_or(""),
                )
                .expect("opening validates the layers and built-in bases are valid");
        }
        Input::Preview
    }

    pub fn save(&mut self) -> Result<bool, SquadError> {
        self.config.set_theme_base(&self.scope, self.selected)
    }

    pub fn saved_message(&self, changed: bool) -> String {
        format!(
            "{} theme {} for {} (board only)",
            if changed { "Set" } else { "Kept" },
            self.selected.name(),
            self.scope.label()
        )
    }
}

pub(super) fn render(frame: &mut Frame, picker: &Picker, look: Look, body: Rect) {
    let width = body.width.min(72);
    let height = (Base::ALL.len() as u16
        + 8
        + u16::from(picker.masked().is_some())
        + u16::from(picker.notice.is_some()))
    .min(body.height);
    let area = Rect {
        x: body.x + (body.width - width) / 2,
        y: body.y + (body.height - height) / 2,
        width,
        height,
    };
    let active = look.role(Role::Accent).add_modifier(Modifier::BOLD);
    let inactive = look.role(Role::Muted);
    let mut scope = vec![Span::styled(
        "all boards",
        if picker.scope == ThemeScope::Board {
            active
        } else {
            inactive
        },
    )];
    if picker.squad.is_some() {
        scope.extend([
            Span::styled(" · ", inactive),
            Span::styled(
                "this squad",
                if picker.scope != ThemeScope::Board {
                    active
                } else {
                    inactive
                },
            ),
        ]);
    }
    let mut lines = vec![Line::from(scope), Line::default()];
    for base in Base::ALL {
        let text = format!("{:<10} {}", base.name(), base.description());
        let text = super::view::fit(&text, usize::from(width.saturating_sub(4)));
        let style = if base == picker.selected {
            look.selection()
        } else {
            look.role(Role::Muted)
        };
        lines.push(Line::styled(text, style));
    }
    lines.push(Line::default());
    if let Some(masked) = picker.masked() {
        lines.push(Line::styled(masked, inactive));
    }
    lines.push(Line::styled(
        "Board only; CLI colors stay unchanged",
        inactive,
    ));
    if let Some(notice) = &picker.notice {
        lines.push(Line::styled(notice.as_str(), look.role(Role::Waiting)));
    }
    lines.push(Line::styled(
        "Enter save · Esc cancel · Tab scope",
        inactive,
    ));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::new()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .title(" theme "),
        ),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::app::{App, Effect};
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    };
    use serde_json::json;

    fn fixture(name: &str, text: &str) -> (std::path::PathBuf, Config) {
        let directory =
            std::env::temp_dir().join(format!("tmt-picker-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("squad.toml");
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
        assert_eq!(picker.selected, Base::Auto);
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
            Some("squad product keeps mono (its own setting)")
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
        assert_eq!(picker.selected, Base::TmtLight);
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
                buffer[(find(0, "product"), 0)].fg,
                expected,
                "tab uses the preview, including retained tokens"
            );
            assert_eq!(
                buffer[(find(1, " · 1 waiting"), 1)].fg,
                expected,
                "summary uses the preview, including retained tokens"
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
        let mut picker = Picker::open(config, Some("product".into())).unwrap();
        picker.key(key(KeyCode::Tab));
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
}
