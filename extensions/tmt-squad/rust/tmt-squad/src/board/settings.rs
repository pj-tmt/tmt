//! A settings snapshot and disposable edit, saved through the opening Config only.
use super::picker_surface;
use crate::{config::Config, look::Look, settings::BoardSettings};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent},
    layout::Rect,
};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap};
use tmt_cli_style::{Role, grid::Align, mark::Mark, table::escape};
use tmt_tui::{
    binding::Schema,
    components::{ListRow, PickerField},
};
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
    fn display(&self, width: u16, path: &str) -> String {
        // Keep the controller's complete diagnostic; only its terminal projection
        // abbreviates the known config path and budgets its leading word.
        let file = std::path::Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("squad.toml");
        let message = self.message.replace(path, file);
        let split = message.find(char::is_whitespace).unwrap_or(message.len());
        let first = &message[..split];
        // Leave the following word boundary inside the shared wrap budget, too.
        let budget = width.saturating_sub(self.mark.symbol().width() as u16 + 2);
        if escape(first).width() <= usize::from(budget) {
            return format!("{} {message}", self.mark.symbol());
        }
        let first =
            tmt_tui::text::fit_line(first, budget, tmt_tui::style::TextFlow::Middle, Align::Left);
        format!("{} {first}{}", self.mark.symbol(), &message[split..])
    }
}

struct CachedTemplate<K> {
    key: K,
    template: tmt_tui::components::surface::Template<()>,
}

#[derive(PartialEq, Eq)]
struct ReferenceStyle {
    key_width: usize,
    source_width: usize,
    notice_role: &'static str,
}

#[derive(PartialEq, Eq)]
struct PromptStyle {
    width: u16,
    height: usize,
    notice_role: &'static str,
}

