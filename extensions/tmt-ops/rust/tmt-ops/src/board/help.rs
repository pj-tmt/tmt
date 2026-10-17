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
    let entries = app
        .bindings()
        .into_iter()
        .map(|(key, action)| {
            let description =
                if action.verb == crate::action::Verb::Ui(crate::action::OpsUiAction::Pin) {
                    match app.workspace.selected {
                        Some(crate::board::workspace::OpsTarget::Member { .. }) => {
                            "pin or unpin the selected agent".into()
                        }
                        Some(crate::board::workspace::OpsTarget::Squad(_)) => {
                            "pin or unpin the selected squad".into()
                        }
                        _ => "pin or unpin the selected agent or squad".into(),
                    }
                } else if app.workspace_snapshot().is_some()
                    && action.verb == crate::action::Verb::Annotate
                    && action.args.first().and_then(|arg| arg.literal()) != Some("member")
                {
                    match app.workspace.selected {
                        Some(crate::board::workspace::OpsTarget::Squad(_)) => {
                            "write a note to the selected squad's lead".into()
                        }
                        _ => "write a note to the selected agent's lead".into(),
                    }
                } else {
                    action.description()
                };
            entry(&format!("binding-{key}"), &key, description)
        })
        .collect();
    let mut sections = vec![
        KeyHelpSection {
            id: "bindings".into(),
            title: "effective scoped bindings".into(),
            entries,
        },
        section(
            "workspace",
            "workspace navigation",
            &[
                (
                    "shell",
                    "effective bindings above select Overview / Agents / Digest from any non-typing pane or overlay; active editors, search and completion keep their input",
                ),
                (
                    "tree",
                    "effective bindings above collapse/expand the selected squad; no squad-tab switching",
                ),
                (
                    "Shift-Left/Right",
                    "old tab reorder is removed; use squad pins/sort instead",
                ),
                (
                    "wheel",
                    "scroll the pane under the pointer without moving the selected target",
                ),
                (
                    "divider drag",
                    "drag a visible separator to resize; hover shows the axis; double-click restores its default",
                ),
                (
                    "resize-pane",
                    "optional authored action: resize-pane <boundary> <left|right|up|down|reset>; no default binding",
                ),
                ("Esc", "step back one level; at the list stay in Ops UI"),
                (
                    "quit",
                    "use the effective quit binding above (default q) or Ctrl-C; q closes an overlay and types in an editor",
                ),
            ],
        ),
        section(
            "reader",
            "conversation reader",
            &[
                (
                    "scoped keys",
                    "Reader uses its own effective bindings above; defaults scroll without jumping, and explicit compatible overrides win",
                ),
                ("Esc", "return focus to the list"),
                (
                    "pane focus",
                    "use the effective next-pane binding to move between visible list and conversation panes",
                ),
            ],
        ),
    ];
    if matches!(
        app.shell.tab,
        super::shell::OpsTab::Overview | super::shell::OpsTab::Digest
    ) {
        sections.push(section("usage-chart", "usage chart", &[
            ("controls", "focus the chart control with its effective binding; Tab changes fields, Enter opens Range or Each, arrows choose and Enter applies; Esc closes"),
            ("Auto", "uses the finest interval fitting the plot width and 720 buckets; fixed intervals survive resize; disabled choices explain the minimum interval"),
            ("history", "arrows or wheel scroll; Home/End reach oldest/newest; hover or keyboard selection shows interval tokens and provider shares inline"),
            ("readout", "click or Enter pins the selected interval; click again or Esc unpins; no Details window"),
        ]));
    }
    if matches!(
        app.shell.tab,
        super::shell::OpsTab::Overview | super::shell::OpsTab::Digest
    ) && super::home::limits_enabled(app)
        && app.limits.as_ref().is_none_or(|values| !values.is_empty())
    {
        sections.push(section("provider-limits", "provider limits", &[
            ("balance", "remaining share inside each reported window bar; (7d)/(5h) are provider quota windows, separate from token usage windows"),
            ("freshness", "missing or invalid readings, or readings older than 2 hours, remain unknown; they are never zero"),
            ("warning", "a depletion estimate is shown only from measured weekly use; reset countdown, pace and trend fit only when reported and space permits"),
            ("details", "click a quota row or choose Quota details from Actions for source timestamps, reset and pace; no new default key"),
        ]));
    }
    if let Some(snapshot) = app
        .workspace_snapshot()
        .filter(|snapshot| !snapshot.notices.is_empty())
    {
        sections.push(KeyHelpSection {
            id: "migration-notices".into(),
            title: "configuration migration (file preserved)".into(),
            entries: snapshot
                .notices
                .iter()
                .enumerate()
                .map(|(index, notice)| entry(&format!("notice-{index}"), "", notice.clone()))
                .collect(),
        });
    }
    sections.push(KeyHelpSection {
        id: "composer-focus".into(), title: "message composer focus".into(),
        entries: vec![entry("focus-message", "", format!("{}; opening keeps the draft and sends nothing; double-click still inspects content", app.composer_focus_hint()))],
    });
    if app.member_link_hint().is_some() {
        sections.push(section("member-links", "member links", &[
            ("#tmt:name / #tmt:UUID", "Known member links open an anchored action menu. Unknown references stay literal; copy and annotation keep exact markup."),
            ("arrows / Enter", "In conversation read focus, left/right select a visible mention; Enter opens its actions. Tab and BackTab switch panes; authored bindings take precedence. j uses its configured host action; Enter shows the member in Agents and keeps the draft."),
            ("Esc / click outside", "Close the mention menu without acting. Links never broadcast or change the message recipient."),
        ]));
    }
    sections.push(section("digest-estimate", "Digest estimate", &[
        ("window", "Choose Digest estimate window in workspace settings: 24h, 7d or all time; the choice is shared by member, squad and Digest views."),
        ("Est. ctx avoided", "Estimated context input tokens avoided by delivering messages together. Assumes each message would otherwise have been delivered on its own. Not billed tokens, not cost, not turns saved."),
        ("coverage", "— means unavailable; known zero is 0. (N of M) counts included members. All time starts when each member's Digest began recording."),
        ("unit", "Click the estimate, choose Savings unit in the Actions menu, or press a key bound to savings-unit, to cycle tokens, percent of input and cost; the choice is shared by every view. Percent is not available for all time. Cost is an estimate at list price for uncached input, never money saved or a bill; Digest Details lists the price table behind it."),
    ]));
    sections.push(section("editor", "message editor", &[
        ("Enter", "while typing, send to the displayed recipient; while @ completion is open, Enter/Tab insert the selected reference instead"),
        ("Shift+Enter / Ctrl+N", "Ctrl+N always inserts a new line; Alt+Enter and Ctrl+J do too; Shift+Enter needs enhanced terminal key reporting"),
        ("Esc", "leave the editor and keep its draft; drafts and folded pastes are session-only and do not survive restart"),
        ("arrows / Home / End", "move by grapheme or wrapped line; Shift selects; Ctrl-A selects all"),
        ("click / drag", "place the caret or select text; terminal IME commits arrive as ordinary input"),
        ("paste / Ctrl-P", "bracketed paste never sends; large pastes fold, Ctrl-P previews the range at the caret, Backspace/Delete removes a folded range"),
        ("@", "search agent and squad references; mentions are text only and never broadcast"),
        ("response: s", "select exact response source with Shift-arrows or drag; y copies, a attaches its request and byte range to a draft without acknowledging it"),
        ("Alt-Backspace", "remove the last response quote from the draft"),
        ("native selection", "use the terminal emulator's selection override modifier, or use source selection inside the response reader"),
    ]));
    KeyHelp { sections }
}

pub(super) fn render(frame: &mut Frame, app: &App, body: Rect) {
    let mut help = app.help_state.borrow_mut();
    help.render(frame, model(app), app.look(), body);
}
