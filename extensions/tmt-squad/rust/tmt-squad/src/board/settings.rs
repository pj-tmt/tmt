//! A settings snapshot and disposable edit, saved through the opening Config only.
use super::scroll::{Scrolls, Step, WHEEL_LINES};
use crate::{
    config::{Config, Pane},
    look::Look,
    settings::BoardSettings,
};
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind},
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use tmt_cli_style::{Role, grid::Align, mark::Mark, table::escape};
use unicode_width::UnicodeWidthStr;

pub(super) enum Input {
    None,
    Preview,
    Save,
    Close,
}

struct SettingNotice {
    message: String,
    mark: Mark,
}
impl SettingNotice {
    fn error(message: String, key: &str, scope: Option<&str>) -> Self {
        let short = setting_name(key).1;
        let mut message = message;
        if let Some(scope) = scope {
            message = message.replace(&format!("`squad.{scope}.{key}`"), short);
        }
        Self {
            message: message
                .replace(&format!("`{key}`"), short)
                .replace('`', "")
                .replace(
                    "whole s/m/h units, such as \"30m\"",
                    "whole s, m or h, e.g. 30m",
                ),
            mark: Mark::Failed,
        }
    }
    fn text(&self) -> String {
        format!("{} {}", self.mark.symbol(), self.message)
    }
    fn style(&self, look: Look) -> ratatui::style::Style {
        look.role(self.mark.token().role().expect("notice mark has a token"))
    }
}

