//! Frozen form defaults and review over held public observations.
use super::*;
use crate::membership::workspace::RoomTarget;
use std::collections::{BTreeMap, BTreeSet};
use tmt_tui::components::Editor;

pub(super) const LABELS: [&str; 7] = [
    "Name", "AI tool", "Model", "Squad", "Host", "Session", "Identity",
];
#[derive(Clone, Debug)]
pub(super) struct Draft {
    pub fields: [Editor; 6],
    pub saved: bool,
    pub tools: Vec<String>,
    pub models: BTreeMap<String, Vec<String>>,
    pub rooms: Vec<RoomTarget>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Review {
    pub name: String,
    pub host: String,
    pub session: String,
    pub command: Vec<String>,
    pub outer: Option<Vec<String>>,
    pub room: Option<RoomTarget>,
    pub(super) display: Box<Display>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Display {
    pub command: Vec<String>,
    pub outer: Option<Vec<String>>,
}
impl Review {
    pub fn copy(&self) -> String {
        let mut text =
            crate::launch::line(self.display.outer.as_ref().unwrap_or(&self.display.command));
        if let Some(room) = &self.room {
            text.push('\n');
            text.push_str(&crate::launch::line(&[
                self.display.command[0].clone(),
                "ops".into(),
                "sq".into(),
                "add".into(),
                "--squad".into(),
                room.name.clone(),
                "--".into(),
                self.name.clone(),
            ]));
        }
        text
    }
    pub fn copy_notice(&self) -> String {
        if self.room.is_some() {
            "Copy-only commands. Nothing was created. Run the squad-add command only after the identity exists.".into()
        } else {
            "Copy-only command. Nothing was created.".into()
        }
    }
    pub fn lines(&self) -> Vec<String> {
        let mut text = vec![
            format!("{} · {} / {}", self.name, self.host, self.session),
            format!(
                "Squad: {}",
                self.room
                    .as_ref()
                    .map_or("Common pool", |room| room.name.as_str())
            ),
            if self.outer.is_some() {
                "Will run"
            } else {
                "Run in your terminal"
            }
            .into(),
            crate::launch::line(&self.display.command),
        ];
        if let Some(outer) = &self.display.outer {
            text.push("Opens with".into());
            text.push(crate::launch::line(outer));
        } else {
            text.insert(
                0,
                format!("Copy only · {} has no launch template.", self.host),
            );
        }
        if self.room.is_some() {
            text.push("After the identity exists · squad add".into());
            text.push(self.copy().split_once('\n').unwrap().1.into());
        }
        text
    }
}
fn mode(values: impl Iterator<Item = String>, preferred: Option<&str>) -> Option<String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    let count = counts.values().max().copied()?;
    if let Some(preferred) = preferred
        && counts.get(preferred) == Some(&count)
    {
        return Some(preferred.into());
    }
    counts
        .into_iter()
        .find(|(_, n)| *n == count)
        .map(|(value, _)| value)
}
fn observed_model(agent: &super::super::inventory::Agent) -> Option<&str> {
    agent.parameters["resume"]["observedModel"]["model"]
        .as_str()
        .filter(|model| !model.is_empty())
}
impl Draft {
    pub fn from_app(app: &App) -> Self {
        let target = app.workspace.selected.as_ref();
        let room_id = match target {
            Some(OpsTarget::Squad(id))
            | Some(OpsTarget::Member {
                room_id: Some(id), ..
            }) => Some(id),
            _ => None,
        };
        let selected = match target {
            Some(OpsTarget::Member { identity_id, .. }) => app.references.agents.get(identity_id),
            _ => None,
        };
        let snapshot = app.workspace_snapshot();
        let rooms: Vec<_> = snapshot
            .into_iter()
            .flat_map(|s| &s.roster.squads)
            .filter(|s| s.observation == super::super::workspace::OpsObservation::Complete)
            .filter_map(|s| {
                Some(RoomTarget {
                    id: s.room_id.clone(),
                    name: s.name.clone(),
                    revision: snapshot?.rows.get(&OpsTarget::Squad(s.room_id.clone()))?["revision"]
                        .as_u64()?,
                })
            })
            .collect();
        let squad =
            snapshot.and_then(|s| s.roster.squads.iter().find(|s| Some(&s.room_id) == room_id));
        let ids: BTreeSet<_> = squad.into_iter().flat_map(|s| &s.members).collect();
        let agents: Vec<_> = ids
            .into_iter()
            .filter_map(|id| app.references.agents.get(id))
            .collect();
        let tool = mode(
            agents.iter().filter_map(|a| a.runtime.clone()),
            selected.and_then(|a| a.runtime.as_deref()),
        )
        .unwrap_or_default();
        let model = mode(
            agents
                .iter()
                .filter(|a| a.runtime.as_deref() == Some(&tool))
                .filter_map(|a| observed_model(a).map(str::to_owned)),
            selected
                .filter(|a| a.runtime.as_deref() == Some(&tool))
                .and_then(observed_model),
        )
        .unwrap_or_default();
        let mut models = BTreeMap::<String, Vec<String>>::new();
        for agent in app.references.agents.values() {
            if let (Some(tool), Some(model)) = (&agent.runtime, observed_model(agent)) {
                let options = models.entry(tool.clone()).or_default();
                if !options.iter().any(|option| option == model) {
                    options.push(model.to_owned());
                }
            }
        }
        for options in models.values_mut() {
            options.sort();
        }
        let tools = app
            .references
            .agents
            .values()
            .filter_map(|a| a.runtime.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let location = selected
            .map(|a| &a.parameters["location"])
            .filter(|l| l["host"].is_string())
            .or_else(|| {
                agents
                    .iter()
                    .map(|a| &a.parameters["location"])
                    .find(|l| l["host"].is_string())
            });
        let host = location.and_then(|l| l["host"].as_str()).unwrap_or("");
        let session = location
            .and_then(|l| l["session"].as_str())
            .or_else(|| squad.map(|s| s.name.as_str()))
            .unwrap_or("common");
        Self {
            fields: [
                "",
                &tool,
                &model,
                squad.map_or("", |s| s.name.as_str()),
                host,
                session,
            ]
            .map(Editor::from),
            saved: true,
            tools,
            models,
            rooms,
        }
    }
    pub fn review(&self, loaded: &Loaded) -> Result<Review, String> {
        let [name, tool, model, squad, host, session] =
            self.fields.each_ref().map(|field| field.as_str());
        if name.is_empty()
            || name.chars().count() > 64
            || name.starts_with('-')
            || name.trim() != name
            || name.chars().any(char::is_control)
        {
            return Err(
                "Choose an agent name of 1–64 characters without outer spaces, controls or a leading '-'.".into(),
            );
        }
        if tool.is_empty() || tool.starts_with('-') || tool.chars().any(char::is_control) {
            return Err("Choose an AI tool executable.".into());
        }
        if model.chars().any(char::is_control) {
            return Err("Model cannot contain control characters.".into());
        }
        let squad = if self.saved { squad } else { "" };
        let room = if squad.is_empty() {
            None
        } else {
            Some(
                self.rooms
                    .iter()
                    .find(|r| r.name == squad)
                    .ok_or("Choose a published squad or clear it for Common pool.")?
                    .clone(),
            )
        };
        let chosen = loaded
            .hosts
            .iter()
            .find(|h| h.name == host && matches!(h.state.as_str(), "builtin" | "ok"))
            .ok_or("Choose an available published host.")?;
        let mut command = vec![loaded.executable.clone(), "run".into()];
        if self.saved {
            command.push("--save".into());
        }
        command.extend([name.into(), tool.into()]);
        if !model.is_empty() {
            command.push(format!("--model={model}"));
        }
        let argv = loaded
            .config
            .launch_override(host)
            .map_err(|e| e.message)?
            .or_else(|| chosen.argv.clone());
        // Validate a session even for copy-only reviews; it remains explicit.
        let probe = ["host", "{session}", "{command}"].map(str::to_owned);
        crate::launch::fill(&probe, session, &crate::launch::line(&command))?;
        let outer = argv
            .as_ref()
            .map(|argv| crate::launch::fill(argv, session, &crate::launch::line(&command)))
            .transpose()?;
        let mut display_command = command.clone();
        display_command[0] = loaded.display_executable.clone();
        let display_outer = argv
            .as_ref()
            .map(|argv| crate::launch::fill(argv, session, &crate::launch::line(&display_command)))
            .transpose()?;
        Ok(Review {
            name: name.into(),
            host: host.into(),
            session: session.into(),
            command,
            outer,
            room,
            display: Box::new(Display {
                command: display_command,
                outer: display_outer,
            }),
        })
    }
}
