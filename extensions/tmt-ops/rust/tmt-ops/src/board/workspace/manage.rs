//! Staged workspace forms share App's Editor. Review freezes exact IDs before
//! the action lane can write; a result never automatically resubmits a partial.
use super::{OpsMember, OpsObservation, OpsTarget};
use crate::board::{
    app::{App, Choice, Compose, Effect, Menu, MenuEntry, Request},
    command_line::command_line,
    shell::OpsTab,
};
use crate::membership::workspace::{Operation, RoomTarget};
use ratatui::layout::Rect;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
};
use tmt_tui::components::ScrollState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Create,
    Add,
    Lead,
    RenameSquad,
    RenameMember,
    RemoveMember,
    Remove,
}
impl Kind {
    pub(in crate::board) fn label(self) -> &'static str {
        match self {
            Self::Create => "Create squad",
            Self::Add => "Add member",
            Self::Lead => "Set lead",
            Self::RenameSquad => "Rename squad",
            Self::RenameMember => "Rename",
            Self::RemoveMember => "Close and delete",
            Self::Remove => "Leave squad",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    Fields,
    Review,
    Pending,
    Result(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Change {
    Membership(Operation),
    RenameMember(super::rename_member::Request),
    CloseMember(super::close_member::Request),
}
impl Change {
    pub(super) fn confirm_label(&self) -> &'static str {
        match self {
            Self::RenameMember(_) => "Rename",
            Self::CloseMember(_) => "Close and delete",
            Self::Membership(operation) => match operation {
                Operation::Create { .. } => "Create squad",
                Operation::Add { .. } => "Add member",
                Operation::Lead {
                    member: Some(_), ..
                } => "Set lead",
                Operation::Lead { member: None, .. } => "Clear lead",
                Operation::RenameSquad { .. } => "Rename",
                Operation::Remove { .. } => "Leave squad",
                Operation::Assign {
                    move_member: true, ..
                } => "Move",
                Operation::Assign {
                    move_member: false, ..
                } => "Add",
            },
        }
    }
}

pub struct Draft {
    pub kind: Kind,
    pub stage: Stage,
    pub room: Option<RoomTarget>,
    pub member: Option<OpsMember>,
    pub identities: BTreeMap<String, OpsMember>,
    pub values: Vec<String>,
    pub operation: Option<Change>,
    pub shown: bool,
    pub timer_impact: Option<usize>,
    pub scroll: RefCell<ScrollState>,
    /// Painted Cancel / Close, apply and Discard, indexed by `ReviewButton`.
    pub controls: Cell<[Option<Rect>; 3]>,
    /// The current field's unsent text while the form is hidden; the editor holds it otherwise.
    pub typed: String,
    /// Opened again from a kept draft: its title names the target and Discard drops it.
    pub reopened: bool,
    /// The Review stage's Cancel / apply cursor (`review_start`).
    pub review: tmt_tui::components::DialogState,
    /// The Fields stage's chosen button: ↓ on the editor's last line reaches Cancel, ←→ Discard
    /// when reopened, ↑ goes back to the editor (`None`).
    pub fields_button: Option<ReviewButton>,
    pub close: Option<super::close_member::Close>,
}
/// Review of a removal starts on Cancel, so a stray Enter never takes an agent out of a squad;
/// any other review starts on its apply button, so Enter applies what was just reviewed.
pub(in crate::board) fn review_start(
    kind: Kind,
    reopened: bool,
) -> tmt_tui::components::DialogState {
    let destructive = matches!(kind, Kind::Remove | Kind::RemoveMember);
    let apply = review_buttons(reopened)
        .iter()
        .position(|button| *button == ReviewButton::Apply)
        .unwrap_or_default();
    tmt_tui::components::DialogState {
        focus: tmt_tui::components::Focus::Buttons(if destructive { 0 } else { apply }),
    }
}
/// A form's buttons, also the slots of `Draft::controls`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::board) enum ReviewButton {
    Cancel,
    Apply,
    Discard,
}
/// The Review row: Discard sits between Cancel and the apply button on a reopened form.
pub(in crate::board) fn review_buttons(reopened: bool) -> &'static [ReviewButton] {
    if reopened {
        &[
            ReviewButton::Cancel,
            ReviewButton::Discard,
            ReviewButton::Apply,
        ]
    } else {
        &[ReviewButton::Cancel, ReviewButton::Apply]
    }
}
impl Draft {
    fn validate_field(&self, text: &str) -> Result<(), String> {
        let index = self.values.len();
        match (self.kind, index) {
            (Kind::Create, 0) | (Kind::RenameSquad, 0) if !crate::squad::valid_name(text) => Err(
                "Choose a lowercase squad name of 1–24 bytes using letters, digits and hyphens."
                    .into(),
            ),
            (Kind::Create, 1) | (Kind::Lead, 0) if !text.is_empty() => {
                self.identity(text).map(|_| ())
            }
            (Kind::Create, index) if index >= 2 && !text.is_empty() => {
                self.identity(text).map(|_| ())
            }
            (Kind::Add, 0) => self.identity(text).map(|_| ()),
            _ => Ok(()),
        }
    }
    pub fn prompts(&self) -> &'static [&'static str] {
        match self.kind {
            Kind::Create => &[
                "Squad name",
                "Lead (optional tmt/agent)",
                "Member (tmt/agent; blank reviews the chosen members)",
            ],
            Kind::Add => &["Member (tmt/agent)"],
            Kind::Lead => &["Lead (tmt/agent; blank clears leadership only)"],
            Kind::RenameSquad => &["New squad name"],
            Kind::RenameMember => &["New agent name"],
            Kind::Remove | Kind::RemoveMember => &[],
        }
    }
    fn identity(&self, text: &str) -> Result<String, String> {
        // `tmt/name` as the picker inserts it; a typed `@name` or a bare UUID also names one.
        let text = text.trim();
        let text = text
            .strip_prefix(crate::board::mentions::TRIGGER)
            .or_else(|| text.strip_prefix('@'))
            .unwrap_or(text);
        let matches: Vec<_> = self
            .identities
            .values()
            .filter(|member| member.id == text || member.name == text)
            .collect();
        if matches.len() != 1 {
            return Err("Choose one known identity by its tmt/name or UUID.".into());
        }
        Ok(matches[0].id.clone())
    }
    fn operation(&self) -> Result<Change, String> {
        if self.kind == Kind::RenameMember {
            let member = self
                .member
                .as_ref()
                .ok_or_else(|| "Select a named member.".to_owned())?;
            return Ok(Change::RenameMember(super::rename_member::Request {
                identity_id: member.id.clone(),
                expected_name: member.name.clone(),
                name: self.values.first().cloned().unwrap_or_default(),
            }));
        }
        let value = |index| self.values.get(index).map(String::as_str).unwrap_or("");
        let room = || {
            self.room
                .clone()
                .ok_or_else(|| "A complete squad snapshot is required.".to_owned())
        };
        let member = || {
            self.member
                .as_ref()
                .map(|member| member.id.clone())
                .ok_or_else(|| "Select a named member.".to_owned())
        };
        let optional_identity = |text: &str| {
            if text.trim().is_empty() {
                Ok(None)
            } else {
                self.identity(text).map(Some)
            }
        };
        Ok(Change::Membership(match self.kind {
            Kind::Create => {
                if !crate::squad::valid_name(value(0)) {
                    return Err(
                        "Squad name must use lowercase letters, digits and hyphens (1–24 bytes)."
                            .into(),
                    );
                }
                let mut members = Vec::new();
                for text in self.values.iter().skip(2).filter(|text| !text.is_empty()) {
                    let id = self.identity(text)?;
                    if !members.contains(&id) {
                        members.push(id);
                    }
                }
                Operation::Create {
                    name: value(0).into(),
                    lead: optional_identity(value(1))?,
                    members,
                }
            }
            Kind::Add => Operation::Add {
                room: room()?,
                member: self.identity(value(0))?,
            },
            Kind::Lead => Operation::Lead {
                room: room()?,
                member: optional_identity(value(0))?,
            },
            Kind::RenameSquad => {
                if !crate::squad::valid_name(value(0)) {
                    return Err("Choose a valid lowercase squad name.".into());
                }
                Operation::RenameSquad {
                    room: room()?,
                    name: value(0).into(),
                }
            }
            Kind::RenameMember => unreachable!("handled above"),
            Kind::RemoveMember => {
                unreachable!("close/delete uses the typed lifecycle request")
            }
            Kind::Remove => Operation::Remove {
                room: room()?,
                member: member()?,
            },
        }))
    }
    pub fn title(&self) -> String {
        let name = self
            .room
            .as_ref()
            .map_or("Common pool", |room| room.name.as_str());
        match self.kind {
            Kind::Create => "Create squad".into(),
            Kind::Add => format!("Add member · {name}"),
            Kind::Lead => format!("Set/clear lead · {name}"),
            Kind::RenameSquad => format!("Rename squad · {name}"),
            Kind::RenameMember | Kind::RemoveMember => {
                let label = if self.kind == Kind::RenameMember {
                    "Rename agent"
                } else {
                    "Close and delete"
                };
                // A reopened draft names the agent it was opened for.
                match self.member.as_ref().filter(|_| self.reopened) {
                    Some(member) => format!("{label} · {}", member.name),
                    None => label.into(),
                }
            }
            Kind::Remove => format!("Leave squad · {name}"),
        }
    }
    pub fn review_command(&self) -> Option<String> {
        let Change::RenameMember(request) = self.operation.as_ref()? else {
            return None;
        };
        if [&request.expected_name, &request.name]
            .iter()
            .any(|word| word.is_empty() || word.chars().any(char::is_control))
        {
            return None;
        }
        // TODO(#4579): display the UUID once Core mv accepts it; execution keeps its CAS.
        let mut words = vec!["tmt", "mv"];
        if request.expected_name.starts_with('-') || request.name.starts_with('-') {
            words.push("--");
        }
        words.extend([request.expected_name.as_str(), request.name.as_str()]);
        Some(command_line(&words))
    }
    pub fn review(&self) -> String {
        let identity = |id: &str| {
            self.identities.get(id).map_or_else(
                || id.to_owned(),
                |member| format!("{} · {}", member.name, id),
            )
        };
        let room =
            |room: &RoomTarget| format!("{} · {} · revision {}", room.name, room.id, room.revision);
        if matches!(self.operation, Some(Change::RenameMember(_))) {
            return format!(
                "{}\n\nStops if the name changed first.",
                self.review_command()
                    .unwrap_or_else(|| "Unavailable: name cannot be copied safely.".into())
            );
        }
        let mut lines = vec![];
        let operation = self.operation.as_ref().and_then(|change| match change {
            Change::Membership(operation) => Some(operation),
            Change::RenameMember(_) | Change::CloseMember(_) => None,
        });
        match operation {
            Some(Operation::Create {
                name,
                lead,
                members,
            }) => {
                lines.push(format!("New squad: {name}"));
                lines.push(format!(
                    "Lead: {}",
                    lead.as_deref()
                        .map(identity)
                        .unwrap_or_else(|| "not set".into())
                ));
                lines.push(format!("Members: {}", members.len()));
                lines.extend(members.iter().map(|id| identity(id)));
            }
            Some(Operation::Add {
                room: target,
                member,
            }) => {
                lines.push(format!("Squad: {}", room(target)));
                lines.push(format!("Add member: {}", identity(member)));
                lines.push("Existing memberships remain.".into());
            }
            Some(Operation::Lead {
                room: target,
                member,
            }) => {
                lines.push(format!("Squad: {}", room(target)));
                lines.push(format!(
                    "Lead: {}",
                    member
                        .as_deref()
                        .map(identity)
                        .unwrap_or_else(|| "clear leadership only".into())
                ));
            }
            Some(Operation::RenameSquad { room: target, name }) => {
                lines.push(format!("Squad: {}", room(target)));
                lines.push(format!("New name: {name}"));
                lines.push("Room UUID, timer IDs and pause state stay the same. A partial rename provides an explicit repair command.".into());
            }
            Some(Operation::Remove {
                room: target,
                member,
            }) => {
                lines.push(format!("Remove membership: {}", identity(member)));
                lines.push(format!("From squad: {}", room(target)));
                lines.push("Other memberships, identity, process, history and session draft remain. Removing this squad's lead clears its leadership.".into());
                self.timer_review(&mut lines);
            }
            Some(Operation::Assign {
                source,
                destination,
                member,
                move_member,
            }) => {
                let name = self
                    .identities
                    .get(member)
                    .map_or(member.as_str(), |member| member.name.as_str());
                let source_name = source
                    .as_ref()
                    .map_or("Common pool", |room| room.name.as_str());
                lines.push(format!("{name}: {source_name} → {}", destination.name));
                lines.push(String::new());
                lines.push(command_line(&[
                    "tmt",
                    "ops",
                    "sq",
                    "add",
                    "--squad",
                    &destination.name,
                    "--",
                    member,
                ]));
                if *move_member && let Some(source) = source {
                    lines.push(command_line(&[
                        "tmt",
                        "ops",
                        "sq",
                        "rm",
                        "--squad",
                        &source.name,
                        "--",
                        member,
                    ]));
                }
                lines.push(String::new());
                lines.push(
                    if *move_member {
                        "History, drafts and timers stay; source timers pause until resumed."
                    } else {
                        "History, drafts and timers stay."
                    }
                    .into(),
                );
            }
            None => {}
        }
        lines.join("\n")
    }
    fn timer_review(&self, lines: &mut Vec<String>) {
        lines.push(format!(
            "Source timers to pause: {}",
            self.timer_impact.map_or_else(
                || "unknown; refresh to inspect the timer list".into(),
                |count| count.to_string()
            )
        ));
        lines.push("Source timers stay in their original squad. Re-adding or reassigning the owner never resumes them; only explicit Resume does.".into());
    }
    pub fn invalidate(&self) {
        if let Some(close) = &self.close {
            close.invalidate();
        }
        self.controls.set([None; 3]);
        let mut scroll = self.scroll.borrow_mut();
        let content = scroll.content();
        scroll.update(Rect::default(), content, None);
    }
}
pub(super) fn room(app: &App, id: &str) -> Option<RoomTarget> {
    let snapshot = app.workspace_snapshot()?;
    snapshot.roster.squads.iter().find(|squad| {
        squad.room_id == id && squad.observation == super::OpsObservation::Complete
    })?;
    let row = app
        .workspace_snapshot()?
        .rows
        .get(&OpsTarget::Squad(id.into()))?;
    Some(RoomTarget {
        id: id.into(),
        name: row["name"].as_str()?.into(),
        revision: row["revision"].as_u64()?,
    })
}