pub(super) struct Overlay {
    pub settings: BoardSettings,
    scrolls: Scrolls,
    display_path: String,
    config: Option<Config>,
    pub draft: Option<Config>,
    editing: Option<(String, String)>,
    notice: Option<SettingNotice>,
    selected: usize,
    reveal: std::cell::Cell<bool>,
    section: Option<usize>,
    pub squad_keys: Vec<String>,
    pub opening_focus: usize,
    pub staleness: Option<crate::staleness::Snapshot>,
}
impl Overlay {
    pub fn new(mut settings: BoardSettings) -> Self {
        group_entries(&mut settings);
        let display_path = std::env::var("HOME")
            .ok()
            .and_then(|home| {
                settings
                    .path
                    .strip_prefix(&format!("{home}/"))
                    .map(|path| format!("~/{path}"))
            })
            .unwrap_or_else(|| settings.path.clone());
        Self {
            settings,
            display_path,
            scrolls: Scrolls::default(),
            config: None,
            draft: None,
            editing: None,
            notice: None,
            selected: 0,
            reveal: std::cell::Cell::new(false),
            section: None,
            squad_keys: Vec::new(),
            opening_focus: 0,
            staleness: None,
        }
    }
    pub fn open(
        config: Config,
        context: Option<&str>,
        section: Option<usize>,
    ) -> Result<Self, crate::core::SquadError> {
        let mut overlay = Self::new(config.settings(
            context,
            crate::effects::tmux_socket().is_some(),
            section,
        )?);
        overlay.config = Some(config);
        overlay.section = section;
        Ok(overlay)
    }
    pub fn config(&self) -> Option<&Config> {
        self.draft.as_ref().or(self.config.as_ref())
    }
    pub fn editing(&self) -> bool {
        self.editing.is_some()
    }
    fn scope(&self) -> Option<&str> {
        self.settings
            .context
            .as_deref()
            .filter(|key| !crate::tabs::aggregate(key))
    }
    pub fn key(&mut self, key: KeyEvent) -> Input {
        if self.editing.is_none() {
            self.notice = None;
        }
        if let Some((_, text)) = &mut self.editing {
            match key.code {
                KeyCode::Esc => {
                    self.editing = None;
                    self.draft = None;
                    return Input::Preview;
                }
                KeyCode::Enter => {
                    return if self.draft.is_some() {
                        Input::Save
                    } else {
                        Input::Preview
                    };
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => text.clear(),
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && !ch.is_control() =>
                {
                    text.push(ch)
                }
                KeyCode::Backspace => {
                    text.pop();
                }
                _ => return Input::None,
            }
            self.preview();
            return Input::Preview;
        }
        let step = match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char(',') => return Input::Close,
            KeyCode::Enter if self.config.is_some() => {
                if let Some(entry) = self.settings.entries.get(self.selected) {
                    if !entry.editable {
                        self.notice = Some(SettingNotice {
                            message: "This setting is read-only; edit squad.toml.".into(),
                            mark: Mark::Warning,
                        });
                        return Input::None;
                    }
                    self.editing =
                        Some((entry.key.clone(), crate::settings::display(&entry.value)));
                    self.preview();
                }
                return Input::Preview;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                Step::Lines(-1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected =
                    (self.selected + 1).min(self.settings.entries.len().saturating_sub(1));
                Step::Lines(1)
            }
            KeyCode::PageUp => {
                self.selected = self.selected.saturating_sub(10);
                Step::Pages(-1)
            }
            KeyCode::PageDown => {
                self.selected =
                    (self.selected + 10).min(self.settings.entries.len().saturating_sub(1));
                Step::Pages(1)
            }
            KeyCode::Home => {
                self.selected = 0;
                Step::Top
            }
            KeyCode::End => {
                self.selected = self.settings.entries.len().saturating_sub(1);
                Step::Bottom
            }
            _ => return Input::None,
        };
        self.reveal.set(true);
        self.scrolls.scroll(Pane::Rows, step);
        Input::None
    }
    fn preview(&mut self) {
        let (name, text) = self.editing.as_ref().expect("editing value").clone();
        self.draft = None;
        self.notice = None;
        match self
            .config
            .as_ref()
            .unwrap()
            .preview_setting(self.scope(), &name, &text)
        {
            Ok(config) => self.draft = Some(config),
            Err(error) => {
                self.notice = Some(SettingNotice::error(error.message, &name, self.scope()))
            }
        }
    }

    pub fn save(&mut self) -> bool {
        let (name, text) = self.editing.as_ref().unwrap().clone();
        let scope = self.scope().map(str::to_owned);
        match self
            .config
            .as_mut()
            .unwrap()
            .set_setting(scope.as_deref(), &name, &text)
        {
            Ok(_) => {
                self.settings = self
                    .config
                    .as_ref()
                    .unwrap()
                    .settings(
                        self.settings.context.as_deref(),
                        self.settings.host == "tmux",
                        self.section,
                    )
                    .expect("validated settings draft");
                group_entries(&mut self.settings);
                self.editing = None;
                self.draft = None;
                let pinning = crate::settings::saved_notice(
                    self.config.as_ref().unwrap(),
                    scope.as_deref(),
                    &name,
                )
                .expect("validated saved setting");
                self.notice = Some(SettingNotice {
                    mark: if pinning.is_some() {
                        Mark::Warning
                    } else {
                        Mark::Done
                    },
                    message: pinning.unwrap_or_else(|| "Saved to squad.toml".into()),
                });
                true
            }
            Err(error) => {
                self.notice = Some(SettingNotice::error(error.message, &name, scope.as_deref()));
                false
            }
        }
    }
    pub fn mouse(&self, event: MouseEvent) {
        if self.scrolls.pane_at(event.column, event.row).is_some() {
            let direction = match event.kind {
                MouseEventKind::ScrollUp => -1,
                MouseEventKind::ScrollDown => 1,
                _ => return,
            };
            self.scrolls
                .scroll(Pane::Rows, Step::Lines(direction * WHEEL_LINES as isize));
        }
    }
}

// Keep keyboard selection in the same order as the grouped presentation.
fn group_entries(settings: &mut BoardSettings) {
    let mut groups = Vec::new();
    for entry in &settings.entries {
        let group = setting_name(&entry.key).0.to_owned();
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    settings.entries.sort_by_key(|entry| {
        groups
            .iter()
            .position(|group| group == setting_name(&entry.key).0)
            .unwrap()
    });
}

fn setting_name(key: &str) -> (&str, &str) {
    key.split_once('.').unwrap_or(match key {
        "layout" => ("layout", "preset"),
        "rows" => ("rows", "grid"),
        "state_patterns" => ("states", "patterns"),
        _ => ("programs", key),
    })
}

// Serialized JSON punctuation is a break opportunity only outside quoted strings.
fn value_lines(text: &str, width: usize, structured: bool) -> Vec<String> {
    let (mut quoted, mut escaped) = (false, false);
    let mut lines = Vec::new();
    let mut line = String::new();
    for token in text.split_inclusive(|ch| {
        if ch == '"' && !escaped {
            quoted = !quoted;
        }
        let boundary = structured && !quoted && matches!(ch, ',' | ':');
        escaped = quoted && ch == '\\' && !escaped;
        boundary
    }) {
        if !line.is_empty() && line.width() + token.width() > width {
            lines.push(std::mem::take(&mut line));
        }
        if token.width() <= width {
            line.push_str(token);
        } else {
            let mut parts = super::notes::wrap(token, width);
            line = parts.pop().unwrap_or_default();
            lines.extend(parts);
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    if lines.len() > 4 {
        let more = lines.len() - 3;
        lines.truncate(3);
        lines.push(format!("… ({more} more)"));
    }
    lines
}

pub(super) fn render(frame: &mut Frame, overlay: &Overlay, look: Look, body: Rect) {
    if let Some((name, text)) = &overlay.editing {
        let width = body.width.saturating_sub(4);
        let (notice, notice_style) = overlay.notice.as_ref().map_or_else(
            || {
                (
                    format!(
                        "{} valid · Enter save · Esc cancel · Ctrl-U clear",
                        Mark::Done.symbol()
                    ),
                    look.role(Role::Dim),
                )
            },
            |notice| (notice.text(), notice.style(look)),
        );
        let input = tmt_tui::text::fit_line(
            &format!("{text}▏"),
            width.saturating_sub(1),
            tmt_tui::style::TextFlow::Middle,
            Align::Left,
        );
        let mut lines = vec![Line::styled(format!(" {input}"), look.role(Role::Text))];
        lines.extend(
            super::notes::wrap(&notice, usize::from(width.saturating_sub(1).max(1)))
                .into_iter()
                .map(|line| Line::styled(format!(" {line}"), notice_style)),
        );
        let height = (lines.len() as u16 + 2).min(body.height);
        let area = Rect {
            y: body.y + body.height.saturating_sub(height),
            height,
            ..body
        };
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(look.role(Role::Dim))
                    .title(format!(" {name} · preview ")),
            ),
            area,
        );
        return;
    }
    frame.render_widget(Clear, body);
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(look.role(Role::Dim))
        .title(" settings · Enter edit · * read-only ");
    let mut inner = block.inner(body);
    frame.render_widget(block, body);
    inner.x += u16::from(inner.width > 0);
    inner.width = inner.width.saturating_sub(2);
    let mut footer_lines: Vec<Line> = overlay
        .notice
        .as_ref()
        .into_iter()
        .flat_map(|notice| {
            super::notes::wrap(&notice.text(), usize::from(inner.width.max(1)))
                .into_iter()
                .map(|line| Line::styled(line, notice.style(look)))
        })
        .collect();
    footer_lines.push(Line::styled(
        super::view::fit(
            "↑↓ select · Enter edit · PgUp/PgDn page · Esc close",
            usize::from(inner.width),
        ),
        look.role(Role::Muted),
    ));
    let footer_height = (footer_lines.len() as u16).min(inner.height);
    let footer = Rect {
        y: inner.y + inner.height.saturating_sub(footer_height),
        height: footer_height,
        ..inner
    };
    let content = Rect {
        height: inner.height.saturating_sub(footer_height),
        ..inner
    };
    let settings = &overlay.settings;
    let width = usize::from(content.width);
    let row_width = width.saturating_sub(2);
    let key_width = settings
        .entries
        .iter()
        .map(|entry| setting_name(&entry.key).1.width() + usize::from(!entry.editable))
        .max()
        .unwrap_or(0)
        .min(row_width / 4);
    let source_width = (row_width / 3).min(42);
    let value_width = row_width
        .saturating_sub(key_width + source_width + 4)
        .max(1);
    let fit = |text: &str, width: usize| {
        tmt_tui::text::fit_line(
            text,
            width.min(usize::from(u16::MAX)) as u16,
            tmt_tui::style::TextFlow::Middle,
            Align::Left,
        )
    };
    let context = settings.context.as_deref().unwrap_or("board defaults");
    let heading = format!("{context} {} · {}", settings.host, overlay.display_path);
    let mut lines = vec![Line::styled(fit(&heading, width), look.role(Role::Dim))];
    let cell = |text: &str, width, role| Span::styled(fit(text, width), look.role(role));
    lines.push(Line::styled(
        fit(
            "Full values: tmt sq config show --json (--squad/--tab)",
            width,
        ),
        look.role(Role::Dim),
    ));
    for notice in &settings.notices {
        lines.push(Line::styled(fit(notice, width), look.role(Role::Waiting)));
    }
    let mut groups = Vec::new();
    for entry in &settings.entries {
        let group = setting_name(&entry.key).0;
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    let mut selected_range = 0..1;
    for group in groups {
        lines.push(Line::styled(group.to_owned(), look.role(Role::Dim)));
        for entry in settings
            .entries
            .iter()
            .filter(|entry| setting_name(&entry.key).0 == group)
        {
            let selected = settings
                .entries
                .get(overlay.selected)
                .is_some_and(|chosen| chosen.key == entry.key);
            let start = lines.len();
            let name = format!(
                "{}{}",
                setting_name(&entry.key).1,
                if entry.editable { "" } else { "*" }
            );
            let (value, role) = match &entry.value {
                serde_json::Value::Null => ("unset".into(), Role::Dim),
                serde_json::Value::Array(items) if items.is_empty() => ("none".into(), Role::Dim),
                value => (crate::settings::display(value), Role::Text),
            };
            for (index, value) in value_lines(
                &escape(&value),
                value_width,
                entry.value.is_array() || entry.value.is_object(),
            )
            .into_iter()
            .enumerate()
            {
                lines.push(
                    Line::from(vec![
                        Span::raw(if selected { "› " } else { "  " }),
                        cell(
                            if index == 0 { &name } else { "" },
                            key_width,
                            if entry.editable {
                                Role::Accent
                            } else {
                                Role::Muted
                            },
                        ),
                        Span::raw("  "),
                        cell(&value, value_width, role),
                        Span::raw("  "),
                        cell(
                            if index == 0 { &entry.source } else { "" },
                            source_width,
                            Role::Dim,
                        ),
                    ])
                    .style(if selected {
                        look.selection()
                    } else {
                        look.role(Role::Text)
                    }),
                );
            }
            if selected {
                selected_range = start..lines.len();
            }
            if let Some(description) = &entry.description {
                for line in super::notes::wrap(
                    &escape(description),
                    row_width.saturating_sub(key_width + 2).max(1),
                ) {
                    lines.push(Line::from(vec![
                        Span::raw(" ".repeat(key_width + 4)),
                        Span::styled(line, look.role(Role::Muted)),
                    ]));
                }
            }
        }
    }
    let count = lines.len();
    if overlay.reveal.replace(false) {
        overlay
            .scrolls
            .reveal_range(Pane::Rows, selected_range, content, count);
    }
    let (offset, shown) =
        overlay
            .scrolls
            .show(frame, Pane::Rows, content, lines, look.role(Role::Dim));
    if shown < usize::from(content.height) {
        let indicator = Rect {
            y: content.y + shown as u16,
            height: 1,
            ..content
        };
        frame.render_widget(Clear, indicator);
        frame.render_widget(
            Paragraph::new(format!(
                "{}–{} of {count}",
                offset + 1,
                (offset + shown).min(count)
            ))
            .style(look.role(Role::Dim)),
            indicator,
        );
    }
    frame.render_widget(Paragraph::new(footer_lines), footer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyModifiers};
    #[test]
    fn structured_values_break_at_punctuation_preserve_quotes_and_cap_lines() {
        for text in ["x,y:z", r#"x\"y,z:q"#] {
            let value = serde_json::json!({"a":text, "b":1}).to_string();
            let lines = value_lines(&value, 20, true);
            assert_eq!(lines.concat(), value);
            assert!(
                lines[..lines.len() - 1]
                    .iter()
                    .all(|line| line.ends_with([',', ':']))
            );
        }
        let lines = value_lines(
            &serde_json::json!((0..40).collect::<Vec<_>>()).to_string(),
            12,
            true,
        );
        assert_eq!(lines.len(), 4);
        assert!(lines[3].starts_with("… (") && lines[3].ends_with(" more)"));
    }
    #[test]
    fn overlay_scrolls_escaped_values_and_closes_without_row_actions() {
        let mut shown = BoardSettings {
            path: "/isolated/squad.toml".into(),
            context: Some("x".into()),
            host: "plain",
            entries: Vec::new(),
            notices: Vec::new(),
        };
        shown.push("board.mode", serde_json::Value::Null, "preset:team");
        shown.push("board.collapsed", serde_json::json!([]), "preset:team");
        // CJK fixture data exercises wide terminal cells.
        for index in 0..20 {
            shown.push(
                format!("key.{index}"),
                serde_json::json!("long value with wide characters 日本語 and\nnewlines"),
                "preset:team",
            );
        }
        let mut overlay = Overlay::new(shown);
        let look = Look::new(tmt_cli_style::Theme::default());
        for width in [80, 24, 80] {
            let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
            terminal
                .draw(|frame| render(frame, &overlay, look, frame.area()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            if width >= 80 {
                assert!(text.contains("unset") && text.contains("none"));
            }
            assert_eq!(buffer[(1, 1)].symbol(), " ", "content has inset padding");
            assert!(matches!(
                overlay.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                Input::None
            ));
            overlay.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
            terminal
                .draw(|frame| render(frame, &overlay, look, frame.area()))
                .unwrap();
            assert!(overlay.scrolls.offset(Pane::Rows) > 0);
            overlay.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
            assert_eq!(overlay.scrolls.offset(Pane::Rows), 0);
        }
        let mut app = super::super::app::App::new(Some("x".into()));
        app.apply(super::super::app::tests::snapshot(
            "x",
            serde_json::json!([]),
        ));
        app.settings = Some(overlay);
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            super::super::app::Effect::None
        );
        app.apply(super::super::app::tests::snapshot(
            "x",
            serde_json::json!([]),
        ));
        assert!(
            app.settings.is_some(),
            "refresh retains the opening settings snapshot"
        );
        assert_eq!(
            app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            super::super::app::Effect::CancelSettings
        );
        assert!(app.settings.is_none());
    }

    struct Fixture {
        root: std::path::PathBuf,
        path: std::path::PathBuf,
        app: crate::board::app::App,
        original: String,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }
    fn fixture(name: &str, extra: &str) -> Fixture {
        let root =
            std::env::temp_dir().join(format!("tmt-settings-editor-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("squad.toml");
        let original = format!(
            "# preserve comments\nunknown = 'opaque'\n[squad.product]\nlayout = 'crew'\n[squad.product.board]\npanes = ['rows', 'notes']\nsizes = [60, 40]\n{extra}"
        ).replace("{marker}", root.join("unexpected").to_str().unwrap());
        std::fs::write(&path, &original).unwrap();
        let config = Config::read(path.clone()).unwrap();
        let mut app = crate::board::app::App::new(Some("product".into()));
        let mut snapshot = crate::board::app::tests::snapshot(
            "product",
            serde_json::json!([{"rows":[{"name":"before", "state":"working", "fields":{"task":"kept"}}]}]),
        );
        let view = snapshot.view.as_mut().unwrap();
        view.board = config.board("product").unwrap();
        view.rows = config.rows("product").unwrap();
        view.notes = crate::board::app::Notes::Text("# lead notebook".into());
        view.bindings = config.bindings(true, &view.board.panes).unwrap();
        (snapshot.tabs, snapshot.pinned) =
            crate::tabs::arrange(&snapshot.squad_keys, &config.tabs().unwrap());
        app.apply(snapshot);
        app.set_body_width(120);
        app.open_settings(config).unwrap();
        Fixture {
            root,
            path,
            app,
            original,
        }
    }
    fn press(app: &mut crate::board::app::App, code: KeyCode) -> crate::board::app::Effect {
        app.key(KeyEvent::new(code, KeyModifiers::NONE))
    }
    fn edit(app: &mut crate::board::app::App, key: &str, text: &str) {
        let overlay = app.settings.as_mut().unwrap();
        overlay.selected = overlay
            .settings
            .entries
            .iter()
            .position(|entry| entry.key == key)
            .unwrap();
        assert_eq!(press(app, KeyCode::Enter), crate::board::app::Effect::None);
        app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for ch in text.chars() {
            press(app, KeyCode::Char(ch));
        }
    }
    #[test]
    fn each_supported_area_previews_without_writing_and_cancel_restores_it() {
        let mut f = fixture("areas", "");
        let baseline_board = f.app.view.as_ref().unwrap().board.clone();
        let baseline_rows = f.app.view.as_ref().unwrap().rows.value();
        let baseline_tabs = f.app.tabs.clone();
        for (key, text) in [
            ("layout", "pr-queue"),
            ("board.direction", "top-bottom"),
            ("board.sizes", "[30,70]"),
            ("board.panes", "[\"rows\",\"detail\"]"),
            ("board.refresh", "off"),
            ("notes.render", "plain"),
            ("board.hidden_columns", "[\"task\"]"),
            ("states.working.color", "red"),
            ("tabs.order", "[\"product\",\"infra\"]"),
            ("tabs.hide", "[\"infra\"]"),
        ] {
            edit(&mut f.app, key, text);
            assert!(f.app.settings.as_ref().unwrap().draft.is_some(), "{key}");
            let view = f.app.view.as_ref().unwrap();
            match key {
                "layout" => {
                    assert!(view.document["sections"][0]["rows"][0]["colors"]["state"].is_null())
                }
                "board.direction" => assert_ne!(view.board, baseline_board),
                "board.sizes" => assert_ne!(view.board, baseline_board),
                "board.panes" => {
                    assert_eq!(view.board.panes, [Pane::Rows, Pane::Detail]);
                    assert_eq!(f.app.bindings()["d"].text, "toggle detail");
                }
                "board.refresh" => assert!(view.refresh.is_none()),
                "notes.render" => assert_eq!(view.render, crate::config::NotesRender::Plain),
                "board.hidden_columns" => assert_eq!(view.rows.hidden_columns, ["task"]),
                "states.working.color" => assert_eq!(
                    view.document["sections"][0]["rows"][0]["colors"]["state"],
                    "red"
                ),
                "tabs.order" => assert_eq!(f.app.tabs[0], "product"),
                "tabs.hide" => assert!(!f.app.tabs.iter().any(|key| key == "infra")),
                _ => unreachable!(),
            }
            assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
            press(&mut f.app, KeyCode::Esc);
            assert!(f.app.settings.as_ref().unwrap().draft.is_none());
            assert_eq!(f.app.view.as_ref().unwrap().board, baseline_board);
            assert_eq!(f.app.view.as_ref().unwrap().rows.value(), baseline_rows);
            // The original tab policy adds the built-ins to the acquired squad keys.
            assert_eq!(&f.app.tabs[..baseline_tabs.len()], baseline_tabs);
        }
    }
    #[test]
    fn preview_refresh_and_cancel_retain_latest_rows_and_restore_focus() {
        let mut f = fixture("refresh", "");
        f.app.focus = 1;
        edit(&mut f.app, "board.panes", "[\"rows\",\"detail\"]");
        let mut latest = crate::board::app::tests::snapshot(
            "product",
            serde_json::json!([{"rows":[{"name":"newest"}]}]),
        );
        latest.tabs.push("new-squad".into());
        latest.squad_keys.push("new-squad".into());
        f.app.apply(latest);
        assert_eq!(
            f.app.view.as_ref().unwrap().board.panes,
            [Pane::Rows, Pane::Detail]
        );
        assert!(f.app.tabs.iter().any(|key| key == "new-squad"));
        press(&mut f.app, KeyCode::Esc);
        assert_eq!(
            f.app.view.as_ref().unwrap().board.panes,
            [Pane::Rows, Pane::Notes]
        );
        assert_eq!(f.app.focus, 1);
        assert_eq!(
            f.app.view.as_ref().unwrap().document["sections"][0]["rows"][0]["name"],
            "newest"
        );
        assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
        assert_eq!(
            press(&mut f.app, KeyCode::Esc),
            crate::board::app::Effect::CancelSettings
        );
        assert!(f.app.settings.is_none());
    }
    #[test]
    fn confirm_preserves_file_and_sources_then_conflicting_save_refuses() {
        let mut f = fixture("save-notes.render", "");
        edit(&mut f.app, "board.sizes", "[30,70]");
        assert_eq!(
            press(&mut f.app, KeyCode::Enter),
            crate::board::app::Effect::SaveSetting
        );
        assert!(f.app.settings.as_mut().unwrap().save());
        f.app.settings_preview();
        let saved = std::fs::read_to_string(&f.path).unwrap();
        assert!(saved.starts_with("# preserve comments\nunknown = 'opaque'"));
        let persisted = Config::read(f.path.clone())
            .unwrap()
            .settings(Some("product"), false, None)
            .unwrap();
        let sizes = persisted
            .entries
            .iter()
            .find(|entry| entry.key == "board.sizes")
            .unwrap();
        assert_eq!(sizes.value, serde_json::json!([30, 70]));
        assert_eq!(sizes.source, "squad.product.board.sizes");
        assert!(
            f.app
                .settings
                .as_ref()
                .unwrap()
                .notice
                .as_ref()
                .unwrap()
                .message
                .contains("Later preset changes won't override them.")
        );
        edit(&mut f.app, "notes.render", "plain");
        let external = format!("{saved}\n# concurrent edit\n");
        std::fs::write(&f.path, &external).unwrap();
        assert_eq!(
            press(&mut f.app, KeyCode::Enter),
            crate::board::app::Effect::SaveSetting
        );
        assert!(!f.app.settings.as_mut().unwrap().save());
        assert_eq!(std::fs::read_to_string(&f.path).unwrap(), external);
        assert!(
            f.app
                .settings
                .as_ref()
                .unwrap()
                .notice
                .as_ref()
                .unwrap()
                .message
                .contains("changed")
        );
        assert!(
            f.app
                .settings
                .as_ref()
                .unwrap()
                .notice
                .as_ref()
                .unwrap()
                .message
                .contains(f.path.to_str().unwrap()),
            "conflict identifies the actual file even when its path contains the edited key"
        );
        press(&mut f.app, KeyCode::Esc);
        assert_eq!(
            f.app.view.as_ref().unwrap().render,
            crate::config::NotesRender::Markdown
        );
    }
    #[test]
    fn invalid_input_and_configured_commands_never_write_or_dispatch_actions() {
        let mut f = fixture(
            "readonly",
            "[squad.product.fields.probe]\nrun = ['touch', '{marker}']\n[bind]\nenter = 'run touch {marker}'\n",
        );
        for key in ["fields.probe", "bind.enter", "board.layout"] {
            let overlay = f.app.settings.as_mut().unwrap();
            overlay.selected = overlay
                .settings
                .entries
                .iter()
                .position(|entry| entry.key == key)
                .unwrap();
            assert_eq!(
                press(&mut f.app, KeyCode::Enter),
                crate::board::app::Effect::None
            );
            assert!(!f.app.settings.as_ref().unwrap().editing());
            assert!(
                f.app
                    .settings
                    .as_ref()
                    .unwrap()
                    .notice
                    .as_ref()
                    .unwrap()
                    .message
                    .contains("read-only")
            );
        }
        edit(&mut f.app, "board.sizes", "[0,100]");
        assert!(f.app.settings.as_ref().unwrap().draft.is_none());
        assert_eq!(
            press(&mut f.app, KeyCode::Enter),
            crate::board::app::Effect::None
        );
        assert!(f.app.settings.as_ref().unwrap().editing());
        assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
        assert!(!f.root.join("unexpected").exists());
        // Ordinary configured Enter is consumed.
        press(&mut f.app, KeyCode::Esc);
        assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
    }

    #[test]
    fn reminder_preview_cancel_save_and_reload_use_existing_age_evidence() {
        let mut f = fixture(
            "reminders",
            "[squad.product.reminders]\nenabled = true\nstale_after = '30m'\n",
        );
        let mut evidence = serde_json::json!({"state":"fresh", "ageMs":120000, "unchangedSinceMs":10, "activityAfterUpdate":true, "reasons":["pending"]});
        let view = f.app.view.as_mut().unwrap();
        view.document["sections"][0]["rows"][0]["id"] = "member-id".into();
        view.document["sections"][0]["rows"][0]["staleness"] = evidence.clone();
        view.document["squad"]["notesStaleness"] = evidence.clone();
        f.app
            .open_settings(Config::read(f.path.clone()).unwrap())
            .unwrap();
        edit(&mut f.app, "reminders.stale_after", "1m");
        assert_eq!(
            f.app.view.as_ref().unwrap().document["squad"]["notesStaleness"]["state"],
            "stale"
        );
        assert_eq!(
            f.app.view.as_ref().unwrap().document["sections"][0]["rows"][0]["staleness"]["reasons"],
            evidence["reasons"]
        );
        assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
        evidence["ageMs"] = 90000.into();
        let mut latest = crate::board::app::tests::snapshot(
            "product",
            serde_json::json!([{"rows":[{"id":"member-id", "name":"latest", "staleness":evidence}]}]),
        );
        latest.view.as_mut().unwrap().document["squad"]["notesStaleness"] = evidence.clone();
        f.app.apply(latest);
        assert_eq!(
            f.app.view.as_ref().unwrap().document["squad"]["notesStaleness"]["state"],
            "stale"
        );
        assert_eq!(
            f.app.view.as_ref().unwrap().document["sections"][0]["rows"][0]["name"],
            "latest"
        );
        press(&mut f.app, KeyCode::Esc);
        assert_eq!(
            f.app.view.as_ref().unwrap().document["squad"]["notesStaleness"],
            evidence
        );
        edit(&mut f.app, "reminders.enabled", "false");
        assert_eq!(
            f.app.view.as_ref().unwrap().document["sections"][0]["rows"][0]["staleness"]["state"],
            "disabled"
        );
        assert!(
            crate::staleness::label(
                &f.app.view.as_ref().unwrap().document["squad"]["notesStaleness"]
            )
            .is_none()
        );
        press(&mut f.app, KeyCode::Esc);
        assert_eq!(
            f.app.view.as_ref().unwrap().document["squad"]["notesStaleness"],
            evidence
        );
        for invalid in ["59s", "25h", "1.5m", "+1m"] {
            edit(&mut f.app, "reminders.stale_after", invalid);
            assert!(f.app.settings.as_ref().unwrap().draft.is_none());
            assert_eq!(
                press(&mut f.app, KeyCode::Enter),
                crate::board::app::Effect::None
            );
            press(&mut f.app, KeyCode::Esc);
        }
        edit(&mut f.app, "reminders.stale_after", "24h");
        assert_eq!(
            press(&mut f.app, KeyCode::Enter),
            crate::board::app::Effect::SaveSetting
        );
        assert!(f.app.settings.as_mut().unwrap().save());
        let reloaded = Config::read(f.path.clone()).unwrap();
        assert_eq!(
            reloaded.reminders("product").unwrap().stale_after.as_secs(),
            86400
        );
        let saved = std::fs::read_to_string(&f.path).unwrap();
        assert!(saved.contains("unknown = 'opaque'"));
        edit(&mut f.app, "reminders.enabled", "false");
        std::fs::write(&f.path, format!("{saved}\n# other writer\n")).unwrap();
        assert!(!f.app.settings.as_mut().unwrap().save());
        assert_eq!(
            std::fs::read_to_string(&f.path).unwrap(),
            format!("{saved}\n# other writer\n")
        );
    }
    #[test]
    fn aggregate_settings_offer_global_policy_and_consume_edit_keys() {
        let mut f = fixture("aggregate", "");
        f.app.current = Some(crate::tabs::ALL.into());
        f.app.apply(crate::board::app::tests::snapshot(
            crate::tabs::ALL,
            serde_json::json!([]),
        ));
        f.app
            .open_settings(Config::read(f.path.clone()).unwrap())
            .unwrap();
        assert!(
            !f.app
                .settings
                .as_ref()
                .unwrap()
                .settings
                .entries
                .iter()
                .any(|entry| entry.key.starts_with("reminders."))
        );
        edit(&mut f.app, "tabs.hide", "[\"infra\"]");
        assert!(!f.app.tabs.iter().any(|key| key == "infra"));
        assert_eq!(f.app.view.as_ref().unwrap().board.panes, [Pane::Rows]);
        assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
        press(&mut f.app, KeyCode::Esc);
        assert!(f.app.tabs.iter().any(|key| key == "infra"));
    }

    #[test]
    fn clearing_tab_order_previews_core_order_and_cancel_restores_the_saved_policy() {
        let mut f = fixture("tab-order", "[tabs]\norder = ['product','infra']\n");
        assert_eq!(&f.app.tabs[..2], ["product", "infra"]);
        edit(&mut f.app, "tabs.order", "[]");
        assert_eq!(&f.app.tabs[..2], ["infra", "product"]);
        assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
        press(&mut f.app, KeyCode::Esc);
        assert_eq!(&f.app.tabs[..2], ["product", "infra"]);
    }

    #[test]
    fn saved_notice_stays_visible_when_the_settings_list_is_scrolled_and_resized() {
        let mut f = fixture("notice", "");
        let look = Look::new(tmt_cli_style::Theme::default());
        press(&mut f.app, KeyCode::End);
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
        terminal
            .draw(|frame| render(frame, f.app.settings.as_ref().unwrap(), look, frame.area()))
            .unwrap();
        edit(&mut f.app, "board.sizes", "[30,70]");
        assert!(f.app.settings.as_mut().unwrap().save());
        for width in [80, 24, 80] {
            let mut terminal = Terminal::new(TestBackend::new(width, 16)).unwrap();
            terminal
                .draw(|frame| render(frame, f.app.settings.as_ref().unwrap(), look, frame.area()))
                .unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains("Saved layout crew"));
            assert!(
                text.contains("override them."),
                "pinning warning stays complete at width {width}: {text}"
            );
        }
    }
}
