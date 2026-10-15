//! Disposable dashboard policy preview. Persistence stays in the workspace lane;
//! the opening preferences remain the stale-save baseline across acquisitions.
use super::picker_surface;
use crate::{
    config::{DigestPanels, OpsPreferences, OverviewPanels, Panel, PanelSetting, Panels},
    look::Look,
};
use ratatui::{
    Frame,
    crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind},
    layout::Rect,
};
use serde_json::json;
use std::{cell::RefCell, sync::OnceLock};
use tmt_tui::components::{ListRow, PickerEvent, PickerField, PickerInput, surface};

const FILE: &str = "ops.panels-picker.xml";
const MARKUP: &str = r#"<tmt-view version="1"><tmt-modal id="panels-picker" title="Panels" placement="center" class="w-72"><tmt-scroll id="choices-body"><tmt-table id="choices" bind="$.rows"><tmt-row class="grid grid-cols-[3_1fr] gap-0"><tmt-cell bind="row.mark" token="text"/><tmt-cell bind="row.label" token="text"/></tmt-row></tmt-table></tmt-scroll><tmt-text slot="query" bind="$.query" token="muted"/><tmt-text slot="status" bind="$.status" token="blocked"/><tmt-text slot="footer" bind="$.footer" token="muted"/></tmt-modal></tmt-view>"#;

fn template() -> &'static surface::Template<()> {
    static TEMPLATE: OnceLock<surface::Template<()>> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        picker_surface::compile(FILE, MARKUP, picker_surface::schema(&["mark", "label"]))
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Draft {
    Overview(Option<OverviewPanels>),
    Digest(Option<DigestPanels>),
}
impl Draft {
    pub fn overview(&self) -> Option<OverviewPanels> {
        match self {
            Self::Overview(policy) => Some(policy.clone().unwrap_or_default()),
            Self::Digest(_) => None,
        }
    }
    pub fn digest(&self) -> Option<DigestPanels> {
        match self {
            Self::Digest(policy) => Some(policy.clone().unwrap_or_default()),
            Self::Overview(_) => None,
        }
    }
    fn rows(&self) -> Vec<(String, String, String)> {
        match self {
            Self::Overview(policy) => rows(&policy.clone().unwrap_or_default()),
            Self::Digest(policy) => rows(&policy.clone().unwrap_or_default()),
        }
    }
    fn edit(&mut self, action: Edit, selected: &str) -> Result<(), String> {
        match self {
            Self::Overview(policy) => edit(policy, action, selected),
            Self::Digest(policy) => edit(policy, action, selected),
        }
    }
    fn reset(&mut self) {
        match self {
            Self::Overview(policy) => *policy = None,
            Self::Digest(policy) => *policy = None,
        }
    }
}

