//! Member-scoped driver-switch review; Core owns handoff and stopping the old agent.
mod load;
mod model;
mod surface;
#[cfg(test)]
pub(in crate::board) mod tests;
use super::{
    app::{App, CopyFeedback, Effect, Request},
    host_launch::Loaded,
};
pub(in crate::board) use load::{Completed, Task, complete};
use load::{Outcome, Work};
use model::{Review, Source};
use ratatui::{
    crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    layout::Rect,
};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};
use tmt_tui::components::{Editor, ScrollState};
#[derive(Clone, Debug, PartialEq, Eq)]
enum Stage {
    Form,
    Review,
    Pending,
    Result(String),
}
/// A footer button: the first one (Cancel, Back or Close), Discard on a reopened draft, or the
/// stage's action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    First,
    Discard,
    Apply,
}
#[derive(Clone, Copy)]
enum ResultKind {
    Switched,
    Unknown,
    Refused,
    Changed,
    CopyOnly,
}
impl Stage {
    fn result(kind: ResultKind, text: impl Into<String>) -> Self {
        use tmt_cli_style::mark::Mark;
        let (mark, word) = match kind {
            ResultKind::Switched => (Mark::Done, "Switched"),
            ResultKind::Unknown => (Mark::Decision, "Unknown"),
            ResultKind::Refused => (Mark::Failed, "Refused"),
            ResultKind::Changed => (Mark::Failed, "Changed"),
            ResultKind::CopyOnly => (Mark::Decision, "Copy only"),
        };
        Self::Result(format!("{} {word} · {}", mark.symbol(), text.into()))
    }
}
pub(in crate::board) struct Controller {
    owner: u64,
    /// False while the dialog is hidden: a kept draft, or a switch running behind it.
    shown: bool,
    /// Opened again from a kept draft: Discard drops it.
    reopened: bool,
    source: Option<Source>,
    loaded: Option<Loaded>,
    stage: Stage,
    fields: [Editor; 2],
    tools: Vec<String>,
    models: BTreeMap<String, Vec<String>>,
    review: Option<Review>,
    focus: usize,
    notice: Option<String>,
    readable: Cell<bool>,
    controls: Cell<[Option<Rect>; 2]>,
    discard: Cell<Option<Rect>>,
    fields_hit: Cell<[Option<Rect>; 2]>,
    editor_width: Cell<u16>,
    scroll: RefCell<ScrollState>,
    choices: RefCell<Vec<(usize, String, Rect)>>,
}
impl Controller {
    fn finish(&mut self, kind: ResultKind, text: impl Into<String>) {
        self.stage = Stage::result(kind, text);
        self.focus = 0;
        self.notice = None;
        self.scroll
            .borrow_mut()
            .step(tmt_tui::components::Step::Top);
        self.invalidate();
    }
    fn review_from_fields(&self) -> Result<Review, String> {
        let source = self
            .source
            .as_ref()
            .ok_or("Member observation unavailable. Close and reopen.")?;
        let loaded = self
            .loaded
            .as_ref()
            .ok_or("Host templates unavailable. Close and reopen.")?;
        Review::new(
            source,
            self.fields[0].as_str(),
            self.fields[1].as_str(),
            loaded,
        )
    }
    pub fn invalidate(&self) {
        self.readable.set(false);
        self.controls.set([None, None]);
        self.discard.set(None);
        self.fields_hit.set([None, None]);
        self.choices.borrow_mut().clear();
    }
    /// The stage's buttons in order; on the form they follow its two fields (`focus` 2, 3, …).
    fn buttons(&self) -> Vec<Control> {
        match self.stage {
            Stage::Form | Stage::Review => [Control::First]
                .into_iter()
                .chain(self.reopened.then_some(Control::Discard))
                .chain([Control::Apply])
                .collect(),
            Stage::Pending => vec![],
            Stage::Result(_) => vec![Control::First],
        }
    }
    /// The focused button, if focus is not in a form field.
    fn focused(&self) -> Option<Control> {
        let at = if self.stage == Stage::Form {
            self.focus.checked_sub(2)?
        } else {
            self.focus
        };
        self.buttons().get(at).copied()
    }
    fn cycle(&mut self, back: bool) {
        if self.focus > 1 {
            return;
        }
        let options: Vec<String> = if self.focus == 0 {
            self.tools
                .iter()
                .filter(|tool| {
                    self.source
                        .as_ref()
                        .is_none_or(|s| s.driver() != tool.as_str())
                })
                .cloned()
                .collect()
        } else {
            std::iter::once(String::new())
                .chain(
                    self.models
                        .get(self.fields[0].as_str())
                        .into_iter()
                        .flatten()
                        .cloned(),
                )
                .collect()
        };
        if options.is_empty() {
            return;
        }
        let i = options
            .iter()
            .position(|v| v == self.fields[self.focus].as_str());
        let next = i.map_or(0, |i| {
            (i + if back { options.len() - 1 } else { 1 }) % options.len()
        });
        self.fields[self.focus] = Editor::from(options[next].clone());
        if self.focus == 0 {
            self.fields[1].clear();
        }
    }
}
impl App {
    fn switch_tool_reopen_hint(&self) -> String {
        self.reopen_hint(
            crate::action::OpsUiAction::SwitchTool,
            "S",
            "this switch",
            "Switch tool on an agent row",
        )
    }
    pub(in crate::board) fn switch_tool_shown(&self) -> bool {
        self.switch_tool.as_ref().is_some_and(|c| c.shown)
    }
    pub(in crate::board) fn open_switch_tool(&mut self) -> Effect {
        if self.workspace_scope() != crate::config::OpsKeyScope::Member
            || self.shell.tab != super::shell::OpsTab::Agents
            || self.workspace_selected_member().is_none()
            || self.input.is_some()
            || self.searching
            || self.modal_open() && self.menu.is_none()
        {
            return self.say(
                "Select an agent and close the current input or overlay before switching AI tool.",
            );
        }
        // A kept draft belongs to its agent's observed session; on another agent it gives way
        // to a fresh form and says so. A running switch or its result reopens from anywhere.
        let dropped = self
            .switch_tool
            .as_ref()
            .filter(|c| {
                matches!(c.stage, Stage::Form | Stage::Review)
                    && c.source.as_ref().is_none_or(|s| !s.still_selected(self))
            })
            .map(|c| {
                c.source
                    .as_ref()
                    .map_or("an agent".to_owned(), |s| s.name.clone())
            });
        if dropped.is_some() {
            self.switch_tool = None;
        }
        if let Some(c) = &mut self.switch_tool {
            c.shown = true;
            // Discard joins the row; the focused button stays the same button.
            let control = c.focused();
            c.reopened = matches!(c.stage, Stage::Form | Stage::Review);
            if let Some(control) = control
                && let Some(at) = c.buttons().iter().position(|b| *b == control)
            {
                c.focus = at + if c.stage == Stage::Form { 2 } else { 0 };
            }
            c.invalidate();
            self.menu = None;
            return Effect::None;
        }
        let source = Source::from_app(self);
        let (tools, models) = super::tool_choices::observed(&self.references);
        self.switch_tool_generation += 1;
        let task = Task {
            owner: self.switch_tool_generation,
            work: Work::Load,
        };
        self.switch_tool = Some(Controller {
            owner: task.owner,
            shown: true,
            reopened: false,
            source,
            loaded: None,
            stage: Stage::Form,
            fields: std::array::from_fn(|_| Editor::default()),
            tools,
            models,
            review: None,
            focus: 0,
            notice: None,
            readable: Cell::new(false),
            controls: Cell::new([None, None]),
            discard: Cell::new(None),
            fields_hit: Cell::new([None, None]),
            editor_width: Cell::new(40),
            scroll: RefCell::new(Default::default()),
            choices: RefCell::new(Vec::new()),
        });
        self.menu = None;
        if let Some(name) = dropped {
            self.say(format!("Dropped the kept Switch tool draft for {name}."));
        }
        Effect::SwitchTool(task)
    }
    pub(in crate::board) fn switch_tool_changed(&mut self) -> bool {
        // A hidden draft waits for its reopen, which handles a changed selection.
        let stale = self.switch_tool.as_ref().is_some_and(|c| {
            c.shown
                && matches!(c.stage, Stage::Form | Stage::Review)
                && c.source.as_ref().is_some_and(|s| !s.still_selected(self))
        });
        if stale && let Some(c) = &mut self.switch_tool {
            c.finish(
                ResultKind::Changed,
                "Selected member or session observation changed. Close and review again; nothing launched.",
            );
            c.review = None;
            return true;
        }
        false
    }
    pub(in crate::board) fn finished_switch_tool(&mut self, completed: Completed) -> Effect {
        self.switch_tool_changed();
        let reopen = self.switch_tool_reopen_hint();
        let Some(c) = &mut self.switch_tool else {
            return Effect::None;
        };
        if c.owner != completed.task.owner {
            return Effect::None;
        }
        let mut refresh = false;
        match completed.result {
            Ok(Outcome::Loaded(loaded)) if c.stage == Stage::Form => c.loaded = Some(*loaded),
            Ok(Outcome::Loaded(_)) => {}
            Ok(Outcome::Switched(text)) => {
                c.finish(ResultKind::Switched, text);
                refresh = true;
            }
            Ok(Outcome::Unknown(text)) => c.finish(ResultKind::Unknown, text),
            Err(e) => c.finish(ResultKind::Refused, e.message),
        }
        c.invalidate();
        let hidden_notice = if !c.shown {
            if let Stage::Result(text) = &c.stage {
                Some(format!(
                    "{} · {reopen}",
                    text.lines().next().unwrap_or(text)
                ))
            } else {
                None
            }
        } else {
            None
        };
        if let Some(text) = hidden_notice {
            self.set_notice(Some(text), false);
        }
        if refresh {
            Effect::Refresh
        } else {
            Effect::None
        }
    }
    /// Esc, Cancel or an outside press. A loaded form or a review hides and keeps its draft for
    /// the same key; a running switch hides and keeps going; a load or a result closes.
    pub(in crate::board) fn switch_tool_close(&mut self) -> Effect {
        let reopen = self.switch_tool_reopen_hint();
        let Some(c) = &mut self.switch_tool else {
            return Effect::None;
        };
        if c.stage == Stage::Review || c.stage == Stage::Form && c.loaded.is_some() {
            c.shown = false;
            c.invalidate();
            return Effect::None;
        }
        if c.stage != Stage::Pending {
            self.switch_tool = None;
            return Effect::None;
        }
        c.shown = false;
        c.invalidate();
        let name = c
            .source
            .as_ref()
            .map_or("selected agent", |s| s.name.as_str());
        let text = format!("◌ Switching · {name}. {reopen}");
        self.say(&text)
    }
    pub(in crate::board) fn switch_tool_event(&mut self, event: &Event) -> Option<Effect> {
        let changed = self.switch_tool_changed();
        let c = self.switch_tool.as_mut()?;
        if !c.shown {
            return None;
        }
        if changed {
            return Some(Effect::None);
        }
        if matches!(event, Event::Resize(..)) {
            c.invalidate();
            return Some(Effect::None);
        }
        let mut activate = None;
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
        {
            let point = (mouse.column, mouse.row).into();
            let choice = c
                .choices
                .borrow()
                .iter()
                .find(|(_, _, r)| r.contains(point))
                .cloned();
            if let Some((index, value, _)) = choice {
                c.fields[index] = Editor::from(value);
                c.focus = index;
                if index == 0 {
                    c.fields[1].clear();
                }
                c.invalidate();
                return Some(Effect::None);
            }
            if let Some(index) = c
                .fields_hit
                .get()
                .iter()
                .position(|r| r.is_some_and(|r| r.contains(point)))
            {
                c.focus = index;
                c.invalidate();
                return Some(Effect::None);
            }
            let [first, apply] = c.controls.get();
            activate = [
                (first, Control::First),
                (c.discard.get(), Control::Discard),
                (apply, Control::Apply),
            ]
            .into_iter()
            .find(|(area, _)| area.is_some_and(|a| a.contains(point)))
            .map(|(_, control)| control);
        }
        if let Event::Key(key) = event {
            if key.kind == KeyEventKind::Release {
                return Some(Effect::None);
            }
            if key.code == KeyCode::Esc {
                return Some(self.switch_tool_close());
            }
            if c.stage == Stage::Pending {
                return Some(Effect::None);
            }
            if key.code == KeyCode::Char('c')
                && c.stage == Stage::Review
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && let Some(review) = &c.review
            {
                return Some(Effect::Act(Request::Copy {
                    text: review.copy(),
                    program: self.view.as_ref().and_then(|v| v.clipboard.clone()),
                    feedback: CopyFeedback::Destination,
                }));
            }
            let form = c.stage == Stage::Form;
            let buttons = c.buttons();
            match key.code {
                KeyCode::Enter => {
                    if form && c.focus < 2 {
                        c.focus += 1;
                        c.invalidate();
                    } else {
                        activate = c.focused();
                    }
                }
                // ↑↓ change the field, and below Model the button row; ←→ change the value.
                KeyCode::Up if form && c.focus > 0 => {
                    c.focus = c.focus.min(2) - 1;
                    c.invalidate();
                }
                KeyCode::Down if form && c.focus < 2 => {
                    c.focus += 1;
                    c.invalidate();
                }
                KeyCode::Left | KeyCode::Right if form && c.focus < 2 => {
                    c.cycle(key.code == KeyCode::Left);
                    c.invalidate();
                }
                KeyCode::Left | KeyCode::Right if !buttons.is_empty() && c.focused().is_some() => {
                    let base = if form { 2 } else { 0 };
                    let n = buttons.len();
                    let at = c.focus - base;
                    c.focus = base + (at + if key.code == KeyCode::Left { n - 1 } else { 1 }) % n;
                    c.invalidate();
                }
                _ if c.stage == Stage::Form && c.focus < 2 => {
                    let before = c.fields[0].to_string();
                    c.fields[c.focus].key(*key, c.editor_width.get(), 1024);
                    if c.fields[0].as_str() != before {
                        c.fields[1].clear();
                    }
                    c.invalidate();
                }
                _ => {}
            }
        }
        if let Event::Paste(text) = event
            && c.stage == Stage::Form
            && c.focus < 2
            && !text.chars().any(char::is_control)
        {
            c.fields[c.focus].insert(text, 1024);
            if c.focus == 0 {
                c.fields[1].clear();
            }
            c.invalidate();
        }
        if c.stage != Stage::Form && c.scroll.borrow_mut().input(event) {
            c.readable.set(false);
        }
        // Only a painted Discard drops the draft, by key or click.
        if activate == Some(Control::Discard) {
            if c.discard.get().is_some() {
                self.switch_tool = None;
            }
            return Some(Effect::None);
        }
        if let Some(control) = activate {
            if control == Control::First {
                // Back from the review to the form; Cancel hides the form; Close ends a result.
                if c.stage == Stage::Review {
                    c.stage = Stage::Form;
                    c.focus = 0;
                    c.invalidate();
                    return Some(Effect::None);
                }
                return Some(self.switch_tool_close());
            }
            if !c.readable.get() {
                return Some(Effect::None);
            }
            match c.stage {
                Stage::Form => {
                    match c.review_from_fields() {
                        Ok(r) => {
                            c.review = Some(r);
                            c.stage = Stage::Review;
                            c.focus = 0;
                            c.notice = None;
                            c.scroll.borrow_mut().step(tmt_tui::components::Step::Top);
                        }
                        Err(e) => c.notice = Some(e),
                    }
                    c.invalidate();
                }
                Stage::Review => {
                    if let Some(review) = c.review.clone() {
                        if review.outer.is_none() {
                            c.finish(
                                ResultKind::CopyOnly,
                                "Nothing ran. Run the command in a suitable host pane.",
                            );
                            c.invalidate();
                            return Some(Effect::Act(Request::Copy {
                                text: review.copy(),
                                program: self.view.as_ref().and_then(|v| v.clipboard.clone()),
                                feedback: CopyFeedback::Destination,
                            }));
                        }
                        c.stage = Stage::Pending;
                        c.notice = None;
                        c.invalidate();
                        return Some(Effect::SwitchTool(Task {
                            owner: c.owner,
                            work: Work::Launch(Box::new(review)),
                        }));
                    }
                }
                _ => {}
            }
        }
        Some(Effect::None)
    }
}
