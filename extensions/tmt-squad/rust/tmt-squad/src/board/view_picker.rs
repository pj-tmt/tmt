//! Disposable arrangement preview; only the opening Config can save the draft.
use crate::{
    config::{Board, Config},
    core::SquadError,
    view::{ViewName, ViewScope},
};
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent},
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph},
};
use tmt_cli_style::Role;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Choice {
    Custom,
    Reset,
    View(ViewName),
}
pub(super) enum Input {
    Preview,
    Cancel,
    Save,
}
pub(super) struct Picker {
    config: Config,
    squad: Option<String>,
    pub scope: ViewScope,
    pub selected: Choice,
    current: Choice,
    custom: bool,
    pub notice: Option<String>,
    pub opening: Board,
    pub opening_focus: usize,
    preview: Board,
}
impl Picker {
    pub fn open(
        config: Config,
        squad: Option<String>,
        opening: Board,
        focus: usize,
    ) -> Result<Self, SquadError> {
        let name = squad.as_deref().unwrap_or("");
        let (view, source) = config.view_source(name)?;
        let custom = config.custom_board(name)?;
        let current = if custom {
            Choice::Custom
        } else {
            view.map_or(Choice::Reset, Choice::View)
        };
        let scope = if source == "squad" || custom {
            ViewScope::Squad(squad.clone().expect("squad source"))
        } else {
            ViewScope::Board
        };
        Ok(Self {
            config,
            squad,
            scope,
            selected: current,
            current,
            custom,
            notice: None,
            preview: opening.clone(),
            opening,
            opening_focus: focus,
        })
    }
    fn choices(&self) -> Vec<Choice> {
        self.custom
            .then_some(Choice::Custom)
            .into_iter()
            .chain([Choice::Reset])
            .chain(ViewName::ALL.map(Choice::View))
            .collect()
    }
    pub fn board(&self) -> &Board {
        &self.preview
    }
    pub fn masked(&self) -> Option<String> {
        let name = self.squad.as_deref()?;
        if self.scope == ViewScope::Board && self.config.view_source(name).ok()?.1 == "squad" {
            Some(format!("squad {name} keeps its own view"))
        } else {
            None
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Input {
        self.notice = None;
        let choices = self.choices();
        let at = choices
            .iter()
            .position(|choice| *choice == self.selected)
            .expect("picker entry");
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Input::Cancel,
            KeyCode::Enter => return Input::Save,
            KeyCode::Up | KeyCode::Char('k') => self.selected = choices[at.saturating_sub(1)],
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = choices[(at + 1).min(choices.len() - 1)]
            }
            KeyCode::Tab => {
                if let Some(name) = &self.squad {
                    self.scope = match self.scope {
                        ViewScope::Board => ViewScope::Squad(name.clone()),
                        ViewScope::Squad(_) => ViewScope::Board,
                    };
                }
            }
            _ => return Input::Preview,
        }
        let preview = match self.selected {
            Choice::Custom => {
                self.preview = self.opening.clone();
                return Input::Preview;
            }
            Choice::Reset => {
                self.config
                    .preview_view(&self.scope, None, self.squad.as_deref().unwrap_or(""))
            }
            Choice::View(view) => self.config.preview_view(
                &self.scope,
                Some(view),
                self.squad.as_deref().unwrap_or(""),
            ),
        };
        match preview.and_then(|config| config.board(self.squad.as_deref().unwrap_or(""))) {
            Ok(board) => self.preview = board,
            Err(error) => self.notice = Some(error.message),
        }
        Input::Preview
    }
    pub fn save(&mut self) -> Result<bool, SquadError> {
        let changed = match self.selected {
            Choice::Reset => self.config.remove_view(&self.scope),
            Choice::View(view) if !self.custom => self.config.set_view(&self.scope, view),
            _ => Err(SquadError::hinted(
                "SQUAD_VIEW_CUSTOM",
                "This squad has a custom layout",
                "; ",
                "remove board.layout or panes from squad.toml to use a view",
            )),
        }?;
        self.preview = self.config.board(self.squad.as_deref().unwrap_or(""))?;
        Ok(changed)
    }
    fn inherited(&self) -> Result<String, SquadError> {
        let squad = self.squad.as_deref().unwrap_or("");
        let draft = self.config.preview_view(&self.scope, None, squad)?;
        let (view, source) = draft.view_source(squad)?;
        let name = if source == "custom" {
            "custom layout".into()
        } else if let Some(view) = view {
            view.name().into()
        } else {
            format!("{} layout", draft.layout(squad)?.as_str())
        };
        Ok(format!("inherit the arrangement ({name})"))
    }
    pub fn saved_message(&self, changed: bool) -> String {
        match self.selected {
            Choice::View(view) => format!(
                "{} view {} for {}",
                if changed { "Set" } else { "Kept" },
                view.name(),
                self.scope.label()
            ),
            _ => format!(
                "{} view override for {}",
                if changed { "Removed" } else { "Kept inherited" },
                self.scope.label()
            ),
        }
    }
}

