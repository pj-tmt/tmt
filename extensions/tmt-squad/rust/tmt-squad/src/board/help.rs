//! Help projects board data; shared TUI components own layout, scrolling and modality.
use super::app::App;
use crate::look::Look;
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode},
    layout::Rect,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::OnceLock};
use tmt_tui::{
    binding::{Schema, Schemas, Scopes, Sources},
    components::{
        KeyHelp, KeyHelpEntry, KeyHelpSection, ScrollState,
        surface::{self, ModalSurface, RenderStyle},
    },
};

const FILE: &str = "squad.help.xml";
const MARKUP: &str = r#"<tmt-view version="1"><tmt-modal id="help" title="help" placement="body"><tmt-scroll id="help-body"><tmt-key-help id="help-keys" bind="$.help" heading-token="text" heading-bold="true" section-gap="1"/></tmt-scroll><tmt-text slot="footer" token="muted" bind="$.footer"/></tmt-modal></tmt-view>"#;
pub(super) const FOOTER: &str = "↑↓ scroll · PgUp/PgDn page · Esc close";
struct Data;
impl Sources for Data {
    type Source = ();
    fn compile(&self, _: &str, _: &str, _: &Schemas<'_>) -> Result<(), String> {
        Err("help has no field sources".into())
    }
    fn resolve(&self, _: &(), _: &Scopes<'_>) -> Result<Option<String>, String> {
        Err("help has no field sources".into())
    }
}
fn template() -> &'static surface::Template<()> {
    static TEMPLATE: OnceLock<surface::Template<()>> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let parsed = tmt_tui::parse(FILE, MARKUP).expect("embedded help markup");
        let schema = Schema::Object(BTreeMap::from([
            ("help".into(), KeyHelp::schema()),
            ("footer".into(), Schema::Scalar),
        ]));
        surface::compile(FILE, &parsed, &schema, &Data).expect("embedded help schema")
    })
}