fn label(key: &str) -> &'static str {
    match key {
        "summary" => "Summary counts",
        "digest" => "Digest summary",
        "subscriptions" => "Subscription quota",
        "usage" => "Usage chart",
        "monitor" => "Agent monitor",
        "exchanges" => "Recent exchanges",
        "status" => "Status",
        "chart" => "Usage chart",
        "list" => "Member list",
        "conversation" => "Conversation side pane",
        "details" => "Details side pane",
        _ => unreachable!("typed panel ID"),
    }
}
fn rows<P: Panel>(policy: &Panels<P>) -> Vec<(String, String, String)> {
    let mut rows: Vec<_> = policy
        .order()
        .iter()
        .filter(|panel| {
            P::SIDE_KEY != "conversation"
                || matches!(panel.key(), "usage" | "subscriptions" | "monitor")
        })
        .map(|panel| {
            (
                panel.key().into(),
                if policy.shown(*panel) { "✓" } else { "" }.into(),
                if P::SIDE_KEY == "conversation" && panel.key() == "usage" {
                    "Usage (fixed header)".into()
                } else if P::SIDE_KEY == "conversation" && panel.key() == "subscriptions" {
                    "Quota in Usage (fixed row)".into()
                } else if P::SIDE_KEY == "conversation" && panel.key() == "monitor" {
                    "Squads / Needs you (fixed body)".into()
                } else if panel.key() == "usage" && P::SIDE_KEY == "details" {
                    "Usage".into()
                } else {
                    label(panel.key()).into()
                },
            )
        })
        .collect();
    rows.push((
        "side".into(),
        if policy.side_shown() { "✓" } else { "" }.into(),
        label(P::SIDE_KEY).into(),
    ));
    for (id, name) in [
        ("up", "Move selected panel up"),
        ("down", "Move selected panel down"),
        ("reset", "Reset to default"),
        ("save", "Save"),
        ("cancel", "Cancel"),
    ] {
        if P::SIDE_KEY != "conversation" || !matches!(id, "up" | "down") {
            rows.push((id.into(), String::new(), name.into()));
        }
    }
    rows
}
#[derive(Clone, Copy)]
enum Edit {
    Toggle,
    Side,
    Up,
    Down,
}
fn edit<P: Panel>(
    draft: &mut Option<Panels<P>>,
    action: Edit,
    selected: &str,
) -> Result<(), String> {
    let mut policy = draft.clone().unwrap_or_default();
    match action {
        Edit::Side => policy.set_side_shown(!policy.side_shown())?,
        Edit::Toggle => {
            let panel = P::parse(selected).ok_or("Select a vertical panel first.")?;
            policy.set_shown(panel, !policy.shown(panel))?;
        }
        Edit::Up | Edit::Down => {
            if P::SIDE_KEY == "conversation" {
                return Err("Overview sections have fixed positions.".into());
            }
            let panel = P::parse(selected).ok_or("Select a vertical panel first.")?;
            let index = policy
                .order()
                .iter()
                .position(|item| *item == panel)
                .expect("admitted panel");
            let down = matches!(action, Edit::Down);
            let next = if down {
                index
                    .checked_add(1)
                    .filter(|next| *next < policy.order().len())
            } else {
                index.checked_sub(1)
            };
            if let Some(next) = next {
                for _ in 0..index.abs_diff(next) {
                    policy.move_panel(panel, down);
                }
            }
        }
    }
    *draft = Some(policy);
    Ok(())
}