pub(super) struct Overlay {
    pub settings: BoardSettings,
    pub surface: RefCell<picker_surface::State>,
    prompt: RefCell<picker_surface::State>,
    reference_template: RefCell<Option<CachedTemplate<ReferenceStyle>>>,
    prompt_template: RefCell<Option<CachedTemplate<PromptStyle>>>,
    display_path: String,
    config: Option<Config>,
    pub draft: Option<Config>,
    editing: Option<(String, String)>,
    notice: Option<SettingNotice>,
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
        let rows = list_rows(&settings);
        Self {
            settings,
            reference_template: RefCell::new(None),
            prompt_template: RefCell::new(None),
            display_path,
            surface: RefCell::new(picker_surface::State::new(None, rows, None)),
            prompt: RefCell::new(picker_surface::State::new(None, Vec::new(), None)),
            config: None,
            draft: None,
            editing: None,
            notice: None,
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
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char(',') => return Input::Close,
            KeyCode::Enter if self.config.is_some() => {
                if let Some(entry) = self.selected_entry() {
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
            _ => {}
        }
        self.surface
            .borrow_mut()
            .input(&Event::Key(key), PickerField::List);
        Input::None
    }
    fn selected_entry(&self) -> Option<&crate::settings::BoardSetting> {
        let id = self
            .surface
            .borrow()
            .picker
            .list
            .selected()
            .map(str::to_owned)?;
        self.settings.entries.iter().find(|entry| entry.key == id)
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
                self.surface
                    .borrow_mut()
                    .reconcile(list_rows(&self.settings));
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
        if self.editing.is_none() {
            self.surface
                .borrow_mut()
                .input(&Event::Mouse(event), PickerField::List);
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

fn list_rows(settings: &BoardSettings) -> Vec<ListRow> {
    let mut rows = Vec::new();
    let mut previous = None;
    for entry in &settings.entries {
        let group = setting_name(&entry.key).0;
        if previous != Some(group) {
            rows.push(ListRow {
                id: format!("group:{group}"),
                disabled: true,
            });
            previous = Some(group);
        }
        rows.push(ListRow {
            id: entry.key.clone(),
            disabled: false,
        });
    }
    rows
}

const FILE: &str = "squad.settings.xml";
fn template(
    key_width: usize,
    source_width: usize,
    role: &str,
) -> tmt_tui::components::surface::Template<()> {
    // Groups are disabled rows; setting identity and selection remain owned by the list.
    let markup = format!(
        r#"<tmt-view version="1"><tmt-modal id="settings" title="settings · Enter edit · * read-only" placement="body"><tmt-scroll id="body"><tmt-text bind="$.query" token="dim" class="truncate-middle"/><tmt-text token="dim" class="truncate">Full values: tmt sq config show --json (--squad/--tab)</tmt-text><tmt-repeat each="$.notes" as="note"><tmt-text bind="note.text" token="waiting" wrap="true"/></tmt-repeat><tmt-list id="choices" bind="$.rows" empty="(no settings)"><tmt-row class="flex-col"><tmt-repeat each="row.lines" as="line"><tmt-row id-bind="line.id" class="grid grid-cols-[2_{key_width}_1fr_{source_width}] gap-x-2 shrink-0"><tmt-text bind="line.mark"/><tmt-text id="name" bind="line.name" token="accent" class="truncate-middle"/><tmt-text id="value" bind="line.value" token="text" wrap="true"/><tmt-text bind="line.source" token="dim" class="truncate-middle"/></tmt-row></tmt-repeat><tmt-repeat each="row.description" as="description"><tmt-row class="grid grid-cols-[2_{key_width}_1fr_{source_width}] gap-x-2 shrink-0"><tmt-text/><tmt-text/><tmt-text bind="description.text" token="muted" wrap="true" class="col-span-2"/></tmt-row></tmt-repeat><tmt-repeat each="row.heading" as="heading"><tmt-text bind="heading.text" token="dim" class="shrink-0"/></tmt-repeat></tmt-row></tmt-list></tmt-scroll><tmt-text slot="status" bind="$.status" token="{role}" wrap="true"/><tmt-text slot="footer" bind="$.footer" token="muted"/></tmt-modal></tmt-view>"#
    );
    let scalar_row = |fields: &[&str]| {
        Schema::Collection(Box::new(Schema::Object(
            fields
                .iter()
                .map(|name| {
                    (
                        name.to_string(),
                        if *name == "id" {
                            Schema::StableId
                        } else {
                            Schema::Scalar
                        },
                    )
                })
                .collect(),
        )))
    };
    let mut schema = picker_surface::schema(&[]);
    let Schema::Object(root) = &mut schema else {
        unreachable!()
    };
    root.insert(
        "rows".into(),
        Schema::Collection(Box::new(Schema::Object(BTreeMap::from([
            ("id".into(), Schema::StableId),
            ("disabled".into(), Schema::Boolean),
            (
                "lines".into(),
                scalar_row(&["id", "mark", "name", "value", "source"]),
            ),
            ("description".into(), scalar_row(&["text"])),
            ("heading".into(), scalar_row(&["text"])),
        ])))),
    );
    picker_surface::compile(FILE, &markup, schema)
}
fn notice_role(notice: Option<&SettingNotice>) -> &'static str {
    notice
        .map_or(Role::Dim, |notice| {
            notice.mark.token().role().expect("notice mark has a role")
        })
        .name()
}

pub(super) fn render(frame: &mut Frame, overlay: &Overlay, look: Look, body: Rect) {
    if let Some((name, text)) = &overlay.editing {
        // Raw edit text stays in the existing Config controller. Markup owns
        // fitting, wrapped validation, docking, opacity and fixed key hints.
        let width = tmt_tui::components::Modal {
            title: String::new(),
            placement: tmt_tui::components::Placement::Docked,
        }
        .areas(body, [body.width, body.height], true, true)
        .content
        .width;
        let status = overlay
            .notice
            .as_ref()
            .map(|notice| notice.display(width, &overlay.settings.path))
            .unwrap_or_else(|| format!("{} valid", Mark::Done.symbol()));
        let height = tmt_tui::text::lines(&status, width, tmt_tui::style::TextFlow::Wrap)
            .len()
            .saturating_add(5)
            .min(usize::from(u16::MAX));
        let markup = format!(
            r#"<tmt-view version="1"><tmt-modal id="settings-edit" title="settings · preview" placement="docked" class="w-{} h-{height}"><tmt-scroll id="body"/><tmt-text slot="query" bind="$.query" token="text" class="truncate-middle"/><tmt-text slot="status" bind="$.status" token="{}" wrap="true"/><tmt-text slot="footer" bind="$.footer" token="muted"/></tmt-modal></tmt-view>"#,
            body.width,
            notice_role(overlay.notice.as_ref())
        );
        let key = PromptStyle {
            width: body.width,
            height,
            notice_role: notice_role(overlay.notice.as_ref()),
        };
        let mut cache = overlay.prompt_template.borrow_mut();
        if cache.as_ref().is_none_or(|old| old.key != key) {
            *cache = Some(CachedTemplate {
                key,
                template: picker_surface::compile(FILE, &markup, picker_surface::schema(&["name"])),
            });
        }
        let template = &cache.as_ref().unwrap().template;
        overlay.prompt.borrow_mut().render_modal(FILE, template, json!({"rows":[], "query":format!("{name}: {text}▏"), "status":status, "footer":"Enter save · Esc cancel · Ctrl-U clear", "height":height, "width":body.width}), frame, look, body);
        return;
    }
    let width = usize::from(body.width.saturating_sub(4));
    let row_width = width.saturating_sub(2);
    let key_width = overlay
        .settings
        .entries
        .iter()
        .map(|entry| setting_name(&entry.key).1.width() + usize::from(!entry.editable))
        .max()
        .unwrap_or(0)
        .min(row_width / 4);
    let source_width = (row_width / 3).min(42);
    // Three gaps include the selection marker's track. Shared geometry owns
    // the actual grid fitting at tiny widths.
    let value_width = row_width
        .saturating_sub(key_width + source_width + 6)
        .max(1);
    let selected = overlay
        .surface
        .borrow()
        .picker
        .list
        .selected()
        .map(str::to_owned);
    let mut rows = Vec::new();
    let mut previous = None;
    for entry in &overlay.settings.entries {
        let group = setting_name(&entry.key).0;
        if previous != Some(group) {
            rows.push(json!({"id":format!("group:{group}"), "disabled":true, "lines":[], "description":[], "heading":[{"text":group}]}));
            previous = Some(group);
        }
        let value = match &entry.value {
            Value::Null => "unset".into(),
            Value::Array(items) if items.is_empty() => "none".into(),
            value => crate::settings::display(value),
        };
        let name = format!(
            "{}{}",
            setting_name(&entry.key).1,
            if entry.editable { "" } else { "*" }
        );
        let lines: Vec<_> = value_lines(&escape(&value), value_width, entry.value.is_array() || entry.value.is_object()).into_iter().enumerate().map(|(index,value)| json!({"id":format!("line:{index}"), "mark":if selected.as_deref() == Some(&entry.key) {"›"} else {""}, "name":if index == 0 {name.as_str()} else {""}, "value":value, "source":if index == 0 {entry.source.as_str()} else {""}})).collect();
        rows.push(json!({"id":entry.key, "disabled":false, "lines":lines, "description":entry.description.as_ref().map(|text| vec![json!({"text":text})]).unwrap_or_default(), "heading":[]}));
    }
    let key = ReferenceStyle {
        key_width,
        source_width,
        notice_role: notice_role(overlay.notice.as_ref()),
    };
    let mut cache = overlay.reference_template.borrow_mut();
    if cache.as_ref().is_none_or(|old| old.key != key) {
        *cache = Some(CachedTemplate {
            template: template(key_width, source_width, key.notice_role),
            key,
        });
    }
    let template = &cache.as_ref().unwrap().template;
    let mut state = overlay.surface.borrow_mut();
    state.render(FILE, template, json!({"rows":rows, "query":format!("{} {} · {}",overlay.settings.context.as_deref().unwrap_or("board defaults"), overlay.settings.host, overlay.display_path), "notes":overlay.settings.notices.iter().enumerate().map(|(index,text)| json!({"id":format!("notice:{index}"),"text":text})).collect::<Vec<_>>(), "status":overlay.notice.as_ref().map(|notice| notice.display(width as u16, &overlay.settings.path)).unwrap_or_default(), "footer":"↑↓ select · Enter edit · PgUp/PgDn page · Esc close", "tracks":[key_width,source_width], "role":notice_role(overlay.notice.as_ref())}), frame, look, body);
    if let Some(map) = &state.frame {
        for hit in &map.hits {
            let Some(entry) = hit.row_id.as_ref().and_then(|id| {
                overlay
                    .settings
                    .entries
                    .iter()
                    .find(|entry| &entry.key == id)
            }) else {
                continue;
            };
            let role = match hit.id.last().map(String::as_str) {
                Some("name") if !entry.editable => Role::Muted,
                Some("value")
                    if entry.value.is_null()
                        || entry.value.as_array().is_some_and(Vec::is_empty) =>
                {
                    Role::Dim
                }
                _ => continue,
            };
            let selected = selected.as_deref() == Some(&entry.key);
            frame
                .buffer_mut()
                .set_style(hit.rect, look.row_span(selected, look.role(role), false));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Pane;
    use ratatui::{Terminal, backend::TestBackend, crossterm::event::KeyModifiers};
    #[test]
    fn structured_values_break_at_punctuation_preserve_quotes_and_cap_lines() {
        for text in ["x,y:z", r#"x\"y,z:q"#] {
            let value = serde_json::json!({"a":text, "b":1}).to_string();
            let lines = value_lines(&escape(&value), 20, true);
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
            assert!(overlay.surface.borrow().picker.list.scroll.offset() > 0);
            overlay.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
            assert_eq!(
                overlay.surface.borrow().picker.list.selected(),
                Some("board.mode")
            );
            terminal
                .draw(|frame| render(frame, &overlay, look, frame.area()))
                .unwrap();
            let state = overlay.surface.borrow();
            let map = state.frame.as_ref().unwrap();
            assert!(
                map.list
                    .as_ref()
                    .unwrap()
                    .geometry
                    .iter()
                    .any(|row| row.id == "board.mode" && row.visible.height > 0)
            );
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
        overlay.surface.borrow_mut().select(key);
        assert_eq!(press(app, KeyCode::Enter), crate::board::app::Effect::None);
        app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for ch in text.chars() {
            press(app, KeyCode::Char(ch));
        }
    }
    #[test]
    fn admitted_settings_rows_use_current_hits_and_restore_list_after_raw_edit() {
        use ratatui::crossterm::event::{MouseButton, MouseEventKind};
        let mut f = fixture("component-hits", "");
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let look = f.app.look();
        let paint = |frame: &mut Frame, app: &crate::board::app::App| {
            render(frame, app.settings.as_ref().unwrap(), look, frame.area());
        };
        terminal.draw(|frame| paint(frame, &f.app)).unwrap();
        let hit = f
            .app
            .settings
            .as_ref()
            .unwrap()
            .surface
            .borrow()
            .frame
            .as_ref()
            .unwrap()
            .list
            .as_ref()
            .unwrap()
            .geometry
            .iter()
            .find(|row| row.id == "board.sizes")
            .unwrap()
            .visible;
        assert!(hit.height > 0);
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        };
        f.app.settings.as_ref().unwrap().mouse(mouse);
        assert_eq!(
            f.app
                .settings
                .as_ref()
                .unwrap()
                .surface
                .borrow()
                .picker
                .list
                .selected(),
            Some("board.sizes")
        );
        let before = f
            .app
            .settings
            .as_ref()
            .unwrap()
            .surface
            .borrow()
            .picker
            .list
            .scroll
            .offset();
        edit(&mut f.app, "board.sizes", "[30,70]");
        let overlay = f.app.settings.as_ref().unwrap();
        overlay.mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            ..mouse
        });
        assert_eq!(overlay.surface.borrow().picker.list.scroll.offset(), before);
        terminal.draw(|frame| paint(frame, &f.app)).unwrap();
        let overlay = f.app.settings.as_ref().unwrap();
        let prompt = overlay.prompt.borrow();
        let map = prompt.frame.as_ref().unwrap();
        assert_eq!(map.areas.outer.bottom(), 24);
        assert!(map.areas.outer.height <= 7);
        assert!(map.list.is_none());
        assert_eq!(prompt.picker.list.scroll.content(), 0);
        drop(prompt);
        press(&mut f.app, KeyCode::Esc);
        assert!(!f.app.settings.as_ref().unwrap().editing());
        assert_eq!(
            f.app
                .settings
                .as_ref()
                .unwrap()
                .surface
                .borrow()
                .picker
                .list
                .selected(),
            Some("board.sizes")
        );
        assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
        f.app.invalidate_overlay_frames();
        let old = f
            .app
            .settings
            .as_ref()
            .unwrap()
            .surface
            .borrow()
            .picker
            .list
            .selected()
            .map(str::to_owned);
        f.app.settings.as_ref().unwrap().mouse(mouse);
        assert_eq!(
            f.app
                .settings
                .as_ref()
                .unwrap()
                .surface
                .borrow()
                .picker
                .list
                .selected(),
            old.as_deref()
        );
    }
    #[test]
    fn docked_edit_reserves_width_for_query_and_wrapped_validation_at_all_board_widths() {
        let mut f = fixture("prompt-width", "");
        edit(&mut f.app, "board.sizes", "[30,70]");
        for width in [160, 100, 80, 24, 160] {
            let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
            let overlay = f.app.settings.as_ref().unwrap();
            terminal
                .draw(|frame| render(frame, overlay, f.app.look(), frame.area()))
                .unwrap();
            let state = overlay.prompt.borrow();
            let map = state.frame.as_ref().unwrap();
            assert_eq!(
                map.areas.outer.width,
                if width < 100 { width } else { width * 9 / 10 }
            );
            assert_eq!(map.areas.outer.bottom(), 30);
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains("▏"));
            assert!(text.contains("✓ valid"));
            if width >= 80 {
                assert!(text.contains("board.sizes: [30,70]▏"));
            }
        }
        assert_eq!(std::fs::read_to_string(&f.path).unwrap(), f.original);
    }
    #[test]
    fn saved_error_and_warning_notices_use_admitted_existing_roles() {
        let mut f = fixture("notice-roles", "");
        for mark in [Mark::Done, Mark::Failed, Mark::Warning] {
            let overlay = f.app.settings.as_mut().unwrap();
            overlay.notice = Some(SettingNotice {
                mark,
                message: "Retained result".into(),
            });
            let role = mark.token().role().unwrap();
            assert_eq!(notice_role(overlay.notice.as_ref()), role.name());
            let look = Look::new(tmt_cli_style::Theme::default());
            let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
            terminal
                .draw(|frame| render(frame, overlay, look, frame.area()))
                .unwrap();
            let state = overlay.surface.borrow();
            let status = state.frame.as_ref().unwrap().areas.status;
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[(status.x, status.y)].symbol(), mark.symbol());
            assert_eq!(
                buffer[(status.x, status.y)].fg,
                look.role(role).fg.unwrap_or_default()
            );
        }
    }
    #[test]
    fn narrow_notices_keep_mark_with_short_file_or_long_leading_word() {
        let mut f = fixture("narrow-notice", "");
        edit(&mut f.app, "notes.render", "plain");
        let concurrent = format!("{}\n# external writer retained\n", f.original);
        std::fs::write(&f.path, &concurrent).unwrap();
        assert!(!f.app.settings.as_mut().unwrap().save());
        assert!(
            f.app
                .settings
                .as_ref()
                .unwrap()
                .notice
                .as_ref()
                .unwrap()
                .message
                .contains(f.path.to_str().unwrap())
        );
        for width in [80, 24] {
            let overlay = f.app.settings.as_ref().unwrap();
            let mut terminal = Terminal::new(TestBackend::new(width, 30)).unwrap();
            terminal
                .draw(|frame| render(frame, overlay, f.app.look(), frame.area()))
                .unwrap();
            let state = overlay.prompt.borrow();
            let area = state.frame.as_ref().unwrap().areas.status;
            let line: String = (area.x..area.right())
                .map(|x| terminal.backend().buffer()[(x, area.y)].symbol())
                .collect();
            assert!(line.starts_with("✗ squad.toml"), "{line:?}");
            assert!(!line.contains("/private/"));
        }
        let notice = SettingNotice {
            message: format!("{} needs attention", "unbreakable".repeat(20)),
            mark: Mark::Warning,
        };
        for width in [76, 20] {
            let display = notice.display(width, f.path.to_str().unwrap());
            let lines = tmt_tui::text::lines(&display, width, tmt_tui::style::TextFlow::Wrap);
            assert!(lines[0].starts_with("! unbreak"));
            assert!(lines[0].contains('…'));
            assert!(lines.join(" ").contains("needs attention"));
        }
        assert_eq!(std::fs::read_to_string(&f.path).unwrap(), concurrent);
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
            overlay.surface.borrow_mut().select(key);
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
