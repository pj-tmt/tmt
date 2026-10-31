//! Frozen form defaults, field choices and the launch plan over held public observations.
use super::super::tool_choices::observed_model;
use super::*;
use crate::board::host_launch::CallerHost;
use crate::membership::workspace::RoomTarget;
use std::collections::{BTreeMap, BTreeSet};
use tmt_tui::components::Editor;

/// One form field. Every field but Keep holds text in `Draft::fields` at its `index`;
/// `Slot::ALL` is the screen order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Slot {
    Name,
    Tool,
    Model,
    Effort,
    Host,
    Session,
    Squad,
    /// The plain program's command line, shown only for the plain-shell tool choice.
    Command,
    Keep,
}
impl Slot {
    pub const ALL: [Slot; 9] = [
        Slot::Name,
        Slot::Tool,
        Slot::Command,
        Slot::Model,
        Slot::Effort,
        Slot::Host,
        Slot::Session,
        Slot::Squad,
        Slot::Keep,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Slot::Name => "Name",
            Slot::Tool => "AI tool",
            Slot::Model => "Model",
            Slot::Effort => "Effort",
            Slot::Host => "Host",
            Slot::Session => "Session",
            Slot::Squad => "Squad",
            Slot::Command => "Command",
            Slot::Keep => "Keep",
        }
    }
    /// Its text in `Draft::fields`; Keep has none.
    pub fn index(self) -> usize {
        self as usize
    }
    /// A field chosen from a list (▾).
    pub fn listed(self) -> bool {
        !matches!(self, Slot::Name | Slot::Command | Slot::Keep)
    }
    /// A field the keyboard also types into.
    pub fn typed(self) -> bool {
        !matches!(self, Slot::Host | Slot::Effort | Slot::Keep)
    }
}
/// One row of a field's list. `value: None` is the free-form row: it uses the filter text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Choice {
    pub label: String,
    pub value: Option<String>,
}
impl Choice {
    fn of(value: &str) -> Self {
        Self {
            label: value.into(),
            value: Some(value.into()),
        }
    }
    fn named(label: &str, value: &str) -> Self {
        Self {
            label: label.into(),
            value: Some(value.into()),
        }
    }
    fn free(label: &str) -> Self {
        Self {
            label: label.into(),
            value: None,
        }
    }
}
/// The host's name for people; `pty` itself appears only in the command.
pub(super) fn host_label(host: &str) -> &str {
    match host {
        BUILT_IN_HOST => "Built-in terminal (no tmux)",
        "herdr" => "Herdr",
        other => other,
    }
}
/// The only host whose sessions are chosen in the form.
const SESSION_HOST: &str = "tmux";
const PROVIDER_DEFAULT: &str = "(provider default)";
/// The AI tool choice that starts a plain program instead of an agent: its label is its value.
pub(super) const PLAIN_TOOL: &str = "None · plain shell (no AI)";
/// The program a blank Command runs: the board's `$SHELL`, else `/bin/sh`.
pub(super) fn login_shell(value: Option<std::ffi::OsString>) -> String {
    value
        .and_then(|v| v.into_string().ok())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/bin/sh".into())
}
/// One text field per slot that holds a value; Keep is a toggle, not a field.
pub(super) const FIELDS: usize = 8;
#[derive(Clone, Debug)]
pub(super) struct Draft {
    pub fields: [Editor; FIELDS],
    /// What a blank Command runs (see `login_shell`).
    pub shell: String,
    pub saved: bool,
    pub tools: Vec<String>,
    pub models: BTreeMap<String, Vec<String>>,
    pub sessions: Vec<String>,
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
    /// A squad typed in the form: created with the member once it is kept.
    pub new_squad: Option<String>,
    /// Keep the identity: it launches temporary and is saved only once it started, so a
    /// failed launch leaves no offline identity (#5536).
    pub save: bool,
    pub(super) display: Box<Display>,
}
/// The built-in host: it has no sessions, and its launch names the host itself.
pub(in crate::board) const BUILT_IN_HOST: &str = "pty";
/// The Host a new form starts on: the one Ops runs in when it can launch there (a plain
/// terminal means the built-in host), else the first eligible one. The user can change it.
pub(super) fn default_host(loaded: &Loaded) -> Option<&crate::launch::Host> {
    let eligible = |name: &str| loaded.hosts.iter().find(|h| h.name == name && h.eligible());
    match &loaded.caller_host {
        CallerHost::Named(name) => eligible(name),
        CallerHost::Plain => eligible(BUILT_IN_HOST),
        CallerHost::Unknown => None,
    }
    .or_else(|| loaded.hosts.iter().find(|h| h.eligible()))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Display {
    pub command: Vec<String>,
    pub outer: Option<Vec<String>>,
}
impl Review {
    fn tmt(&self, words: &[&str]) -> String {
        crate::launch::line(
            &std::iter::once(self.display.command[0].clone())
                .chain(words.iter().map(|w| (*w).to_owned()))
                .collect::<Vec<_>>(),
        )
    }
    /// The command that keeps the started identity, when Keep is saved.
    pub fn save_line(&self) -> Option<String> {
        self.save
            .then(|| self.tmt(&["identity", "create", &self.name]))
    }
    pub fn squad_lines(&self) -> Vec<String> {
        let squad = match (&self.room, &self.new_squad) {
            (Some(room), _) => &room.name,
            (None, Some(name)) => name,
            (None, None) => return vec![],
        };
        let init = self
            .new_squad
            .is_some()
            .then(|| self.tmt(&["ops", "sq", "init", squad]));
        init.into_iter()
            .chain([self.tmt(&["ops", "sq", "add", "--squad", squad, "--", &self.name])])
            .collect()
    }
    pub fn copy(&self) -> String {
        std::iter::once(crate::launch::line(
            self.display.outer.as_ref().unwrap_or(&self.display.command),
        ))
        .chain(self.save_line())
        .chain(self.squad_lines())
        .collect::<Vec<_>>()
        .join("\n")
    }
    pub fn copy_notice(&self) -> String {
        if self.save || self.room.is_some() {
            "Copy-only commands. Nothing was created. Run the later commands only after the member started.".into()
        } else {
            "Copy-only command. Nothing was created.".into()
        }
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
/// A word passed as one argument: nothing a shell or the option parser would read differently.
fn plain_word(word: &str) -> bool {
    !word.starts_with('-') && word.trim() == word && !word.chars().any(char::is_control)
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
        let (tools, models) = super::super::tool_choices::observed(&app.references);
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
        let sessions = app
            .references
            .agents
            .values()
            .map(|a| &a.parameters["location"])
            .filter(|l| l["host"] == SESSION_HOST)
            .filter_map(|l| l["session"].as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Self {
            fields: [
                "",
                &tool,
                &model,
                "",
                host,
                session,
                squad.map_or("", |s| s.name.as_str()),
                "",
            ]
            .map(Editor::from),
            shell: login_shell(std::env::var_os("SHELL")),
            saved: true,
            tools,
            models,
            sessions,
            rooms,
        }
    }
    pub fn text(&self, slot: Slot) -> &str {
        self.fields[slot.index()].as_str()
    }
    /// The plain-shell choice: no agent, so no Model or Effort, and a Command instead.
    pub fn plain(&self) -> bool {
        self.text(Slot::Tool) == PLAIN_TOOL
    }
    /// The plain program's argv: the Command split like a shell would, never run through one;
    /// blank runs `shell`.
    pub fn plain_argv(&self) -> Result<Vec<String>, String> {
        let typed = self.text(Slot::Command);
        if typed.trim().is_empty() {
            return Ok(vec![self.shell.clone()]);
        }
        let argv = tmt_cli_style::help::shell_words(typed)
            .filter(|argv| !argv.is_empty())
            .ok_or("The command has an unclosed quote or a stray backslash.")?;
        if argv[0].starts_with('-') || argv.iter().any(|w| w.chars().any(char::is_control)) {
            return Err("A command starts with a program, not '-', and has no controls.".into());
        }
        Ok(argv)
    }
    /// Sets a field's text; another AI tool has other models and efforts.
    pub fn set(&mut self, slot: Slot, value: &str) {
        if slot == Slot::Tool && self.text(Slot::Tool) != value {
            self.fields[Slot::Model.index()].clear();
            self.fields[Slot::Effort.index()].clear();
        }
        self.fields[slot.index()] = Editor::from(value);
    }
    /// The efforts the chosen tool's driver offers; none hides the Effort field.
    pub fn efforts<'a>(&self, loaded: &'a Loaded) -> &'a [String] {
        loaded
            .choices
            .get(self.text(Slot::Tool))
            .map_or(&[], |c| c.efforts.as_slice())
    }
    /// The fields shown, in order: Effort only with efforts, Session only on tmux, and the plain
    /// shell's Command in place of Model and Effort.
    pub fn visible(&self, loaded: Option<&Loaded>) -> Vec<Slot> {
        let plain = self.plain();
        Slot::ALL
            .into_iter()
            .filter(|slot| match slot {
                Slot::Command => plain,
                Slot::Model | Slot::Effort if plain => false,
                Slot::Effort => loaded.is_some_and(|l| !self.efforts(l).is_empty()),
                Slot::Session => self.text(Slot::Host) == SESSION_HOST,
                _ => true,
            })
            .collect()
    }
    /// A field's list. Model lists come from the driver, else from what members reported.
    pub fn choices(&self, slot: Slot, loaded: &Loaded) -> Vec<Choice> {
        let tool = self.text(Slot::Tool);
        match slot {
            Slot::Tool => std::iter::once(Choice::of(PLAIN_TOOL))
                .chain(
                    loaded
                        .choices
                        .keys()
                        .chain(&self.tools)
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .map(|tool| Choice::of(tool)),
                )
                .chain([Choice::free("Type a tool…")])
                .collect(),
            Slot::Model => {
                let driver = loaded.choices.get(tool).map(|c| &c.models);
                let models = driver
                    .filter(|m| !m.is_empty())
                    .or_else(|| self.models.get(tool));
                std::iter::once(Choice::named(PROVIDER_DEFAULT, ""))
                    .chain(models.into_iter().flatten().map(|m| Choice::of(m)))
                    .chain([Choice::free("Type a model id…")])
                    .collect()
            }
            Slot::Effort => std::iter::once(Choice::named(PROVIDER_DEFAULT, ""))
                .chain(self.efforts(loaded).iter().map(|e| Choice::of(e)))
                .collect(),
            Slot::Host => loaded
                .hosts
                .iter()
                .filter(|h| matches!(h.state.as_str(), "builtin" | "ok"))
                .map(|h| Choice::named(host_label(&h.name), &h.name))
                .collect(),
            Slot::Session => self
                .sessions
                .iter()
                .map(|s| Choice::of(s))
                .chain([Choice::free("New session…")])
                .collect(),
            Slot::Squad => std::iter::once(Choice::named("Common pool", ""))
                .chain(self.rooms.iter().map(|r| Choice::of(&r.name)))
                .chain([Choice::free("New squad…")])
                .collect(),
            Slot::Name | Slot::Command | Slot::Keep => vec![],
        }
    }
    /// What a field shows: the human host name, or the empty value's meaning.
    pub fn shown(&self, slot: Slot) -> Option<String> {
        let text = self.text(slot);
        match slot {
            Slot::Host if !text.is_empty() => Some(host_label(text).into()),
            Slot::Squad if !self.saved || text.is_empty() => Some("Common pool".into()),
            _ if !text.is_empty() => Some(text.into()),
            _ => None,
        }
    }
    pub fn review(&self, loaded: &Loaded) -> Result<Review, String> {
        let [name, tool, model, effort, host, session, squad, _] =
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
        // A plain program is its own argv, with no model or effort (#5769).
        let (program, model, effort) = if self.plain() {
            (self.plain_argv()?, "", "")
        } else {
            if tool.is_empty() || tool.starts_with('-') || tool.chars().any(char::is_control) {
                return Err("Choose an AI tool executable.".into());
            }
            if !plain_word(model) {
                return Err("A model id has no outer spaces, controls or leading '-'.".into());
            }
            // A hidden Effort field is never sent.
            let effort = if self.efforts(loaded).is_empty() {
                ""
            } else {
                effort
            };
            (vec![tool.to_owned()], model, effort)
        };
        if host != BUILT_IN_HOST && !plain_word(session) {
            return Err("A session name has no outer spaces, controls or leading '-'.".into());
        }
        let squad = if self.saved { squad } else { "" };
        let room = self.rooms.iter().find(|r| r.name == squad).cloned();
        let new_squad = if squad.is_empty() || room.is_some() {
            None
        } else if plain_word(squad) && squad.chars().count() <= 64 {
            Some(squad.to_owned())
        } else {
            return Err(
                "A new squad name has 1–64 characters, no outer spaces, controls or leading '-'."
                    .into(),
            );
        };
        // Never `--save`: a launch that ends during startup must leave no saved identity, so
        // Keep is applied with `identity create` once the member started (#5536).
        let mut command = vec![loaded.executable.clone(), "run".into()];
        // The built-in host has no sessions: one `tmt run --host pty` is the whole launch,
        // and its template ignores the session it is still filled with.
        let session = if host == BUILT_IN_HOST {
            command.extend(["--host".into(), BUILT_IN_HOST.into()]);
            name
        } else {
            session
        };
        for (option, value) in [("--model", model), ("--effort", effort)] {
            if !value.is_empty() {
                command.extend([option.into(), value.into()]);
            }
        }
        command.push(name.into());
        command.extend(program);
        let outer = loaded
            .prepare(host, session, &command)
            .map_err(|e| e.message)?;
        let mut display_command = command.clone();
        display_command[0] = loaded.display_executable.clone();
        let display_outer = loaded
            .prepare(host, session, &display_command)
            .map_err(|e| e.message)?;
        Ok(Review {
            name: name.into(),
            host: host.into(),
            session: session.into(),
            command,
            outer,
            room,
            new_squad,
            save: self.saved,
            display: Box::new(Display {
                command: display_command,
                outer: display_outer,
            }),
        })
    }
}