fn flat_member_context(tab: OpsTab, flat_agents: bool, room_id: Option<&str>) -> bool {
    tab != OpsTab::Agents || flat_agents || room_id.is_none()
}

fn context_text(text: &str) -> String {
    tmt_cli_style::table::escape(&crate::board::notes::sanitize(text))
        .chars()
        .take(160)
        .collect()
}

fn incomplete_context(app: &App, room_id: Option<&str>) -> String {
    let Some(home) = app.view.as_ref().and_then(|view| view.home.as_ref()) else {
        return "Workspace evidence is unavailable; refresh first.".into();
    };
    let squad = room_id
        .and_then(|id| {
            home.workspace
                .roster
                .squads
                .iter()
                .find(|squad| squad.room_id == id)
        })
        .or_else(|| {
            room_id
                .is_none()
                .then(|| {
                    home.workspace
                        .roster
                        .squads
                        .iter()
                        .find(|squad| squad.observation != OpsObservation::Complete)
                })
                .flatten()
        });
    if let Some(failure) = home.failures.iter().find(|failure| {
        room_id.is_none()
            || squad.is_some_and(|squad| failure["squad"].as_str() == Some(&squad.name))
    }) {
        let source = failure["squad"].as_str().unwrap_or("workspace");
        let reason = failure["error"]["message"]
            .as_str()
            .unwrap_or("source data is unavailable");
        return format!(
            "Couldn't read {}: {}; refresh first.",
            context_text(source),
            context_text(reason)
        );
    }
    if let Some(squad) = squad {
        let cause = match squad.observation {
            OpsObservation::Complete => "squad details are incomplete",
            OpsObservation::Partial => "squad roster is partial",
            OpsObservation::Stale => "squad roster is stale",
            OpsObservation::Unavailable => "squad data is unavailable",
        };
        return format!("{} {cause}; refresh first.", context_text(&squad.name));
    }
    match home.workspace.roster.observation {
        OpsObservation::Complete => "Squad memberships are incomplete; refresh first.",
        OpsObservation::Partial => "The identity roster is partial; refresh first.",
        OpsObservation::Stale => "The identity roster is stale; refresh first.",
        OpsObservation::Unavailable => "The identity roster is unavailable; refresh first.",
    }
    .into()
}