pub(super) enum Input {
    Preview,
    Save,
    Cancel,
}
pub(super) struct Picker {
    pub expected: OpsPreferences,
    pub draft: Draft,
    pub notice: Option<String>,
    saving: bool,
    selected_panel: String,
    surface: RefCell<picker_surface::State>,
}
impl Picker {
    pub fn open(expected: OpsPreferences, digest: bool) -> Self {
        let draft = if digest {
            Draft::Digest(Some(
                expected
                    .digest_panels
                    .as_ref()
                    .map(PanelSetting::effective)
                    .unwrap_or_default(),
            ))
        } else {
            Draft::Overview(Some(
                expected
                    .overview_panels
                    .as_ref()
                    .map(PanelSetting::effective)
                    .unwrap_or_default(),
            ))
        };
        let rows = draft.rows();
        let selected_panel = rows[0].0.clone();
        let surface = RefCell::new(picker_surface::State::new(
            None,
            component_rows(&rows, false),
            Some(&selected_panel),
        ));
        Self {
            expected,
            draft,
            notice: None,
            saving: false,
            selected_panel,
            surface,
        }
    }
    pub fn next(&self) -> OpsPreferences {
        let mut next = self.expected.clone();
        match &self.draft {
            Draft::Overview(policy) => {
                next.overview_panels = policy.clone().map(PanelSetting::configured)
            }
            Draft::Digest(policy) => {
                next.digest_panels = policy.clone().map(PanelSetting::configured)
            }
        }
        next
    }
    pub fn input(&mut self, event: &Event) -> Option<Input> {
        if matches!(event, Event::Key(key) if key.kind == ratatui::crossterm::event::KeyEventKind::Release)
        {
            return None;
        }
        if matches!(event, Event::Key(key) if key.code == KeyCode::Esc) {
            return Some(Input::Cancel);
        }
        if self.saving
            && !matches!(event, Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left))
        {
            return None;
        }
        let direct = match event {
            Event::Key(key) if !self.saving && key.modifiers == KeyModifiers::NONE => {
                match key.code {
                    KeyCode::Char('u') => Some(Edit::Up),
                    KeyCode::Char('d') => Some(Edit::Down),
                    _ => None,
                }
            }
            _ => None,
        };
        self.notice = None;
        if let Some(action) = direct {
            self.change(action);
            return Some(Input::Preview);
        }
        let input = if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
        {
            // A click activates the painted control, including the selected row.
            // List navigation alone reports no change for that same-row click.
            let mut state = self.surface.borrow_mut();
            let frame = state.frame.as_ref()?.list.as_ref()?;
            if frame.rows != state.picker.list.rows()
                || frame.viewport != state.picker.list.scroll.viewport()
                || frame.offset != state.picker.list.scroll.offset()
                || !frame.viewport.contains((mouse.column, mouse.row).into())
            {
                return None;
            }
            let id = frame
                .geometry
                .iter()
                .rev()
                .find(|row| row.visible.contains((mouse.column, mouse.row).into()))?
                .id
                .clone();
            if frame.rows.iter().any(|row| row.id == id && row.disabled) {
                return None;
            }
            state.select(&id);
            PickerInput::Event(PickerEvent::Confirm(id))
        } else if matches!(event, Event::Key(key) if key.code == KeyCode::Char(' ') && key.modifiers == KeyModifiers::NONE)
        {
            let Event::Key(key) = event else {
                unreachable!()
            };
            self.surface.borrow_mut().input(
                &Event::Key(ratatui::crossterm::event::KeyEvent {
                    code: KeyCode::Enter,
                    ..*key
                }),
                PickerField::List,
            )?
        } else {
            self.surface.borrow_mut().input(event, PickerField::List)?
        };
        let selected = self.surface.borrow().picker.list.selected()?.to_owned();
        if self
            .draft
            .rows()
            .iter()
            .take_while(|row| row.0 != "side")
            .any(|row| row.0 == selected)
        {
            self.selected_panel = selected.clone();
        }
        match input {
            PickerInput::Event(PickerEvent::Cancel) => return Some(Input::Cancel),
            PickerInput::Event(PickerEvent::Confirm(_)) => match selected.as_str() {
                "save" => return Some(Input::Save),
                "cancel" => return Some(Input::Cancel),
                "reset" => {
                    self.draft.reset();
                    self.reconcile();
                }
                "up" => self.change(Edit::Up),
                "down" => self.change(Edit::Down),
                "side" => self.change(Edit::Side),
                _ => self.change(Edit::Toggle),
            },
            _ => {}
        }
        Some(Input::Preview)
    }
    fn change(&mut self, action: Edit) {
        if let Err(error) = self.draft.edit(action, &self.selected_panel) {
            self.notice = Some(error);
        }
        self.reconcile();
    }
    fn reconcile(&mut self) {
        self.surface
            .borrow_mut()
            .reconcile(component_rows(&self.draft.rows(), self.saving));
    }
    pub fn start_save(&mut self) {
        self.saving = true;
        self.reconcile();
    }
    pub fn save_failed(&mut self, error: String) {
        self.saving = false;
        self.notice = Some(error);
        self.reconcile();
    }
    pub fn invalidate(&self) {
        self.surface.borrow_mut().invalidate();
    }
}
fn component_rows(rows: &[(String, String, String)], saving: bool) -> Vec<ListRow> {
    rows.iter()
        .map(|(id, _, _)| ListRow {
            id: id.clone(),
            disabled: saving && id != "cancel",
        })
        .collect()
}
pub(super) fn render(frame: &mut Frame, picker: &Picker, look: Look, area: Rect) {
    let rows: Vec<_> = picker
        .draft
        .rows()
        .into_iter()
        .map(|(id, mark, label)| json!({"disabled":picker.saving && id != "cancel", "id":id,"mark":mark,"label":label}))
        .collect();
    picker.surface.borrow_mut().render(FILE, template(), json!({
        "rows": rows, "query": match picker.draft { Draft::Overview(_) => "Overview panels", Draft::Digest(_) => "Digest panels" },
        "status": picker.notice.as_deref().unwrap_or(""),
        "footer": if picker.saving { "Saving… · Cancel/Esc close" } else { "↑↓ select · Enter toggle/action · u/d move · Esc cancel" }, "notes": []
    }), frame, look, area);
}

#[cfg(test)]
mod tests;
