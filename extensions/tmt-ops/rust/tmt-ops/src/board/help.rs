//! Help projects board data; shared TUI components own layout, scrolling and modality.
//! The `?` panel shows the keys above and the workspace settings rows below them.
use super::{app::App, picker_surface};
use crate::look::Look;
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyEventKind},
    layout::Rect,
};
use serde_json::json;
use std::sync::OnceLock;
use tmt_tui::{
    binding::Schema,
    components::{
        KeyHelp, KeyHelpEntry, KeyHelpSection, ListRow, PickerEvent, PickerField, PickerInput,
        surface,
    },
};

const FILE: &str = "squad.help.xml";
const MARKUP: &str = r#"<tmt-view version="1"><tmt-modal id="help" title="help" placement="body"><tmt-scroll id="help-body"><tmt-key-help id="help-keys" bind="$.help" heading-token="text" heading-bold="true" section-gap="1"/><tmt-list id="choices" bind="$.rows" empty=""><tmt-row class="flex-row gap-1"><tmt-text bind="row.cursor" class="w-1 shrink-0"/><tmt-text bind="row.label" token="text"/></tmt-row></tmt-list></tmt-scroll><tmt-text slot="footer" token="muted" bind="$.footer"/></tmt-modal></tmt-view>"#;
pub(super) const FOOTER: &str = "↑↓ scroll · PgUp/PgDn page · Esc close";
/// The reading footer when settings rows follow the keys.
pub(super) const READING_FOOTER: &str =
    "↑↓ scroll · PgUp/PgDn page · ↓ at the end: settings · Esc close";
const SETTINGS_FOOTER: &str = "↑↓ choose · Enter apply or open · ↑ on the first: keys · Esc close";
fn template() -> &'static surface::Template<()> {
    static TEMPLATE: OnceLock<surface::Template<()>> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let Schema::Object(mut fields) = picker_surface::schema(&["cursor", "label"]) else {
            unreachable!("picker scenes are objects")
        };
        fields.insert("help".into(), KeyHelp::schema());
        picker_surface::compile(FILE, MARKUP, Schema::Object(fields))
    })
}

