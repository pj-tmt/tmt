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
    Assign,
}
impl Kind {
    pub(in crate::board) fn label(self) -> &'static str {
        match self {
            Self::Create => "Create squad",
            Self::Add => "Add member",
            Self::Lead => "Set lead",
            Self::RenameSquad => "Rename squad",
            Self::RenameMember => "Rename",
            Self::RemoveMember => "Remove member",
            Self::Remove => "Leave squad",
            Self::Assign => "Move membership",
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
}
impl Change {
    pub(super) fn confirm_label(&self) -> &'static str {
        match self {
            Self::RenameMember(_) => "Rename",
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
    pub rooms: BTreeMap<String, RoomTarget>,
    pub identities: BTreeMap<String, OpsMember>,
    pub values: Vec<String>,
    pub operation: Option<Change>,
    pub shown: bool,
    pub timer_impact: Option<usize>,
    pub scroll: RefCell<ScrollState>,
    pub controls: Cell<[Option<Rect>; 2]>,
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
            (Kind::Assign, 0)
                if !self
                    .rooms
                    .values()
                    .any(|room| room.name == text || room.id == text) =>
            {
                Err("Choose an existing squad name or room UUID.".into())
            }
            (Kind::Assign, 1) if text != "add" && !(text == "move" && self.room.is_some()) => {
                Err("Type add, or move for a current squad membership.".into())
            }
            _ => Ok(()),
        }
    }
    pub fn prompts(&self) -> &'static [&'static str] {
        match self.kind {
            Kind::Create => &[
                "Squad name",
                "Lead (optional @agent)",
                "Member (@agent; blank reviews the chosen members)",
            ],
            Kind::Add => &["Member (@agent)"],
            Kind::Lead => &["Lead (@agent; blank clears leadership only)"],
            Kind::RenameSquad => &["New squad name"],
            Kind::RenameMember => &["New agent name"],
            Kind::Remove | Kind::RemoveMember => &[],
            Kind::Assign => &[
                "Destination squad",
                "Add or move (add keeps every current membership)",
            ],
        }
    }
    fn identity(&self, text: &str) -> Result<String, String> {
        let text = text.trim().strip_prefix('@').unwrap_or(text.trim());
        let matches: Vec<_> = self
            .identities
            .values()
            .filter(|member| member.id == text || member.name == text)
            .collect();
        if matches.len() != 1 {
            return Err("Choose one known identity by its @name or UUID.".into());
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
                return Err("Removal is copy-only until Core stop is available.".into());
            }
            Kind::Remove => Operation::Remove {
                room: room()?,
                member: member()?,
            },
            Kind::Assign => {
                let destination = self
                    .rooms
                    .values()
                    .find(|room| room.name == value(0) || room.id == value(0))
                    .cloned()
                    .ok_or_else(|| "Choose an existing squad name or room UUID.".to_owned())?;
                let move_member = match value(1).trim() {
                    "add" => false,
                    "move" if self.room.is_some() => true,
                    _ => return Err("Type add, or move for a current squad membership.".into()),
                };
                Operation::Assign {
                    source: self.room.clone(),
                    destination,
                    member: member()?,
                    move_member,
                }
            }
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
            Kind::RenameMember => "Rename agent".into(),
            Kind::RemoveMember => "Remove member".into(),
            Kind::Remove => format!("Leave squad · {name}"),
            Kind::Assign => "Move to squad".into(),
        }
    }
    pub fn removal_command(&self) -> Option<String> {
        // TODO(#4579): use the identity UUID once Core rm accepts it.
        (self.kind == Kind::RemoveMember)
            .then_some(self.member.as_ref())
            .flatten()
            .filter(|member| !member.name.is_empty() && !member.name.chars().any(char::is_control))
            .map(|member| command_line(&["tmt", "rm", "--force", "--", &member.name]))
    }
    pub fn review_command(&self) -> Option<String> {
        if self.kind == Kind::RemoveMember {
            return self.removal_command();
        }
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
        if self.kind == Kind::RemoveMember {
            let member = self.member.as_ref().expect("admitted selected member");
            return format!(
                "Remove {}\n\nStop the agent first; this only retires the identity and does not close its window.\nRecheck that the name still belongs to this agent before running the command.\nConversation history and notes are kept.\n\n{}",
                member.name,
                self.removal_command()
                    .unwrap_or_else(|| "Unavailable: name cannot be copied safely.".into()),
            );
        }
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
            Change::RenameMember(_) => None,
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
        self.controls.set([None, None]);
        let mut scroll = self.scroll.borrow_mut();
        let content = scroll.content();
        scroll.update(Rect::default(), content, None);
    }
    pub fn focus_fields(&self) -> Vec<tmt_tui::app::ComponentId> {
        let fields: &[&str] = match self.stage {
            Stage::Fields => &["input"],
            Stage::Review => &["cancel", "read", "apply"],
            Stage::Pending | Stage::Result(_) => &["read", "close"],
        };
        fields
            .iter()
            .map(|field| vec!["workspace-form".into(), (*field).into()])
            .collect()
    }
    pub fn stage_id(&self) -> String {
        match self.stage {
            Stage::Fields => "fields",
            Stage::Review => "review",
            Stage::Pending => "pending",
            Stage::Result(_) => "result",
        }
        .into()
    }
}
fn room(app: &App, id: &str) -> Option<RoomTarget> {
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
        field: &tmt_tui::app::ComponentId,
        event: &ratatui::crossterm::event::Event,
    ) -> Option<Effect> {
        use ratatui::crossterm::event::{
            Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind,
        };
        let draft = self.workspace_form.as_ref()?;
        let editing = draft.stage == Stage::Fields;
        if matches!(event, Event::Key(key) if key.code == KeyCode::Esc) {
            self.workspace_close();
            self.input = None;
            return Some(Effect::None);
        }
        if let Event::Mouse(mouse) = event
            && mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && draft.controls.get()[0]
                .is_some_and(|area| area.contains((mouse.column, mouse.row).into()))
        {
            self.workspace_close();
            self.input = None;
            return Some(Effect::None);
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
            Event::Key(key) if editing => {
                self.update_mentions();
                let effect = self
                    .mention_key(*key)
                    .unwrap_or_else(|| self.input_key(*key));
                self.update_mentions();
                Some(effect)
            }
            Event::Key(key) if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) => None,
            Event::Key(key) if key.code == KeyCode::Enter && !editing => {
                match field.last().map(String::as_str) {
                    Some("apply") if draft.stage == Stage::Review => Some(self.workspace_confirm()),
                    Some("cancel" | "close") => {
                        self.workspace_close();
                        Some(Effect::None)
                    }
                    _ => Some(Effect::None),
                }
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
                let controls = draft.controls.get();
                let at = (mouse.column, mouse.row).into();
                if controls[0].is_some_and(|area| area.contains(at)) {
                    self.workspace_close();
                } else if controls[1].is_some_and(|area| area.contains(at)) {
                    return Some(self.workspace_confirm());
                }
                Some(Effect::None)
            }
            _ if field.last().is_some_and(|field| field == "read")
                || matches!(event, Event::Mouse(_)) =>
            {
                draft
                    .scroll
                    .borrow_mut()
                    .input(event)
                    .then_some(Effect::None)
            }
            _ => None,
        }
    }
    pub(in crate::board) fn workspace_close(&mut self) {
        if let Some(draft) = &mut self.workspace_form
            && draft.stage == Stage::Pending
        {
            draft.shown = false;
            draft.invalidate();
        } else {
            self.workspace_form = None;
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
        if let Some(draft) = &mut self.workspace_form
            && (!draft.shown || draft.stage == Stage::Pending)
        {
            if kind == draft.kind {
                draft.shown = true;
                draft.invalidate();
                return Effect::None;
            }
            if draft.stage == Stage::Pending {
                let operation = draft.kind.label();
                return self.say(format!(
                    "{operation} is still pending. Open it from Actions to inspect its result."
                ));
            }
        }
        if matches!(kind, Kind::RenameMember | Kind::RemoveMember)
            && (self.shell.tab != super::super::shell::OpsTab::Agents
                || self.workspace_scope() != crate::config::OpsKeyScope::Member)
        {
            return self.say("Select a visible agent in the Agents list for this action.");
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
        if matches!(
            kind,
            Kind::Remove | Kind::Assign | Kind::RenameMember | Kind::RemoveMember
        ) && member.is_none()
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
                            row_send: None,
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
        let rooms = snapshot
            .roster
            .squads
            .iter()
            .filter_map(|squad| room(self, &squad.room_id).map(|room| (room.id.clone(), room)))
            .collect();
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
        self.workspace_form = Some(Draft {
            kind,
            stage: Stage::Fields,
            room: selected_room,
            member,
            rooms,
            identities: snapshot.roster.members.clone(),
            values: Vec::new(),
            operation: None,
            shown: true,
            timer_impact,
            scroll: Default::default(),
            controls: Cell::new([None, None]),
        });
        self.workspace_form_next()
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
            .is_some_and(|draft| draft.kind == Kind::RemoveMember)
        {
            return if self
                .workspace_form
                .as_ref()
                .is_some_and(|draft| draft.controls.get()[1].is_some())
            {
                self.workspace_copy_command()
            } else {
                Effect::None
            };
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
