//! The j editor keeps one selected-row sample and reuses its parent's Config draft/writer.
use crate::{
    action::{Action, Verb},
    config::{Config, OpsKeyScope},
    core::SquadError,
    settings::BoardSettings,
};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::board) struct Selection {
    pub row: Option<Value>,
    pub squad: Option<String>,
    pub scope: OpsKeyScope,
}

pub(super) struct HostAction {
    pub selection: Selection,
    pub board_pane: Option<String>,
    pub source: String,
}

impl HostAction {
    pub fn settings(&mut self, config: &Config) -> Result<BoardSettings, SquadError> {
        let (action, source) = config.host_action_binding(
            self.selection.scope,
            self.selection.row.as_ref(),
            self.selection.squad.as_deref(),
        )?;
        self.source = source.clone();
        let mut settings = config.settings(None, false, None)?;
        settings.entries.clear();
        settings.push("bind.j", json!(action.text), source);
        settings.entries[0].editable = true;
        settings.entries[0].description = Some(format!(
            "{} · saves {} · row sample held at opening",
            action.workspace_description(),
            self.write_source()
        ));
        Ok(settings)
    }

    pub fn write_source(&self) -> &str {
        if self.source == "workspace default" {
            "bind.j"
        } else {
            &self.source
        }
    }

    pub fn picks() -> Vec<super::Pick> {
        [
            ("host-focus", "Focus agent"),
            ("host-left", "Swap beside board: left"),
            ("host-right", "Swap beside board: right"),
            ("host-up", "Swap beside board: up"),
            ("host-down", "Swap beside board: down"),
            ("host-custom", "Custom command"),
        ]
        .into_iter()
        .map(|(id, name)| super::Pick {
            id,
            name,
            value: String::new(),
            hint: "Enter preview",
        })
        .collect()
    }

    pub fn pick(&self, id: &str, current: &str) -> Option<String> {
        Some(match id {
            "host-focus" => "jump".into(),
            "host-left" => "swap left".into(),
            "host-right" => "swap right".into(),
            "host-up" => "swap up".into(),
            "host-down" => "swap down".into(),
            "host-custom" => {
                if Action::parse(current).is_ok_and(|action| action.verb == Verb::Run) {
                    current.into()
                } else {
                    "run ".into()
                }
            }
            _ => return None,
        })
    }

    pub fn preview(&self, line: &str) -> Result<String, String> {
        let action = Action::parse(line)?;
        let row = self
            .selection
            .row
            .as_ref()
            .ok_or("Preview unavailable: select an agent.")?;
        let name = row["name"]
            .as_str()
            .ok_or("Preview unavailable: this row has no agent.")?;
        match action.verb {
            Verb::Run => Ok(format!(
                "argv {:?}",
                action.argv_with_board_pane(row, self.board_pane.as_deref())?
            )),
            Verb::Swap => Ok(format!(
                "tmt focus {} --into {} --json",
                row["id"]
                    .as_str()
                    .ok_or("Preview unavailable: no identity UUID.")?,
                action.args[0].literal().expect("validated direction")
            )),
            Verb::Jump => Ok(format!("Focus {name}")),
            _ => Err("Host action takes jump, swap <direction> or run <program>….".into()),
        }
    }
}