pub(super) struct Help {
    ui: picker_surface::State,
    /// The settings row last chosen: `,` reopens on it.
    row: Option<String>,
    /// The row to select once the rows are projected.
    reselect: Option<String>,
    /// The last projected rows: (label, divider).
    settings: Vec<(String, bool)>,
}
impl Default for Help {
    fn default() -> Self {
        let mut ui = picker_surface::State::new(None, vec![], None);
        ui.list_focus = false;
        Self {
            ui,
            row: None,
            reselect: None,
            settings: Vec::new(),
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Input {
    Close,
    Scroll,
    /// Enter or a click on the settings row with this index.
    Choose(usize),
}
impl Help {
    /// Opens on the keys, or on the first settings row for `,`.
    pub fn open(&mut self, settings: bool) {
        let row = self.row.take();
        *self = Self::default();
        self.ui.list_focus = settings;
        if settings {
            self.reselect.clone_from(&row);
        }
        self.row = row;
    }
    /// The scroll of the whole panel: the keys, then the settings rows.
    #[cfg(test)]
    pub fn scroll(&self) -> &tmt_tui::components::ScrollState {
        &self.ui.picker.list.scroll
    }
    /// The index of the selected settings row.
    #[cfg(test)]
    pub fn selected_setting(&self) -> Option<usize> {
        self.selected_setting_index()
    }
    fn selected_setting_index(&self) -> Option<usize> {
        self.ui
            .picker
            .list
            .selected()?
            .strip_prefix("setting-")?
            .parse()
            .ok()
    }
    /// Whether the settings rows hold ↑↓ instead of the reading.
    #[cfg(test)]
    pub fn settings_focus(&self) -> bool {
        self.ui.list_focus
    }
    fn first_row(&self) -> Option<String> {
        self.ui
            .picker
            .list
            .rows()
            .iter()
            .find(|row| !row.disabled)
            .map(|row| row.id.clone())
    }
    pub fn input(&mut self, event: &Event) -> Option<Input> {
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                return None;
            }
            if matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
            ) {
                return Some(Input::Close);
            }
            // ↓ past the end of the keys reaches the settings rows; ↑ on the first row reads again.
            let first = self.first_row();
            let scroll = &self.ui.picker.list.scroll;
            let to_settings = !self.ui.list_focus
                && key.code == KeyCode::Down
                && !self.ui.picker.list.rows().is_empty()
                && scroll.offset() >= scroll.limit();
            let to_reading = self.ui.list_focus
                && key.code == KeyCode::Up
                && self.ui.picker.list.selected() == first.as_deref();
            if to_settings || to_reading {
                self.ui.list_focus = to_settings;
                self.reconcile(&self.settings.clone());
                self.ui.invalidate();
                return Some(Input::Scroll);
            }
        }
        let reading_key = !self.ui.list_focus && matches!(event, Event::Key(_));
        if reading_key
            || matches!(event, Event::Mouse(mouse) if matches!(mouse.kind, ratatui::crossterm::event::MouseEventKind::ScrollUp | ratatui::crossterm::event::MouseEventKind::ScrollDown))
        {
            let moved = self.ui.picker.list.scroll.input(event);
            if moved {
                self.ui.invalidate();
            }
            return moved.then_some(Input::Scroll);
        }
        match self.ui.input(event, PickerField::List)? {
            PickerInput::Event(PickerEvent::Confirm(id)) => {
                self.ui.list_focus = true;
                id.strip_prefix("setting-")?.parse().ok().map(Input::Choose)
            }
            _ => Some(Input::Scroll),
        }
    }
    /// Projects the settings rows; keys may arrive before the first paint.
    pub fn reconcile(&mut self, settings: &[(String, bool)]) {
        settings.clone_into(&mut self.settings);
        let focus = self.ui.list_focus;
        // While the keys are read the rows hold no selection, so the list never scrolls the
        // panel to them; ↓ past the end of the keys makes them choosable.
        let rows: Vec<ListRow> = settings
            .iter()
            .enumerate()
            .map(|(index, (_, divider))| ListRow {
                id: format!("setting-{index}"),
                disabled: *divider || !focus,
            })
            .collect();
        self.ui.reconcile(rows);
        if let Some(id) = self.reselect.take()
            && self
                .ui
                .picker
                .list
                .rows()
                .iter()
                .any(|row| row.id == id && !row.disabled)
        {
            self.ui.select(&id);
        }
        if focus {
            self.row = self.ui.picker.list.selected().map(str::to_owned);
        }
    }
    fn render(
        &mut self,
        frame: &mut Frame,
        model: KeyHelp,
        settings: &[(String, bool)],
        look: Look,
        body: Rect,
    ) -> Option<Rect> {
        self.reconcile(settings);
        let focus = self.ui.list_focus;
        let selected = self.ui.picker.list.selected().map(str::to_owned);
        let value = json!({
            "help": model.value(),
            "rows": settings.iter().enumerate().map(|(index, (label, divider))| {
                let id = format!("setting-{index}");
                json!({"id": id, "disabled": *divider || !focus, "label": label,
                    "cursor": if focus && selected.as_deref() == Some(id.as_str()) {"›"} else {""}})
            }).collect::<Vec<_>>(),
            "query": "", "status": "", "notes": [],
            "footer": if focus { SETTINGS_FOOTER } else if settings.is_empty() { FOOTER } else { READING_FOOTER },
        });
        self.ui
            .render(FILE, template(), value.clone(), frame, look, body);
        // Settings focus keeps the chosen row and the rows after it in view, e.g. when `,`
        // opens the panel, as far as they fit.
        let after = self
            .selected_setting_index()
            .map_or(0, |index| settings.len().saturating_sub(index + 1));
        if focus
            && let Some(id) = selected
            && self.ui.reveal_group(&id, &id, after) == Some(true)
        {
            self.ui.render(FILE, template(), value, frame, look, body);
        }
        // While the keys are read the rows hold no selection, so the list prints no position;
        // the reading's own "N more ↓" is the sign that the settings follow below the fold.
        if !focus
            && let Some(area) = self.ui.frame.as_ref().map(|frame| frame.areas.position)
            && let Some(position) = self.ui.picker.list.scroll.overflow_position()
        {
            tmt_tui::components::strip::paint_left(
                frame.buffer_mut(),
                area,
                ratatui::text::Line::styled(position, look.role(tmt_cli_style::Role::Muted)),
            );
        }
        self.ui.outer()
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
                } else if app.workspace_snapshot().is_some() {
                    action.workspace_description()
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
                    "@",
                    "selected row names: one opens its menu; several focus the first, Left/Right chooses, Enter opens, Esc or row movement leaves; authored @ keeps priority",
                ),
                (
                    "Shift-Left/Right",
                    "old tab reorder is removed; use squad pins/sort instead",
                ),
                (
                    "wheel",
                    "scroll the pane under the pointer without moving the selected target; if the wheel scrolls tmux instead, run tmux set -g mouse on",
                ),
                (
                    "right-click row",
                    "select a visible Agents row and open Actions; the settings menu also offers Actions",
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
                (
                    "unread",
                    "moving the selection onto an agent never marks its replies read; they turn read when the reader holds focus on the shown reply or after a successful jump; u or a click on any unread count opens the Unread overlay, whose shown answers turn read",
                ),
            ],
        ),
    ];
    if matches!(
        app.shell.tab,
        super::shell::OpsTab::Agents | super::shell::OpsTab::Digest
    ) {
        sections.push(section("usage-chart", "usage chart", &[
            ("controls", "focus the chart control with its effective binding; ↑↓ change the control, ←→ choose a value or bar, Enter opens Range or Each and applies; Esc closes"),
            ("Auto", "uses the finest interval fitting the plot width and 720 buckets; fixed intervals survive resize; disabled choices explain the minimum interval"),
            ("history", "arrows or wheel scroll; Home/End reach oldest/newest; hover or keyboard selection shows interval tokens and provider shares inline"),
            ("readout", "click or Enter pins the selected interval; click again or Esc unpins; no Details window"),
        ]));
    }
    if matches!(
        app.shell.tab,
        super::shell::OpsTab::Agents | super::shell::OpsTab::Digest
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
        ("window", "Choose Digest estimate window in workspace settings: 1d, 7d or all time; the choice is shared by member, squad and Digest views."),
        ("Est. ctx avoided", "Estimated context input tokens avoided by delivering messages together. Assumes each message would otherwise have been delivered on its own. Not billed tokens, not cost, not turns saved."),
        ("coverage", "— means unavailable; known zero is 0. (N of M) counts included members. All time starts when each member's Digest began recording."),
        ("unit", "Click the estimate, choose Savings unit in the Actions menu, or press a key bound to savings-unit, to cycle tokens, percent of input and cost; the choice is shared by every view. In Agents, click the TOK or ≈$ word of the column heading (hover shows the unit it switches to), or any TOK figure, to switch the column between tokens and cost; the sort for that column is the one mark cell before the word. Percent is not available for all time. Cost is an estimate at list price for uncached input, never money saved or a bill; Digest Details lists the price table behind it."),
    ]));
    sections.push(section("chat-delivery", "chat delivery", &[
        ("direct message", "A person's eligible message goes straight to the agent's pane. Core settles the oldest open request after delivery; capture hooks do not decide eligibility."),
        ("request fallback", "If Core cannot offer direct input, the composer explains that it sent a request instead. A refusal keeps the draft; uncertain acceptance never resends."),
        ("Send now", "While Core says the newest message you sent can go now (held for a Digest checklist, or waiting to be typed), a Send now chip and Ctrl-S type it at once. Core decides who may and what the state allows; its sentence is the result, and nothing is retried."),
        ("turn output", "Captured output appears under its message, marked truncated when needed. working… follows Core's pending or bound capture; Missing capture says no turn output was captured; it is never presented as a reply."),
    ]));
    sections.push(section("editor", "message editor", &[
        ("Enter", "while typing, send to the displayed recipient; while the @ or tmt/ picker is open, Enter/Tab insert the selected agent instead"),
        ("Shift+Enter / Ctrl+N", "Ctrl+N always inserts a new line; Alt+Enter and Ctrl+J do too; Shift+Enter needs enhanced key reporting (in tmux: extended-keys on and extended-keys-format csi-u)"),
        ("Esc", "leave the editor and keep its draft; drafts and folded pastes are session-only and do not survive restart"),
        ("arrows / Home / End", "move by grapheme or wrapped line; Shift selects; Ctrl-A/E: line edges"),
        ("click / drag", "place the caret or select text; terminal IME commits arrive as ordinary input"),
        ("paste / Ctrl-P", "bracketed paste never sends; large pastes fold, Ctrl-P previews the range at the caret, Backspace/Delete removes a folded range"),
        ("@ / tmt/", "at a word start, pick an agent to mention; the draft keeps who it is, the sent text is tmt/<name> and never broadcasts"),
        ("response: s", "select exact response source with Shift-arrows or drag; y copies, a attaches its request and byte range to a draft without acknowledging it"),
        ("Alt-Backspace", "delete the word before the draft cursor"),
        ("native selection", "use the terminal emulator's selection override modifier, or use source selection inside the response reader"),
    ]));
    if let Some(row) = app.selected_row() {
        let ops = app.parameter_observations(row);
        let parameters = crate::template::Parameters {
            row,
            ops: Some(&ops),
            board_pane: None,
        };
        sections.push(KeyHelpSection {
            id: "binding-parameters".into(),
            title: "binding parameters · held row sample".into(),
            entries: parameters
                .inventory()
                .into_iter()
                .enumerate()
                .map(|(index, text)| entry(&format!("parameter-{index}"), "", text))
                .collect(),
        });
    }
    if app.workspace_scope().is_reader() {
        let bindings = app.bindings();
        let entries: Vec<_> = crate::action::OpsUiAction::strip_actions()
            .filter(|ui| {
                !bindings
                    .values()
                    .any(|action| action.verb == crate::action::Verb::Ui(*ui))
            })
            .filter_map(|ui| {
                super::keycap::key(app, ui, "")
                    .map(|key| entry(ui.spec().name, &key, ui.spec().description))
            })
            .collect();
        if !entries.is_empty() {
            sections.push(KeyHelpSection {
                id: "strip-keys".into(),
                title: "tab strip shortcuts (disabled in this scope)".into(),
                entries,
            });
        }
    }
    if let Some(versions) = &app.versions {
        sections.push(section(
            "versions",
            "versions",
            &[
                ("tmt", versions.cli.as_str()),
                ("ops", versions.ops.as_str()),
            ],
        ));
    }
    if app.workspace_setting_entries().is_some() {
        sections.push(section(
            "settings",
            "settings",
            &[("↓", "past the last key: the workspace settings below")],
        ));
    }
    KeyHelp { sections }
}

/// The settings rows under the keys: labels, with dividers disabled.
pub(super) fn settings(app: &App) -> Vec<(String, bool)> {
    app.workspace_setting_entries()
        .unwrap_or_default()
        .into_iter()
        .map(|entry| {
            let divider = matches!(
                entry.choice,
                super::app::Choice::Rule | super::app::Choice::Heading
            );
            (entry.label, divider)
        })
        .collect()
}

pub(super) fn render(frame: &mut Frame, app: &App, body: Rect) {
    let mut help = app.help_state.borrow_mut();
    let outer = help.render(frame, model(app), &settings(app), app.look(), body);
    app.peek_areas.set(super::app::Overlay::Help, outer);
}