impl App {
    pub(in crate::board) fn workspace_reference_field(&self) -> bool {
        self.workspace_form.as_ref().is_some_and(|draft| {
            draft.stage == Stage::Fields
                && matches!(
                    (draft.kind, draft.values.len()),
                    (Kind::Create, 1..) | (Kind::Add | Kind::Lead, 0)
                )
        })
    }
    pub(in crate::board) fn workspace_form_event(
        &mut self,
        event: &ratatui::crossterm::event::Event,
    ) -> Option<Effect> {
        use ratatui::crossterm::event::{
            Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind,
        };
        let draft = self.workspace_form.as_ref()?;
        if draft.close.is_some() {
            return self.close_member_event(event);
        }
        let editing = draft.stage == Stage::Fields;
        if matches!(event, Event::Key(key) if key.code == KeyCode::Esc) {
            self.workspace_close();
            self.input = None;
            return Some(Effect::None);
        }
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
        {
            let at = (mouse.column, mouse.row).into();
            let controls = draft.controls.get();
            if controls[ReviewButton::Cancel as usize].is_some_and(|area| area.contains(at)) {
                self.workspace_close();
                return Some(Effect::None);
            }
            if controls[ReviewButton::Discard as usize].is_some_and(|area| area.contains(at)) {
                self.workspace_discard();
                return Some(Effect::None);
            }
        }
        match event {
            Event::Key(key)
                if !editing
                    && key.code == KeyCode::Char('c')
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && draft.review_command().is_some()
                    && draft.stage == Stage::Review =>
            {
                Some(self.workspace_copy_command())
            }
            Event::Key(key) if editing && draft.fields_button.is_some() => {
                let reopened = draft.reopened;
                let draft = self.workspace_form.as_mut()?;
                match key.code {
                    KeyCode::Up => draft.fields_button = None,
                    KeyCode::Left | KeyCode::Right if reopened => {
                        draft.fields_button = Some(match draft.fields_button {
                            Some(ReviewButton::Cancel) => ReviewButton::Discard,
                            _ => ReviewButton::Cancel,
                        });
                    }
                    KeyCode::Enter => {
                        if draft.fields_button == Some(ReviewButton::Discard) {
                            self.workspace_discard();
                        } else {
                            self.workspace_close();
                            self.input = None;
                        }
                    }
                    _ => {}
                }
                Some(Effect::None)
            }
            Event::Key(key) if editing => {
                self.update_mentions();
                let effect = match self.mention_key(*key) {
                    Some(effect) => effect,
                    None if key.code == KeyCode::Down
                        && key.modifiers.is_empty()
                        && self.input_on_last_line() =>
                    {
                        // ↑↓ change rows: past the field's last line is the button row.
                        self.workspace_form.as_mut()?.fields_button = Some(ReviewButton::Cancel);
                        Effect::None
                    }
                    None => self.input_key(*key),
                };
                self.update_mentions();
                Some(effect)
            }
            Event::Key(key) if draft.stage == Stage::Review => {
                use tmt_tui::components::{Button, DialogInput, DialogKind};
                let controls = draft.controls.get();
                let order = review_buttons(draft.reopened);
                let buttons: Vec<_> = order
                    .iter()
                    .map(|slot| Button::secondary("").enabled(controls[*slot as usize].is_some()))
                    .collect();
                let draft = self.workspace_form.as_mut()?;
                match draft.review.key(DialogKind::Confirm, key, &[], &buttons) {
                    DialogInput::Cancel => {
                        self.workspace_close();
                        Some(Effect::None)
                    }
                    DialogInput::Choose(index) => match order[index] {
                        ReviewButton::Cancel => {
                            self.workspace_close();
                            Some(Effect::None)
                        }
                        ReviewButton::Discard => {
                            self.workspace_discard();
                            Some(Effect::None)
                        }
                        ReviewButton::Apply => Some(self.workspace_confirm()),
                    },
                    DialogInput::Moved => Some(Effect::None),
                    _ => {
                        draft.scroll.borrow_mut().input(event);
                        Some(Effect::None)
                    }
                }
            }
            // Pending and Result read with ↑↓ and the page keys; Enter chooses their one button.
            Event::Key(key) if key.code == KeyCode::Enter && !editing => {
                self.workspace_close();
                Some(Effect::None)
            }
            Event::Mouse(mouse) if editing => {
                if self.mention_mouse(*mouse) {
                    return Some(Effect::None);
                }
                if let Some((area, skip)) = self.input_viewport.get()
                    && area.contains((mouse.column, mouse.row).into())
                    && matches!(
                        mouse.kind,
                        MouseEventKind::Down(MouseButton::Left)
                            | MouseEventKind::Drag(MouseButton::Left)
                    )
                    && let Some(input) = self.input.as_mut()
                {
                    if let Some(draft) = self.workspace_form.as_mut() {
                        draft.fields_button = None;
                    }
                    input.text.place(
                        area.width,
                        skip + usize::from(mouse.row - area.y),
                        usize::from(mouse.column - area.x),
                        matches!(mouse.kind, MouseEventKind::Drag(_))
                            || mouse.modifiers.contains(KeyModifiers::SHIFT),
                    );
                    self.update_mentions();
                }
                Some(Effect::None)
            }
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                let at = (mouse.column, mouse.row).into();
                if draft.controls.get()[ReviewButton::Apply as usize]
                    .is_some_and(|area| area.contains(at))
                {
                    return Some(self.workspace_confirm());
                }
                Some(Effect::None)
            }
            _ if !editing => draft
                .scroll
                .borrow_mut()
                .input(event)
                .then_some(Effect::None),
            _ => None,
        }
    }
    /// Esc, Cancel or an outside press: the form hides with what it holds, so the same key
    /// reopens it at its stage, with the field's typed text. A result has nothing left to keep.
    pub(in crate::board) fn workspace_close(&mut self) {
        let typed = self.input.as_ref().map(|input| input.text.to_string());
        let Some(draft) = &mut self.workspace_form else {
            return;
        };
        if matches!(draft.stage, Stage::Result(_)) {
            self.workspace_form = None;
            return;
        }
        if draft.stage == Stage::Fields {
            draft.typed = typed.unwrap_or_default();
            self.input = None;
        }
        draft.shown = false;
        draft.invalidate();
    }
    /// The editor's cursor is on its last line, so ↓ leaves the field for the buttons.
    fn input_on_last_line(&self) -> bool {
        let width = self.input_viewport.get().map_or(80, |(area, _)| area.width);
        self.input.as_ref().is_none_or(|input| {
            let layout = input.text.layout(width);
            layout.cursor.0 + 1 >= layout.lines.len()
        })
    }
    /// Drops a kept form for good; only an explicit Discard does this.
    pub(in crate::board) fn workspace_discard(&mut self) {
        if self
            .workspace_form
            .as_ref()
            .is_some_and(|draft| draft.stage != Stage::Pending)
        {
            self.workspace_form = None;
            self.input = None;
        }
    }
    pub(in crate::board) fn workspace_begin(&mut self, kind: Kind) -> Effect {
        self.workspace_begin_in_room(kind, None)
    }
    pub(in crate::board) fn workspace_choose_room(
        &mut self,
        kind: Kind,
        identity_id: &str,
        room_id: &str,
    ) -> Effect {
        self.workspace_begin_in_room(kind, Some((identity_id, room_id)))
    }
    fn workspace_begin_in_room(&mut self, kind: Kind, chosen: Option<(&str, &str)>) -> Effect {
        // Member actions stay in the Agents list, also when they would reopen a kept draft; a
        // submitted one stays reachable from anywhere so its result is never lost.
        let submitted = self.workspace_form.as_ref().is_some_and(|draft| {
            draft.kind == kind
                && (draft.stage == Stage::Pending
                    || (kind != Kind::RemoveMember && matches!(draft.stage, Stage::Result(_))))
        });
        if !submitted
            && matches!(kind, Kind::RenameMember | Kind::RemoveMember)
            && (self.loading()
                || self.shell.tab != super::super::shell::OpsTab::Agents
                || self.workspace_scope() != crate::config::OpsKeyScope::Member)
        {
            return self.say("Select a visible agent in the Agents list for this action.");
        }
        if let Some(draft) = &mut self.workspace_form
            && (!draft.shown || draft.stage == Stage::Pending)
        {
            if kind == draft.kind && !(kind == Kind::RemoveMember && draft.stage != Stage::Pending)
            {
                draft.shown = true;
                draft.reopened = matches!(draft.stage, Stage::Fields | Stage::Review);
                draft.fields_button = None;
                if draft.stage == Stage::Review {
                    draft.review = review_start(draft.kind, draft.reopened);
                }
                draft.invalidate();
                if draft.stage == Stage::Fields {
                    let typed = std::mem::take(&mut draft.typed);
                    let effect = self.workspace_form_next();
                    if let Some(input) = &mut self.input {
                        input.text = typed.into();
                    }
                    return effect;
                }
                return Effect::None;
            }
            if draft.stage == Stage::Pending {
                let operation = draft.kind.label();
                return self.say(format!(
                    "{operation} is still pending. Open it from Actions to inspect its result."
                ));
            }
        }
        let Some(snapshot) = self.workspace_snapshot() else {
            return self.say("The workspace has not loaded.");
        };
        let (mut selected_room, mut member) = match self.workspace.selected.as_ref() {
            Some(OpsTarget::Squad(id)) => (room(self, id), None),
            Some(OpsTarget::Member {
                room_id,
                identity_id,
            }) => (
                room_id.as_ref().and_then(|id| room(self, id)),
                snapshot.roster.members.get(identity_id).cloned(),
            ),
            _ => (None, None),
        };
        if kind != Kind::Create && !self.workspace_selection_visible() {
            return self.say("Select a visible target explicitly.");
        }
        let needs_room = matches!(
            kind,
            Kind::Add | Kind::Lead | Kind::RenameSquad | Kind::Remove
        );
        if matches!(kind, Kind::Remove | Kind::RenameMember | Kind::RemoveMember)
            && member.is_none()
        {
            return self.say("Select a named member first.");
        }
        let flat = matches!(
            self.workspace.selected.as_ref(),
            Some(OpsTarget::Member { room_id, .. })
                if flat_member_context(self.shell.tab, self.flat_agents(), room_id.as_deref())
        );
        if let Some((identity_id, _)) = chosen
            && (!flat
                || member
                    .as_ref()
                    .is_none_or(|member| member.id != identity_id))
        {
            return self.say("The selected agent changed; reopen the squad picker.");
        }
        if needs_room && flat {
            if !snapshot.roster.complete() {
                return self.say(incomplete_context(self, None));
            }
            let Some(member) = member.as_ref() else {
                return self.say("Select a named member first.");
            };
            let name: String = context_text(&member.name).chars().take(64).collect();
            let mut memberships = Vec::new();
            for squad in &snapshot.roster.squads {
                if squad.members.contains(&member.id) {
                    let Some(target) = room(self, &squad.room_id) else {
                        return self.say(incomplete_context(self, Some(&squad.room_id)));
                    };
                    memberships.push(target);
                }
            }
            memberships.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
            if let Some((_, room_id)) = chosen {
                selected_room = memberships.into_iter().find(|room| room.id == room_id);
                if selected_room.is_none() {
                    return self.say("That squad membership changed; reopen the squad picker.");
                }
            } else {
                match memberships.len() {
                    0 => {
                        return self.say(if kind == Kind::Remove {
                            format!("{name} is not in a squad.")
                        } else {
                            format!(
                                "{name} is not in a squad; select a squad for {}.",
                                kind.label()
                            )
                        });
                    }
                    1 => selected_room = memberships.pop(),
                    _ => {
                        let title = if kind == Kind::Remove {
                            format!("Remove {name} from:")
                        } else {
                            format!("{} · {name} · choose squad", kind.label())
                        };
                        let identity_id = member.id.clone();
                        self.menu = Some(Menu {
                            link: None,
                            prefill: String::new(),
                            title,
                            entries: memberships
                                .into_iter()
                                .map(|room| MenuEntry {
                                    key: String::new(),
                                    label: context_text(&room.name),
                                    choice: Choice::WorkspaceRoom {
                                        kind,
                                        identity_id: identity_id.clone(),
                                        room_id: room.id,
                                    },
                                })
                                .collect(),
                            selected: 0,
                            anchor: None,
                            surface: Default::default(),
                        });
                        return Effect::None;
                    }
                }
            }
        } else if needs_room && selected_room.is_none() {
            return self.say(match self.workspace.selected.as_ref() {
                Some(OpsTarget::Squad(room_id))
                | Some(OpsTarget::Member {
                    room_id: Some(room_id),
                    ..
                }) => incomplete_context(self, Some(room_id)),
                _ => "Select a visible squad for this operation.".into(),
            });
        }
        if matches!(kind, Kind::RenameMember | Kind::RemoveMember) {
            let member = member.as_mut().expect("selected member checked above");
            // Global presence and squad rows can come from different reads.
            // Freeze the literal name displayed on this exact selected UUID.
            let Some(name) = self
                .selected_row()
                .filter(|row| row["id"].as_str() == Some(member.id.as_str()))
                .and_then(|row| row["name"].as_str())
            else {
                return self.say("The selected agent name is unavailable; refresh before Rename.");
            };
            member.name = name.to_owned();
        }
        let timer_impact = if self.cron.failure.is_none() {
            self.cron.cron.as_ref().map(|cron| {
                cron.jobs
                    .iter()
                    .filter(|job| {
                        selected_room
                            .as_ref()
                            .is_some_and(|room| job.job.room_id == room.id)
                            && member.as_ref().is_some_and(|member| {
                                job.job.owner_id.as_deref() == Some(&member.id)
                            })
                    })
                    .count()
            })
        } else {
            None
        };
        // One kept draft at a time: a different form replaces it, and says so.
        let dropped = self
            .workspace_form
            .as_ref()
            .filter(|draft| !draft.shown && matches!(draft.stage, Stage::Fields | Stage::Review))
            .map(|draft| draft.kind.label());
        self.workspace_form = Some(Draft {
            kind,
            stage: Stage::Fields,
            room: selected_room,
            member,
            identities: snapshot.roster.members.clone(),
            values: Vec::new(),
            operation: None,
            shown: true,
            timer_impact,
            scroll: Default::default(),
            controls: Cell::new([None; 3]),
            typed: String::new(),
            reopened: false,
            review: review_start(kind, false),
            fields_button: None,
            close: (kind == Kind::RemoveMember).then(|| {
                super::close_member::Close::new(self.view.as_ref().and_then(|view| view.me.clone()))
            }),
        });
        let effect = self.workspace_form_next();
        if let Some(label) = dropped {
            self.say(format!("Dropped the kept {label} draft."));
        }
        effect
    }
    fn workspace_form_next(&mut self) -> Effect {
        let draft = self.workspace_form.as_ref().expect("opened workspace form");
        let prompt = if draft.kind == Kind::Create
            && draft.values.len() >= 2
            && !draft
                .values
                .last()
                .is_some_and(|value| value.is_empty() && draft.values.len() > 2)
        {
            draft.prompts().get(2)
        } else {
            draft.prompts().get(draft.values.len())
        };
        if let Some(prompt) = prompt {
            return self.ask(
                format!("{} · {prompt}", draft.title()),
                Compose::Workspace,
                String::new(),
            );
        }
        if draft.kind == Kind::RemoveMember {
            let draft = self.workspace_form.as_mut().unwrap();
            draft.stage = Stage::Review;
            draft.review = review_start(draft.kind, draft.reopened);
            draft.invalidate();
            return Effect::None;
        }
        let operation = match draft.operation() {
            Ok(operation) => operation,
            Err(error) => {
                self.workspace_form.as_mut().unwrap().stage =
                    Stage::Result(format!("Not applied: {error}"));
                return self.say(error);
            }
        };
        let draft = self.workspace_form.as_mut().unwrap();
        draft.operation = Some(operation);
        draft.stage = Stage::Review;
        draft.review = review_start(draft.kind, draft.reopened);
        draft.invalidate();
        *draft.scroll.borrow_mut() = Default::default();
        Effect::None
    }
    pub(in crate::board) fn workspace_submit_field(&mut self, text: String) -> Effect {
        let Some(draft) = self
            .workspace_form
            .as_mut()
            .filter(|draft| draft.stage == Stage::Fields)
        else {
            return self.say("This workspace form is no longer editable.");
        };
        if let Err(error) = draft.validate_field(text.trim()) {
            self.workspace_form_next();
            if let Some(input) = &mut self.input {
                input.text = text.into();
            }
            return self.say(error);
        }
        draft.values.push(if draft.kind == Kind::RenameMember {
            text
        } else {
            text.trim().into()
        });
        self.workspace_form_next()
    }
    fn workspace_copy_command(&self) -> Effect {
        let Some(text) = self
            .workspace_form
            .as_ref()
            .filter(|draft| draft.stage == Stage::Review)
            .and_then(Draft::review_command)
        else {
            return Effect::None;
        };
        Effect::Act(Request::Copy {
            text,
            program: self.view.as_ref().and_then(|view| view.clipboard.clone()),
            feedback: crate::board::app::CopyFeedback::Destination,
        })
    }
    pub(in crate::board) fn workspace_confirm(&mut self) -> Effect {
        if self
            .workspace_form
            .as_ref()
            .is_some_and(|draft| draft.close.is_some())
        {
            return self.close_member_submit(false);
        }
        let Some(draft) = self
            .workspace_form
            .as_mut()
            .filter(|draft| draft.stage == Stage::Review)
        else {
            return self.say("This operation is already pending or finished; close and refresh.");
        };
        if draft.controls.get()[1].is_none() {
            return self.say("The confirmation control is not visible; resize and read the operation before applying.");
        }
        draft.stage = Stage::Pending;
        draft.invalidate();
        *draft.scroll.borrow_mut() = Default::default();
        Effect::Act(Request::WorkspaceOperation(
            draft.operation.clone().expect("reviewed operation"),
        ))
    }
}
