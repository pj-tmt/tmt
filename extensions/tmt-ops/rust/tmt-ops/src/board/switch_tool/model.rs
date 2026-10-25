//! Frozen public source and requested launch, without provider/session ownership.
use super::super::{app::App, host_launch::Loaded, workspace::OpsTarget};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Source {
    pub target: OpsTarget,
    pub name: String,
    pub observation: Value,
}
fn observation(row: &Value) -> Value {
    json!({"location":row["location"], "runningDriver":row["runningDriver"],
        "driverSupport":row["driverSupport"], "unbound":row["session"]["unbound"],
        "driver":row["resume"]["driver"], "session":row["resume"]["session"],
        "staleAtMs":row["resume"]["staleAtMs"]})
}
impl Source {
    pub fn from_app(app: &App) -> Option<Self> {
        let target = app.workspace_selected_member()?.clone();
        let OpsTarget::Member { identity_id, .. } = &target else {
            return None;
        };
        let agent = app.references.agents.get(identity_id)?;
        if app.references.error.is_some() {
            return None;
        }
        Some(Self {
            target,
            name: agent.name.clone(),
            observation: observation(&agent.parameters),
        })
    }
    pub fn id(&self) -> &str {
        let OpsTarget::Member { identity_id, .. } = &self.target else {
            unreachable!()
        };
        identity_id
    }
    pub fn host(&self) -> &str {
        self.observation["location"]["host"].as_str().unwrap_or("")
    }
    pub fn session(&self) -> &str {
        self.observation["location"]["session"]
            .as_str()
            .unwrap_or("")
    }
    pub fn driver(&self) -> &str {
        self.observation["runningDriver"]
            .as_str()
            .or_else(|| self.observation["driver"].as_str())
            .unwrap_or("unknown")
    }
    pub fn matches(&self, row: &Value) -> bool {
        row["id"] == self.id() && observation(row) == self.observation
    }
    pub fn still_selected(&self, app: &App) -> bool {
        Self::from_app(app)
            .is_some_and(|s| s.target == self.target && s.observation == self.observation)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Review {
    pub source: Source,
    pub driver: String,
    pub model: String,
    pub command: Vec<String>,
    pub outer: Option<Vec<String>>,
    pub display_command: Vec<String>,
    pub display_outer: Option<Vec<String>>,
}
impl Review {
    pub fn new(
        source: &Source,
        driver: &str,
        model: &str,
        loaded: &Loaded,
    ) -> Result<Self, String> {
        if driver.is_empty() || driver.starts_with('-') || driver.chars().any(char::is_control) {
            return Err("Choose an AI tool executable.".into());
        }
        if driver == source.driver() {
            return Err("Choose a different AI tool.".into());
        }
        if model.chars().any(char::is_control) {
            return Err("Model cannot contain control characters.".into());
        }
        if source.host().is_empty() || source.session().is_empty() {
            return Err(
                "Host session unavailable. Reopen after a public member observation.".into(),
            );
        }
        let mut command = vec![
            loaded.executable.clone(),
            "resume".into(),
            format!("--driver={driver}"),
        ];
        if !model.is_empty() {
            command.push(format!("--model={model}"));
        }
        command.extend(["--".into(), source.id().into()]);
        let outer = loaded
            .prepare(source.host(), source.session(), &command)
            .map_err(|e| e.message)?;
        let mut display_command = command.clone();
        display_command[0] = loaded.display_executable.clone();
        let display_outer = loaded
            .prepare(source.host(), source.session(), &display_command)
            .map_err(|e| e.message)?;
        Ok(Self {
            source: source.clone(),
            driver: driver.into(),
            model: model.into(),
            command,
            outer,
            display_command,
            display_outer,
        })
    }
    pub fn copy(&self) -> String {
        crate::launch::line(self.display_outer.as_ref().unwrap_or(&self.display_command))
    }
    pub fn lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!(
                "Switch {} from {} to {}",
                self.source.name,
                self.source.driver(),
                self.driver
            ),
            format!(
                "Destination: {} / {}",
                self.source.host(),
                self.source.session()
            ),
            if self.model.is_empty() {
                "Model: selected tool's remembered or default model".into()
            } else {
                format!("Requested model: {}", self.model)
            },
            String::new(),
            "1. Ask once for a handoff in notes; wait up to 30 seconds.".into(),
            "2. Stop the old agent with one TERM; no force-kill.".into(),
            "3. Start the new tool in a new window with the handoff.".into(),
            "Name, UUID, notes and open requests stay.".into(),
            String::new(),
            if self.outer.is_some() {
                "Will run".into()
            } else {
                "Run in a suitable host pane · copy only; nothing runs here".into()
            },
            crate::launch::line(&self.display_command),
        ];
        if let Some(outer) = &self.display_outer {
            lines.extend(["Opens with".into(), crate::launch::line(outer)]);
        }
        lines
    }
    pub fn observed(&self, row: &Value) -> bool {
        row["id"] == self.source.id()
            && row["location"]["host"] == self.source.host()
            && row["location"]["session"] == self.source.session()
            && row["runningDriver"] == self.driver
            && row["session"]["unbound"] != true
    }
}