pub(super) fn render(frame: &mut Frame, picker: &Picker, look: crate::look::Look, body: Rect) {
    let width = body.width.min(72);
    let notice = picker
        .notice
        .as_ref()
        .map(|notice| super::notes::wrap(notice, usize::from(width.saturating_sub(4))))
        .unwrap_or_default();
    let height = (picker.choices().len() as u16
        + 8
        + u16::from(picker.masked().is_some())
        + notice.len() as u16)
        .min(body.height);
    let area = Rect {
        x: body.x + (body.width - width) / 2,
        y: body.y + (body.height - height) / 2,
        width,
        height,
    };
    let active = look.role(Role::Accent).add_modifier(Modifier::BOLD);
    let muted = look.role(Role::Muted);
    let mut scope = vec![Span::styled(
        "all boards",
        if picker.scope == ViewScope::Board {
            active
        } else {
            muted
        },
    )];
    if picker.squad.is_some() {
        scope.extend([
            Span::styled(" · ", muted),
            Span::styled(
                "this squad",
                if picker.scope != ViewScope::Board {
                    active
                } else {
                    muted
                },
            ),
        ]);
    }
    let mut lines = vec![Line::from(scope), Line::default()];
    let inherited = picker
        .inherited()
        .unwrap_or_else(|_| "inherit the arrangement".into());
    for choice in picker.choices() {
        let (name, purpose) = match choice {
            Choice::Custom => ("custom", "(squad.toml)"),
            Choice::Reset => ("default", inherited.as_str()),
            Choice::View(view) => (view.name(), view.description()),
        };
        let prefix = format!(
            "{} {} {name:<10} ",
            if choice == picker.selected {
                "›"
            } else {
                " "
            },
            if choice == picker.current { "●" } else { " " }
        );
        let style = if choice == picker.selected {
            look.selection()
        } else {
            muted
        };
        let mut line = Line::from(vec![
            Span::raw(prefix),
            Span::styled(
                super::view::fit(purpose, width.saturating_sub(19) as usize),
                if choice == Choice::Reset {
                    look.role(Role::Dim)
                } else {
                    style
                },
            ),
        ]);
        line.style = style;
        lines.push(line);
    }
    lines.push(Line::default());
    if let Some(masked) = picker.masked() {
        lines.push(Line::styled(masked, muted));
    }
    lines.push(Line::styled(
        "Pane arrangement and fold defaults only",
        muted,
    ));
    for line in notice {
        lines.push(Line::styled(line, look.role(Role::Waiting)));
    }
    lines.push(Line::styled("Enter save · Esc cancel · Tab scope", muted));
    let title = match &picker.scope {
        ViewScope::Board => " view · all boards ".into(),
        ViewScope::Squad(name) => format!(" view · {name} "),
    };
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::new()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .title(title),
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
    fn fixture(name: &str, text: &str) -> (std::path::PathBuf, Config, App) {
        let root =
            std::env::temp_dir().join(format!("tmt-view-picker-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("squad.toml");
        std::fs::write(&path, text).unwrap();
        let config = Config::read(path.clone()).unwrap();
        let mut app = App::new(Some("product".into()));
        let mut snapshot =
            crate::board::app::tests::snapshot("product", json!([{"rows":[{"name":"before"}]}]));
        let view = snapshot.view.as_mut().unwrap();
        view.board = config.board("product").unwrap();
        view.bindings = config.bindings(true, &view.board.panes).unwrap();
        app.apply(snapshot);
        app.set_body_width(120);
        (path, config, app)
    }
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    #[test]
    fn preview_refresh_and_cancel_keep_runtime_folds_opening_geometry_and_latest_data() {
        let original = "# intact\n[board]\nview = 'team'\n";
        let (path, config, mut app) = fixture("cancel", original);
        app.key(key(KeyCode::Char('d')));
        let opening = app.effective_board().unwrap().clone();
        let folds = app.folds_at(120);
        assert_eq!(app.key(key(KeyCode::Char('l'))), Effect::PickView);
        app.open_view_picker(config).unwrap();
        app.key(key(KeyCode::Down)); // focus
        app.key(key(KeyCode::Down)); // notes
        assert_ne!(app.effective_board().unwrap(), &opening);
        assert!(app.folds_at(120).contains(&crate::config::Pane::Detail));
        let mut snapshot =
            crate::board::app::tests::snapshot("product", json!([{"rows":[{"name":"after"}]}]));
        snapshot.view.as_mut().unwrap().board = crate::config::Board::simple(
            crate::config::BoardMode::Split,
            crate::config::Direction::TopBottom,
            vec![crate::config::Pane::Rows],
            &[100],
        );
        app.apply(snapshot);
        assert_eq!(
            app.view.as_ref().unwrap().document["sections"][0]["rows"][0]["name"],
            "after"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(app.key(key(KeyCode::Esc)), Effect::CancelView);
        assert_eq!(app.effective_board().unwrap(), &opening);
        assert_eq!(app.folds_at(120), folds);
        assert_eq!(
            app.view.as_ref().unwrap().document["sections"][0]["rows"][0]["name"],
            "after"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn scope_reset_and_stale_save_use_the_opening_config_only() {
        let original = "# preserve\n[board]\nview = 'notes'\n[squad.product.board]\nview = 'focus'\nrefresh = 'off' # preserve\n";
        let (path, config, mut app) = fixture("scope", original);
        app.open_view_picker(config).unwrap();
        let picker = app.view_picker.as_mut().unwrap();
        assert_eq!(picker.scope, ViewScope::Squad("product".into()));
        assert_eq!(
            picker.inherited().unwrap(),
            "inherit the arrangement (notes)"
        );
        picker.key(key(KeyCode::Tab));
        assert_eq!(
            picker.masked().as_deref(),
            Some("squad product keeps its own view")
        );
        picker.key(key(KeyCode::Tab));
        picker.selected = Choice::Reset;
        assert!(picker.save().unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original.replace("view = 'focus'\n", "")
        );
        app.close_view_picker(true);
        let config = Config::read(path.clone()).unwrap();
        app.open_view_picker(config).unwrap();
        app.key(key(KeyCode::Down));
        let newer = "# concurrent edit\n[board]\nview = 'wide'\n";
        std::fs::write(&path, newer).unwrap();
        app.apply(crate::board::app::tests::snapshot("product", json!([])));
        let error = app.view_picker.as_mut().unwrap().save().unwrap_err();
        assert_eq!(error.code, "SQUAD_CONFIG_CHANGED");
        assert!(app.view_picker.is_some());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), newer);
        app.key(key(KeyCode::Esc));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), newer);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn custom_can_preview_but_only_reset_can_write_and_keep_custom_bytes() {
        let original = "[squad.product.board]\npanes = ['rows', 'notes'] # own\nview = 'focus'\n";
        let (path, config, mut app) = fixture("custom", original);
        app.open_view_picker(config).unwrap();
        app.key(key(KeyCode::Down)); // reset
        app.key(key(KeyCode::Down)); // team
        assert_eq!(app.effective_board().unwrap().panes.len(), 4);
        let picker = app.view_picker.as_mut().unwrap();
        assert_eq!(picker.save().unwrap_err().code, "SQUAD_VIEW_CUSTOM");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        picker.scope = ViewScope::Squad("product".into());
        picker.selected = Choice::Reset;
        assert!(picker.save().unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original.replace("view = 'focus'\n", "")
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn overlay_isolates_keys_mouse_and_builtin_scope_and_renders_each_depth() {
        let (path, config, mut app) = fixture("isolation", "");
        app.open_view_picker(config.clone()).unwrap();
        for code in [
            KeyCode::Left,
            KeyCode::Char('d'),
            KeyCode::Char('T'),
            KeyCode::Char('/'),
            KeyCode::Char('?'),
        ] {
            assert_eq!(app.key(key(code)), Effect::None);
        }
        assert_eq!(
            app.mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: 1,
                    row: 0,
                    modifiers: KeyModifiers::NONE
                },
                std::time::Instant::now()
            ),
            Effect::None
        );
        assert!(!app.help && !app.searching && app.theme_picker.is_none());
        app.key(key(KeyCode::Esc));
        app.current = Some(crate::board::ALL.into());
        app.open_view_picker(config).unwrap();
        app.key(key(KeyCode::Tab));
        assert_eq!(app.view_picker.as_ref().unwrap().scope, ViewScope::Board);
        for depth in [
            tmt_cli_style::Depth::TrueColor,
            tmt_cli_style::Depth::Ansi16,
            tmt_cli_style::Depth::None,
        ] {
            let mut look = app.look();
            look.depth = depth;
            let mut terminal = Terminal::new(TestBackend::new(90, 22)).unwrap();
            terminal
                .draw(|frame| render(frame, app.view_picker.as_ref().unwrap(), look, frame.area()))
                .unwrap();
            let content: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            for text in [
                "default",
                "inherit the arrangement (team)",
                "view · all boards",
                "all boards",
                "Enter save · Esc cancel · Tab scope",
            ] {
                assert!(content.contains(text), "missing {text}");
            }
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
