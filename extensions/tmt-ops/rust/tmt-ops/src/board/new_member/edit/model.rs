//! Edit defaults and frozen changed-only operations over held public observations.
use super::super::{
    model::{Draft, Slot},
    *,
};
use crate::{
    config::OpsPreferences,
    membership::workspace::{Operation, RoomTarget},
};
use serde_json::Value;

/// Only the public launch settings, never the launch environment or provider files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Preset {
    pub executable: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub associated: bool,
}
impl Preset {
    pub fn read(core: &crate::core::Core, id: &str) -> Result<Self, String> {
        let value = core.resume_show(id).map_err(|e| e.message)?;
        let launch = &value["launch"];
        if !value["identity"].is_string()
            || !(launch.is_null() || launch["executable"].as_str().is_some_and(|s| !s.is_empty()))
        {
            return Err("Recorded launch settings unavailable.".into());
        }
        Ok(Self {
            executable: launch["executable"].as_str().map(str::to_owned),
            model: launch["model"].as_str().map(str::to_owned),
            effort: launch["effort"].as_str().map(str::to_owned),
            associated: launch["associated"].as_bool().unwrap_or(false),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) enum Target {
    Member {
        selection: OpsTarget,
        id: String,
        name: String,
        observation: Value,
        source: Option<RoomTarget>,
        memberships: Vec<String>,
    },
    Squad(RoomTarget),
}
impl Target {
    pub fn selected(&self) -> OpsTarget {
        match self {
            Self::Member { selection, .. } => selection.clone(),
            Self::Squad(room) => OpsTarget::Squad(room.id.clone()),
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Member { name, .. } => name,
            Self::Squad(room) => &room.name,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Plan {
    pub target: Target,
    pub preset: Option<Preset>,
    pub rename: Option<String>,
    pub membership: Option<Operation>,
    pub runtime: Option<crate::board::switch_tool::Review>,
    pub alert: Option<(bool, String)>,
    pub preferences: Option<(OpsPreferences, OpsPreferences)>,
}
impl Plan {
    pub fn only_rename(&self) -> bool {
        self.rename.is_some()
            && self.membership.is_none()
            && self.runtime.is_none()
            && self.alert.is_none()
            && self.preferences.is_none()
    }
    pub fn empty(&self) -> bool {
        self.rename.is_none()
            && self.membership.is_none()
            && self.runtime.is_none()
            && self.alert.is_none()
            && self.preferences.is_none()
    }
    pub fn lines(&self, executable: &str) -> Vec<String> {
        let mut lines = Vec::new();
        let line = |words: Vec<String>| {
            crate::launch::line(
                &std::iter::once(executable.into())
                    .chain(words)
                    .collect::<Vec<_>>(),
            )
        };
        if let Some(name) = &self.rename {
            let words = match &self.target {
                Target::Member { .. } => vec!["mv".into(), self.target.name().into(), name.clone()],
                Target::Squad(_) => vec![
                    "ops".into(),
                    "sq".into(),
                    "rename".into(),
                    self.target.name().into(),
                    name.clone(),
                ],
            };
            lines.push(line(words));
        }
        if let Some(operation) = &self.membership {
            match operation {
                Operation::Assign {
                    source,
                    destination,
                    member,
                    move_member,
                } => {
                    if *move_member && source.is_some() {
                        lines.push(
                            "Source squad timers are paused before the move; they remain there."
                                .into(),
                        );
                    }
                    lines.push(line(vec![
                        "ops".into(),
                        "sq".into(),
                        "add".into(),
                        "--squad".into(),
                        destination.name.clone(),
                        "--".into(),
                        member.clone(),
                    ]));
                    if *move_member && let Some(source) = source {
                        lines.push(line(vec![
                            "ops".into(),
                            "sq".into(),
                            "rm".into(),
                            "--squad".into(),
                            source.name.clone(),
                            "--".into(),
                            member.clone(),
                        ]));
                    }
                }
                Operation::Remove { room, member } => lines.push(line(vec![
                    "ops".into(),
                    "sq".into(),
                    "rm".into(),
                    "--squad".into(),
                    room.name.clone(),
                    "--".into(),
                    member.clone(),
                ])),
                _ => unreachable!("Edit only moves the selected membership"),
            }
        }
        if let Some(runtime) = &self.runtime {
            lines.push("Relaunch once through this agent's host:".into());
            if runtime.outer.is_none() {
                lines.push("Copy only · nothing runs here; other edits remain unsaved".into());
            }
            lines.push(runtime.copy());
        }
        if let Some((enabled, after)) = &self.alert {
            let name = self.rename.as_deref().unwrap_or(self.target.name());
            for (key, value) in [
                ("idle_alert.after", after.clone()),
                ("idle_alert.enabled", enabled.to_string()),
            ] {
                lines.push(line(vec![
                    "ops".into(),
                    "sq".into(),
                    "config".into(),
                    "set".into(),
                    key.into(),
                    value,
                    "--squad".into(),
                    name.into(),
                ]));
            }
        }
        if let Some((before, after)) = &self.preferences {
            let Target::Squad(room) = &self.target else {
                unreachable!()
            };
            if before.hidden.contains(&room.id) != after.hidden.contains(&room.id) {
                lines.push(format!(
                    "{} squad · {}",
                    if after.hidden.contains(&room.id) {
                        "Hide"
                    } else {
                        "Show"
                    },
                    self.rename.as_deref().unwrap_or(&room.name)
                ));
            }
        }
        if lines.is_empty() {
            lines.push("nothing changed yet".into());
        }
        lines
    }
}
fn valid_text(value: &str, what: &str) -> Result<(), String> {
    if value.is_empty()
        || value.starts_with('-')
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        Err(format!(
            "{what} needs a value without outer spaces, controls or a leading '-'."
        ))
    } else {
        Ok(())
    }
}
impl super::Edit {
    pub fn plan(&self, draft: &Draft, loaded: &Loaded) -> Result<Plan, String> {
        let name = draft.text(Slot::Name);
        valid_text(name, "Name")?;
        if name.chars().count() > 64 {
            return Err("Name is limited to 64 characters.".into());
        }
        let mut plan = Plan {
            target: self.target.clone(),
            preset: None,
            rename: (name != self.target.name()).then(|| name.into()),
            membership: None,
            runtime: None,
            alert: None,
            preferences: None,
        };
        match &self.target {
            Target::Member {
                id,
                source,
                memberships,
                ..
            } => {
                let destination = draft.text(Slot::Squad);
                let initial = source.as_ref().map_or("", |r| r.name.as_str());
                if destination != initial {
                    if destination.is_empty() {
                        if memberships.len() != 1 {
                            return Err(
                                "Common pool is offered only for a single membership.".into()
                            );
                        }
                        plan.membership = Some(Operation::Remove {
                            room: source
                                .clone()
                                .ok_or("The selected membership is unavailable.")?,
                            member: id.clone(),
                        });
                    } else {
                        let room = draft
                            .rooms
                            .iter()
                            .find(|r| r.name == destination)
                            .cloned()
                            .ok_or("Choose an observed squad; Edit does not create squads.")?;
                        if source.is_some() || !memberships.contains(&room.id) {
                            plan.membership = Some(Operation::Assign {
                                source: source.clone(),
                                destination: room,
                                member: id.clone(),
                                move_member: source.is_some(),
                            });
                        }
                    }
                }
                let tool = draft.text(Slot::Tool);
                let model = draft.text(Slot::Model);
                let effort = draft.text(Slot::Effort);
                let tool_changed = tool != self.initial[Slot::Tool.index()];
                let model_changed = model != self.initial[Slot::Model.index()] && !model.is_empty();
                let effort_changed =
                    effort != self.initial[Slot::Effort.index()] && !effort.is_empty();
                if tool_changed || model_changed || effort_changed {
                    valid_text(tool, "AI tool")?;
                    if tool_changed
                        && !loaded.choices.contains_key(tool)
                        && !draft.tools.iter().any(|offered| offered == tool)
                    {
                        return Err("Choose an AI tool from the public or observed choices.".into());
                    }
                    if model_changed {
                        valid_text(model, "Model")?;
                    }
                    if effort_changed && !draft.efforts(loaded).iter().any(|v| v == effort) {
                        return Err("Choose an effort offered by this AI tool.".into());
                    }
                    if draft.text(Slot::Host).is_empty() || draft.text(Slot::Session).is_empty() {
                        return Err(
                            "Host and session must be publicly observed before a runtime edit."
                                .into(),
                        );
                    }
                    let Target::Member {
                        name, observation, ..
                    } = &self.target
                    else {
                        unreachable!()
                    };
                    plan.preset=Some(self.preset.as_ref().map_err(|_|"Recorded launch settings unavailable; reopen before changing runtime fields.")?.clone());
                    let source = crate::board::switch_tool::Source::frozen(
                        self.target.selected(),
                        name.clone(),
                        observation,
                    );
                    plan.runtime = Some(crate::board::switch_tool::Review::for_edit(
                        &source,
                        tool,
                        if !model.is_empty() && (tool_changed || model_changed) {
                            model
                        } else {
                            ""
                        },
                        if !effort.is_empty() && (tool_changed || effort_changed) {
                            Some(effort)
                        } else {
                            None
                        },
                        loaded,
                    )?);
                }
            }
            Target::Squad(room) => {
                if !crate::squad::valid_name(name) {
                    return Err("Squad names use 1–24 lowercase letters, digits and '-', starting with a letter.".into());
                }
                let original = loaded
                    .config
                    .idle_alert(&room.name)
                    .map_err(|e| e.message)?;
                let after = self.after.as_str();
                // Both fields are validated against the held Config before any mutation.
                let validated = loaded
                    .config
                    .preview_setting(Some(&room.name), "idle_alert.after", after)
                    .map_err(|_| {
                        "After must be 5m–24h in whole s/m/h units, such as 30m.".to_owned()
                    })?;
                let after_setting = validated.idle_alert(&room.name).map_err(|e| e.message)?;
                if self.enabled != original.enabled || after_setting.after != original.after {
                    plan.alert = Some((self.enabled, after.into()));
                }
                if self.hidden != self.preferences.hidden.contains(&room.id) {
                    let mut next = self.preferences.clone();
                    if self.hidden {
                        if next.hidden.len() >= 1024 {
                            return Err(
                                "Hidden squad limit reached (1024); show a squad first.".into()
                            );
                        }
                        next.hidden.insert(room.id.clone());
                    } else {
                        next.hidden.remove(&room.id);
                    }
                    plan.preferences = Some((self.preferences.clone(), next));
                }
            }
        }
        Ok(plan)
    }
}
