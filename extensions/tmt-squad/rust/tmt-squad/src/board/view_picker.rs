//! Disposable arrangement preview; only the opening Config can save the draft.
use super::picker_surface;
use crate::{
    config::{Board, Config},
    core::SquadError,
    view::{ViewName, ViewScope},
};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode},
    layout::Rect,
};
use serde_json::json;
use std::{cell::RefCell, sync::OnceLock};
use tmt_tui::components::{ListRow, PickerEvent, PickerField, PickerInput, surface};

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
impl Choice {
    fn id(self) -> &'static str {
        match self {
            Self::Custom => "custom",
            Self::Reset => "default",
            Self::View(view) => view.name(),
        }
    }
}
const FILE: &str = "squad.view-picker.xml";
const MARKUP: &str = r#"<tmt-view version="1"><tmt-modal id="view-picker" title="view · all boards" placement="center" class="w-72"><tmt-scroll id="choices-body"><tmt-table id="choices" bind="$.rows"><tmt-row class="grid grid-cols-[2_10_1fr] gap-1"><tmt-cell bind="row.mark" token="text"/><tmt-cell bind="row.name" token="text"/><tmt-cell bind="row.description" wrap="true" token="text"/></tmt-row></tmt-table><tmt-repeat each="$.notes" as="note"><tmt-text bind="note.text" token="muted" wrap="true"/></tmt-repeat></tmt-scroll><tmt-text id="scope" slot="query" bind="$.query" token="accent"/><tmt-text slot="status" bind="$.status" token="blocked"/><tmt-text slot="footer" bind="$.footer" token="muted"/></tmt-modal></tmt-view>"#;
fn template(squad: bool) -> &'static surface::Template<()> {
    static BOARD: OnceLock<surface::Template<()>> = OnceLock::new();
    static SQUAD: OnceLock<surface::Template<()>> = OnceLock::new();
    let template = if squad { &SQUAD } else { &BOARD };
    template.get_or_init(|| {
        picker_surface::compile(
            FILE,
            &if squad {
                MARKUP.replace("view · all boards", "view · this squad")
            } else {
                MARKUP.into()
            },
            picker_surface::schema(&["mark", "name", "description"]),
        )
    })
}