#[derive(Default)]
pub(super) struct Help {
    pub scroll: ScrollState,
    scene: Option<(Value, ModalSurface)>,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Input {
    Close,
    Scroll,
}
impl Help {
    pub fn open(&mut self) {
        *self = Self::default();
    }
    pub fn input(&mut self, event: &Event) -> Option<Input> {
        if matches!(event, Event::Key(key) if matches!(key.code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')))
        {
            return Some(Input::Close);
        }
        self.scroll.input(event).then_some(Input::Scroll)
    }
    fn render(&mut self, frame: &mut Frame, model: KeyHelp, look: Look, body: Rect) {
        let value = json!({"help": model.value(), "footer": FOOTER});
        if self.scene.as_ref().is_none_or(|(old, _)| old != &value) {
            let scene = template()
                .materialize(FILE, &value, &Data)
                .expect("typed help data");
            self.scene = Some((value, scene));
        }
        surface::render(
            &self.scene.as_ref().unwrap().1,
            &mut self.scroll,
            body,
            frame.buffer_mut(),
            RenderStyle {
                theme: &look.theme,
                depth: look.depth,
            },
            |role| {
                look.selection()
                    .patch(look.row_span(true, look.role(role), false))
            },
        )
        .expect("admitted help surface");
    }
}
fn entry(id: &str, keys: &str, description: impl Into<String>) -> KeyHelpEntry {
    KeyHelpEntry {
        id: id.into(),
        keys: keys.into(),
        description: description.into(),
    }
}
fn section(id: &str, title: &str, rows: &[(&str, &str)]) -> KeyHelpSection {
    KeyHelpSection {
        id: id.into(),
        title: title.into(),
        entries: rows
            .iter()
            .enumerate()
            .map(|(i, (keys, description))| entry(&format!("row-{i}"), keys, *description))
            .collect(),
    }
}
pub(super) fn model(app: &App) -> KeyHelp {
    let home = app.view.as_ref().is_some_and(|view| view.home.is_some());
    let navigation = if home {
        section(
            "navigation",
            "navigation",
            &[
                ("↑↓ / j k", "move through attention rows, then squads"),
                ("Tab / Shift-Tab", "move to the next or previous section"),
                ("Enter", "go to a member or open the selected squad"),
                ("a", "answer a request or send the squad lead a note"),
                ("← →", "switch tabs"),
                ("s", "switch to any tab"),
                ("/", "search names and squads"),
                (
                    "obs age",
                    "time since observed unchanged task or state, not time blocked",
                ),
                ("?", "show help"),
                ("q / Esc", "close the board"),
            ],
        )
    } else {
        section(
            "navigation",
            "navigation",
            &[
                (
                    "↑↓ / j k",
                    "select a row or notebook line; scroll detail or replies",
                ),
                ("g G", "go to the first or last notebook line"),
                ("PgUp / PgDn", "page the focused pane"),
                ("Home / End", "go to the top or bottom of the focused pane"),
                ("wheel", "scroll the pane under the pointer"),
                (
                    "title click",
                    "fold or expand a split pane (▸ means folded)",
                ),
                (
                    "Shift-drag",
                    "select text to copy (Option-drag in some terminals)",
                ),
                ("← →", "switch tabs; Shift+← → or drag a tab to move it"),
                ("/", "search; Esc clears"),
                ("?", "show help"),
                ("q / Esc", "close the board"),
            ],
        )
    };
    let mut sections = vec![navigation];
    if !home && !app.bindings().contains_key("s") {
        sections[0].entries.push(entry(
            "switcher",
            "s",
            "switch to any tab, hidden ones too (type to filter)",
        ));
    }
    if let Some(view) = &app.view {
        sections[0].entries.push(entry(
            "reload",
            "reload",
            match view.refresh {
                Some(every) => format!("automatically every {}s", every.as_secs()),
                None => "automatic reload is off".into(),
            },
        ));
    }
    if let Some(rate) = app.view.as_ref().and_then(|view| view.token_rate.as_ref()) {
        let windows = format!(
            "{}; observed totals, no persisted history",
            rate.settings
                .windows
                .map(|window| window.label())
                .join(" / ")
        );
        let mut meter = section(
            "meter",
            "tokens",
            &[
                ("windows", &windows),
                ("trend", "eight bucket-aligned observed-token slices"),
                (
                    "coverage",
                    "no consumption data is explicit; measured zero is 0; – is unreported; ~ is incomplete window/coverage",
                ),
            ],
        );
        meter.entries.insert(
            0,
            entry(
                "sampling",
                "tokens",
                format!(
                    "completed requests observed every {} s; best effort, current session model",
                    rate.settings.every.as_secs()
                ),
            ),
        );
        if !app.excluded_counters.is_empty() {
            let names = app
                .excluded_counters
                .iter()
                .map(|id| {
                    rate.input
                        .names
                        .get(id)
                        .map(String::as_str)
                        .unwrap_or("unknown member")
                })
                .collect::<Vec<_>>()
                .join(", ");
            meter.entries.push(entry(
                "excluded",
                "excluded",
                format!("{names}: no usage counters at last board refresh"),
            ));
        }
        sections.push(meter);
    }
    let mut bindings = app.bindings();
    let mut entries = Vec::new();
    for key in ["l", "L", "T"].into_iter().map(str::to_owned).chain(
        bindings
            .keys()
            .filter(|key| !matches!(key.as_str(), "l" | "L" | "T"))
            .cloned()
            .collect::<Vec<_>>(),
    ) {
        if let Some(action) = bindings.remove(&key) {
            let mut description = action.description();
            if action.verb == crate::action::Verb::Toggle
                && super::view::toggle_label(app, &action).is_some()
            {
                description.push_str(" (▾ open, ▸ folded)");
            }
            entries.push(entry(&format!("binding-{key}"), &key, description));
        }
    }
    sections.push(KeyHelpSection {
        id: "bindings".into(),
        title: "bindings".into(),
        entries,
    });
    KeyHelp { sections }
}
pub(super) fn render(frame: &mut Frame, app: &App, body: Rect) {
    let mut help = app.help_state.borrow_mut();
    help.render(frame, model(app), app.look(), body);
}