pub(super) struct Picker {
    config: Config,
    squad: Option<String>,
    pub scope: ViewScope,
    pub(super) surface: RefCell<picker_surface::State>,
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
        let (view, _) = config.view_source(name)?;
        let custom = config.custom_board(name)?;
        let current = if custom {
            Choice::Custom
        } else {
            view.map_or(Choice::Reset, Choice::View)
        };
        let scope = ViewScope::Board;
        Ok(Self {
            config,
            squad,
            scope,
            surface: RefCell::new(picker_surface::State::new(
                None,
                (if custom {
                    vec![Choice::Custom, Choice::Reset]
                } else {
                    vec![Choice::Reset]
                })
                .into_iter()
                .chain(ViewName::ALL.into_iter().map(Choice::View))
                .map(|choice| ListRow {
                    id: choice.id().into(),
                    disabled: false,
                })
                .collect(),
                Some(current.id()),
            )),
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
        if self.scope == ViewScope::Board && self.custom {
            Some(format!("squad {name} keeps its custom layout"))
        } else if self.scope == ViewScope::Board && self.config.view_source(name).ok()?.1 == "squad"
        {
            Some(format!("squad {name} keeps its own view · r reset in ,"))
        } else {
            None
        }
    }
    pub fn selected(&self) -> Choice {
        let surface = self.surface.borrow();
        let id = surface.picker.list.selected().expect("view choice");
        self.choices()
            .into_iter()
            .find(|choice| choice.id() == id)
            .expect("built-in choice")
    }
    #[cfg(test)]
    pub fn key(&mut self, key: ratatui::crossterm::event::KeyEvent) -> Input {
        self.input(&Event::Key(key)).unwrap_or(Input::Preview)
    }
    pub fn input(&mut self, event: &Event) -> Option<Input> {
        if matches!(event, Event::Key(_)) {
            self.notice = None;
        }
        if matches!(event, Event::Key(key) if key.code == KeyCode::Tab) {
            if let Some(name) = &self.squad {
                self.scope = match self.scope {
                    ViewScope::Board => ViewScope::Squad(name.clone()),
                    ViewScope::Squad(_) => ViewScope::Board,
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
        if self.squad.is_none() || (self.custom && self.scope == ViewScope::Board) {
            self.preview = self.opening.clone();
            return Some(Input::Preview);
        }
        let preview = match self.selected() {
            Choice::Custom => {
                self.preview = self.opening.clone();
                return Some(Input::Preview);
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
        Some(Input::Preview)
    }
    pub fn save(&mut self) -> Result<bool, SquadError> {
        let changed = match self.selected() {
            Choice::Reset => self.config.remove_view(&self.scope),
            Choice::View(view) if !self.custom || self.scope == ViewScope::Board => {
                self.config.set_view(&self.scope, view)
            }
            _ => Err(SquadError::hinted(
                "SQUAD_VIEW_CUSTOM",
                "This squad has a custom layout",
                "; ",
                "remove board.layout or panes from ops.toml to use a view",
            )),
        }?;
        if let Some(squad) = &self.squad {
            self.preview = self.config.board(squad)?;
        }
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
        match self.selected() {
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
    let inherited = picker
        .inherited()
        .unwrap_or_else(|_| "inherit the arrangement".into());
    let selected = picker
        .surface
        .borrow()
        .picker
        .list
        .selected()
        .map(str::to_owned);
    let rows: Vec<_> = picker.choices().into_iter().map(|choice| {
        let (name, description) = match choice {
            Choice::Custom => ("custom", "(ops.toml)"),
            Choice::Reset => ("default", inherited.as_str()),
            Choice::View(view) => (view.name(), view.description()),
        };
        json!({"id":choice.id(),"disabled":false,"mark":match (choice == picker.current, selected.as_deref() == Some(choice.id())) { (true, true) => "●›", (true, false) => "●", (false, true) => " ›", (false, false) => "" },"name":name,"description":description})
    }).collect();
    let mut notes = vec![json!({"id":"purpose", "text":"Pane arrangement and fold defaults only"})];
    if let Some(masked) = picker.masked() {
        notes.insert(0, json!({"id":"masked", "text":masked}));
    }
    if picker.squad.is_none() {
        notes.push(json!({"id":"aggregate", "text":"leads and all stay rows only; views apply to squad tabs"}));
    }
    let query = if picker.squad.is_some() {
        if picker.scope == ViewScope::Board {
            "all boards · Tab: this squad"
        } else {
            "this squad · Tab: all boards"
        }
    } else {
        "all boards"
    };
    let value = json!({"rows":rows,"notes":notes,"query":query,"status":picker.notice.as_deref().unwrap_or(""),"footer":"Enter save · Esc cancel · Tab scope"});
    picker.surface.borrow_mut().render(
        FILE,
        template(picker.scope != ViewScope::Board),
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
    use ratatui::{
        Terminal,
        backend::TestBackend,
        crossterm::event::{KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    };
    use serde_json::json;
    fn fixture(name: &str, text: &str) -> (std::path::PathBuf, Config, App) {
        let root =
            std::env::temp_dir().join(format!("tmt-view-picker-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("ops.toml");
        std::fs::write(&path, text).unwrap();
        let config = Config::read(path.clone()).unwrap();
        let mut app = App::new(Some("product".into()));
        let mut snapshot =
            crate::board::app::tests::snapshot("product", json!([{"rows":[{"name":"before"}]}]));
        let view = snapshot.view.as_mut().unwrap();
        view.board = config.board("product").unwrap();
        view.bindings =
            crate::action::with_action_keys(config.bindings(true, &view.board.panes).unwrap());
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
        assert_eq!(picker.scope, ViewScope::Board);
        assert_eq!(
            picker.masked().as_deref(),
            Some("squad product keeps its own view · r reset in ,")
        );
        picker.key(key(KeyCode::Tab));
        assert_eq!(picker.scope, ViewScope::Squad("product".into()));
        assert_eq!(
            picker.inherited().unwrap(),
            "inherit the arrangement (notes)"
        );
        picker.key(key(KeyCode::Tab));
        assert_eq!(
            picker.masked().as_deref(),
            Some("squad product keeps its own view · r reset in ,")
        );
        picker.key(key(KeyCode::Tab));
        picker.surface.borrow_mut().select(Choice::Reset.id());
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
    fn custom_previews_and_refuses_scoped_save_but_masks_and_saves_all_boards() {
        let original = "[squad.product.board]\npanes = ['rows', 'notes'] # own\nview = 'focus'\n";
        let (path, config, mut app) = fixture("custom", original);
        app.open_view_picker(config).unwrap();
        assert_eq!(app.view_picker.as_ref().unwrap().scope, ViewScope::Board);
        app.key(key(KeyCode::Tab));
        app.key(key(KeyCode::Down)); // reset
        app.key(key(KeyCode::Down)); // members
        app.key(key(KeyCode::Down)); // team
        assert_eq!(app.effective_board().unwrap().panes.len(), 4);
        let picker = app.view_picker.as_mut().unwrap();
        assert_eq!(picker.save().unwrap_err().code, "SQUAD_VIEW_CUSTOM");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        picker.key(key(KeyCode::Tab));
        assert_eq!(picker.scope, ViewScope::Board);
        assert_eq!(
            picker.masked().as_deref(),
            Some("squad product keeps its custom layout")
        );
        assert_eq!(picker.board(), &picker.opening);
        picker.key(key(KeyCode::Down)); // focus
        assert_eq!(
            picker.board(),
            &picker.opening,
            "global preview keeps the custom arrangement"
        );
        assert!(picker.save().unwrap());
        let global = std::fs::read_to_string(&path).unwrap();
        assert_eq!(global, format!("{original}\n[board]\nview = \"focus\"\n"));
        assert_eq!(
            picker.board(),
            &picker.opening,
            "global save keeps the custom arrangement"
        );
        assert_eq!(
            Config::read(path.clone())
                .unwrap()
                .view_source("other")
                .unwrap()
                .0,
            Some(ViewName::Focus)
        );
        picker.key(key(KeyCode::Tab));
        assert_eq!(picker.scope, ViewScope::Squad("product".into()));
        assert_eq!(picker.board().panes.len(), 4);
        assert_eq!(picker.save().unwrap_err().code, "SQUAD_VIEW_CUSTOM");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), global);
        picker.surface.borrow_mut().select(Choice::Reset.id());
        assert!(picker.save().unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            global.replace("view = 'focus'\n", "")
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn aggregate_boards_keep_their_geometry_through_preview_save_and_cancel() {
        for tab in [crate::board::ALL, crate::board::LEADS] {
            let (path, config, mut app) = fixture(tab, "# preserve\n");
            let board = Board::simple(
                crate::config::BoardMode::Split,
                crate::config::Direction::TopBottom,
                vec![crate::config::Pane::Rows],
                &[100],
            );
            app.current = Some(tab.into());
            let mut snapshot = crate::board::app::tests::snapshot(tab, json!([]));
            snapshot.view.as_mut().unwrap().board = board.clone();
            app.apply(snapshot);
            app.open_view_picker(config).unwrap();
            app.key(key(KeyCode::Down)); // team
            app.key(key(KeyCode::Down)); // focus
            app.key(key(KeyCode::Down)); // notes
            assert_eq!(app.effective_board(), Some(&board));
            assert!(app.view_picker.as_mut().unwrap().save().unwrap());
            app.close_view_picker(true);
            assert_eq!(app.effective_board(), Some(&board));
            let config = Config::read(path.clone()).unwrap();
            assert_eq!(
                config.view_source("product").unwrap().0,
                Some(ViewName::Notes)
            );
            let saved = std::fs::read(&path).unwrap();
            app.open_view_picker(config).unwrap();
            app.key(key(KeyCode::Down)); // detail
            assert_eq!(app.effective_board(), Some(&board));
            assert_eq!(app.key(key(KeyCode::Esc)), Effect::CancelView);
            assert_eq!(app.effective_board(), Some(&board));
            assert_eq!(std::fs::read(&path).unwrap(), saved);
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
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
                "inherit the arrangement (members)",
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
    #[test]
    fn saved_dot_and_cursor_move_independently_in_the_existing_mark_track() {
        let original = "[board]\nview = 'team'\n";
        let (path, config, app) = fixture("cursor-track", original);
        let mut picker = Picker::open(
            config,
            Some("product".into()),
            app.effective_board().unwrap().clone(),
            0,
        )
        .unwrap();
        for (variant, look) in picker_surface::evidence::looks().into_iter().enumerate() {
            for (width, height) in [(80, 30), (100, 30), (160, 30), (180, 30), (80, 8)] {
                for choice in [Choice::View(ViewName::Team), Choice::Reset] {
                    picker.surface.borrow_mut().select(choice.id());
                    let mut screen = Terminal::new(TestBackend::new(width, height)).unwrap();
                    screen
                        .draw(|frame| render(frame, &picker, look, frame.area()))
                        .unwrap();
                    let state = picker.surface.borrow();
                    let buffer = screen.backend().buffer();
                    for id in ["team", "default"] {
                        let row = picker_surface::evidence::row(&state, id);
                        if row.height == 0 {
                            continue;
                        }
                        assert_eq!(
                            buffer[(row.x, row.y)].symbol(),
                            if id == "team" { "●" } else { " " }
                        );
                        assert_eq!(
                            buffer[(row.x + 1, row.y)].symbol(),
                            if id == choice.id() { "›" } else { " " }
                        );
                        assert_eq!(
                            buffer[(row.x + 3, row.y)].symbol(),
                            if id == "team" { "t" } else { "d" }
                        );
                    }
                    picker_surface::evidence::capture(
                        &format!("view-{width}x{height}-{variant}-{}", choice.id()),
                        buffer,
                        &state,
                    );
                }
            }
        }
        picker.key(key(KeyCode::Down));
        assert_eq!(picker.current.id(), "team");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
