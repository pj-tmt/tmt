//! The binding editor holds an opening sample and typed Config destination; j presets share it.
use crate::{
    action::{Action, Verb},
    config::{Config, OpsKeyScope},
    core::SquadError,
    settings::BoardSettings,
};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::board) struct Selection {
    /// The selected context owns section binding provenance.
    pub row: Option<Value>,
    /// A squad's host action targets its current lead, not the squad record.
    pub recipient: Option<Value>,
    /// Opening effective factory choices (including recipient-dependent Enter/j).
    pub factory_bindings: crate::action::Bindings,
    pub squad: Option<String>,
    pub scope: OpsKeyScope,
    pub invoker_tmux: bool,
    /// Opening held sample, never saved argv or live authority.
    pub ops: Value,
}

pub(super) struct HostAction {
    pub selection: Selection,
    pub board_pane: Option<String>,
    pub destination: Option<crate::config::BindingLocation>,
}

impl HostAction {
    pub fn render_preview(
        &self,
        frame: &mut ratatui::Frame,
        overlay: &super::Overlay,
        text: &str,
        look: crate::look::Look,
        body: ratatui::layout::Rect,
    ) {
        use serde_json::json;
        let readonly = overlay
            .selected_pick()
            .is_some_and(|pick| pick.id != "host-custom");
        let title = self.title(if readonly {
            "Preview"
        } else {
            "Custom command"
        });
        let event = overlay
            .editing
            .as_ref()
            .and_then(|(key, _)| key.strip_prefix("bind."))
            .unwrap_or("j");
        let meaning = format!(
            "{} · {}",
            self.meaning(text),
            self.destination_notice(overlay.config.as_ref().unwrap(), event)
        );
        let command = self
            .preview(text)
            .unwrap_or_else(|error| format!("Preview unavailable: {error}"));
        let parameters = self
            .selection
            .recipient
            .as_ref()
            .or(self.selection.row.as_ref())
            .map(|row| {
                crate::template::Parameters {
                    row,
                    ops: Some(&self.selection.ops),
                    board_pane: self.board_pane.as_deref(),
                }
                .inventory()
            })
            .unwrap_or_default();
        let error = overlay
            .notice
            .as_ref()
            .filter(|notice| notice.mark == tmt_cli_style::mark::Mark::Failed)
            .map(|notice| notice.display(body.width.saturating_sub(4), &overlay.settings.path))
            .unwrap_or_default();
        let width = body.width.saturating_sub(4);
        // Borders, scroll position and footer reserve four rows; Custom also has a query row.
        let height = [&meaning, &command]
            .into_iter()
            .chain(parameters.iter())
            .map(|line| tmt_tui::text::lines(line, width, tmt_tui::style::TextFlow::Wrap).len())
            .sum::<usize>()
            .saturating_add(if error.is_empty() {
                0
            } else {
                tmt_tui::text::lines(&error, width, tmt_tui::style::TextFlow::Wrap).len()
            })
            .saturating_add(if readonly { 5 } else { 6 })
            .min(usize::from(u16::MAX));
        let query = if readonly {
            ""
        } else {
            r#"<tmt-text slot="query" bind="$.query" token="text" class="truncate-middle"/>"#
        };
        let status = if error.is_empty() {
            ""
        } else {
            r#"<tmt-text slot="status" bind="$.status" token="blocked" wrap="true"/>"#
        };
        let markup = format!(
            r#"<tmt-view version="1"><tmt-modal id="settings-edit" title="{title}" placement="docked" class="w-{} h-{height}"><tmt-scroll id="body"><tmt-text bind="$.meaning" token="text" wrap="true"/><tmt-text bind="$.command" token="dim" wrap="true"/><tmt-text token="muted" wrap="true">Parameters · opening sample</tmt-text><tmt-repeat each="$.parameters" as="parameter"><tmt-text bind="parameter.text" token="dim" wrap="true"/></tmt-repeat></tmt-scroll>{query}{status}<tmt-text slot="footer" bind="$.footer" token="muted"/></tmt-modal></tmt-view>"#,
            body.width
        );
        let key = super::PromptStyle {
            host_title: Some(title),
            width: body.width,
            height,
            notice_role: if error.is_empty() { "dim" } else { "blocked" },
        };
        let mut cache = overlay.prompt_template.borrow_mut();
        if cache.as_ref().is_none_or(|old| old.key != key) {
            let mut schema = crate::board::picker_surface::schema(&[]);
            let tmt_tui::binding::Schema::Object(root) = &mut schema else {
                unreachable!()
            };
            for name in ["meaning", "command"] {
                root.insert(name.into(), tmt_tui::binding::Schema::Scalar);
            }
            root.insert(
                "parameters".into(),
                tmt_tui::binding::Schema::Collection(Box::new(tmt_tui::binding::Schema::Object(
                    std::collections::BTreeMap::from([(
                        "text".into(),
                        tmt_tui::binding::Schema::Scalar,
                    )]),
                ))),
            );
            cache.replace(super::CachedTemplate {
                key,
                template: crate::board::picker_surface::compile(super::FILE, &markup, schema),
            });
        }
        overlay.prompt.borrow_mut().render_modal(super::FILE, &cache.as_ref().unwrap().template, json!({"rows":[],"query":format!("{text}▏"),"meaning":meaning,"command":command,"parameters":parameters.into_iter().map(|text| json!({"text":text})).collect::<Vec<_>>(),"status":error,"footer":if readonly {"s save · Esc cancel"} else {"Enter save · Tab scope · Esc cancel · Ctrl-U clear"}}), frame, look, body);
    }

    pub fn settings(&mut self, config: &Config) -> Result<BoardSettings, SquadError> {
        let locations = self.locations(config)?;
        let mut settings = config.settings(None, false, None)?;
        settings.entries.clear();
        let mut bindings = config
            .binding_events(&locations)?
            .into_iter()
            .map(|event| {
                config
                    .binding(
                        self.selection.scope,
                        &locations,
                        &event,
                        &self.selection.factory_bindings,
                    )
                    .map(|binding| (event, binding))
            })
            .collect::<Result<Vec<_>, _>>()?;
        bindings.sort_by(|(a, binding_a), (b, binding_b)| {
            (
                a != "j",
                binding_a.action.is_none() && binding_a.authored.is_none(),
                a,
            )
                .cmp(&(
                    b != "j",
                    binding_b.action.is_none() && binding_b.authored.is_none(),
                    b,
                ))
        });
        for (
            event,
            crate::config::ResolvedBinding {
                action,
                owner,
                authored,
            },
        ) in bindings
        {
            let source = owner
                .as_ref()
                .map_or_else(|| "workspace default".into(), |owner| owner.label(&event));
            let value = action.as_ref().map(|action| action.text.as_str());
            settings.push(format!("bind.{event}"), json!(value), source);
            let entry = settings.entries.last_mut().unwrap();
            entry.editable = true;
            let availability = match authored.as_ref().or(action.as_ref()) {
                Some(action)
                    if crate::config::admit_workspace_action(action, &entry.key).is_err() =>
                {
                    format!("Retired or unavailable authored action: {}", action.text)
                }
                Some(action) => self
                    .preview(&action.text)
                    .unwrap_or_else(|error| format!("Preview unavailable: {error}")),
                None => "Unassigned".into(),
            };
            entry.description = (action.is_some() || authored.is_some())
                .then(|| format!("{availability} · opening sample: {}", self.name()));
        }
        Ok(settings)
    }

    pub fn locations(
        &self,
        config: &Config,
    ) -> Result<Vec<crate::config::BindingLocation>, SquadError> {
        config.binding_locations(
            self.selection.row.as_ref(),
            self.selection.recipient.as_ref(),
            self.selection.squad.as_deref(),
        )
    }

    pub fn begin(&mut self, config: &Config, event: &str) {
        self.destination = config
            .binding(
                self.selection.scope,
                &self.locations(config).expect("validated locations"),
                event,
                &self.selection.factory_bindings,
            )
            .expect("validated bindings")
            .owner
            .or(Some(crate::config::BindingLocation::Global));
    }

    pub fn cycle_destination(&mut self, config: &Config) {
        let locations = self.locations(config).expect("validated locations");
        let next = locations
            .iter()
            .position(|location| Some(location) == self.destination.as_ref())
            .map_or(0, |index| (index + 1) % locations.len());
        self.destination = Some(locations[next].clone());
    }

    pub fn destination_notice(&self, config: &Config, event: &str) -> String {
        let destination = self.destination.as_ref().expect("editing destination");
        let winner = config
            .binding(
                self.selection.scope,
                &self.locations(config).expect("validated locations"),
                event,
                &self.selection.factory_bindings,
            )
            .expect("validated bindings")
            .owner;
        let masked = winner.filter(|winner| {
            let locations = self.locations(config).expect("validated locations");
            locations.iter().position(|location| location == winner)
                > locations
                    .iter()
                    .position(|location| location == destination)
        });
        format!(
            "Save to {}{} · Tab scope · opening sample",
            destination.label(event),
            masked
                .map(|owner| format!("; still masked by {}", owner.label(event)))
                .unwrap_or_default()
        )
    }

    pub fn name(&self) -> String {
        self.selection
            .recipient
            .as_ref()
            .and_then(|row| row["name"].as_str())
            .map(crate::board::notes::sanitize)
            .unwrap_or_else(|| "(no agent selected)".into())
    }

    pub fn title(&self, label: &str) -> String {
        format!("{label} · {}", self.name())
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('"', "&quot;")
    }

    pub fn meaning(&self, line: &str) -> String {
        let name = self.name();
        match Action::parse(line) {
            Ok(action) if action.verb == Verb::Jump && action.args.is_empty() => {
                format!("Focus {name}")
            }
            Ok(action) if action.verb == Verb::Swap => format!(
                "Swap {name} into the pane {} the board",
                match action.args[0].literal().unwrap() {
                    "left" => "left of",
                    "right" => "right of",
                    "up" => "above",
                    "down" => "below",
                    _ => unreachable!("validated direction"),
                }
            ),
            Ok(action) if action.verb != Verb::Run => action.workspace_description(),
            _ => format!("Run a custom command for {name}"),
        }
    }

    pub fn picks(current: &str) -> Vec<super::Pick> {
        let action = Action::parse(current).ok();
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
            value: if action
                .as_ref()
                .is_some_and(|action| match (id, action.verb) {
                    ("host-focus", Verb::Jump) => action.args.is_empty(),
                    ("host-custom", Verb::Run) => true,
                    (_, Verb::Swap) => id.strip_prefix("host-") == action.args[0].literal(),
                    _ => false,
                }) {
                "(current)".into()
            } else {
                String::new()
            },
            hint: "",
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
        if action.verb.acts_on_member() && self.selection.recipient.is_none() {
            return Err("Select a visible named member before running this action.".into());
        }
        let row = self
            .selection
            .recipient
            .as_ref()
            .or(self.selection.row.as_ref())
            .ok_or("Select a row.")?;
        let parameters = crate::template::Parameters {
            row,
            ops: Some(&self.selection.ops),
            board_pane: self.board_pane.as_deref(),
        };
        match action.verb {
            Verb::Run => Ok(format!("argv {:?}", action.argv_parameters(parameters)?)),
            Verb::Swap => Ok(format!(
                "tmt focus {} --into {} --json",
                row["id"].as_str().ok_or("No identity UUID.")?,
                action.args[0].literal().expect("validated direction")
            )),
            Verb::Jump if !action.args.is_empty() => Err(
                "This binding targets the squad lead; Focus agent is a different preset.".into(),
            ),
            Verb::Jump if !self.selection.invoker_tmux => {
                Ok("UI: open conversation (no shell command)".into())
            }
            Verb::Jump => crate::board::overview::actions::host(row, true)
                .map(|id| format!("tmt focus {id} --json"))
                .map_err(str::to_owned),
            Verb::Copy => action.args[0]
                .fill_parameters(parameters)
                .map(|text| format!("Copy {text:?}")),
            Verb::Open => crate::effects::resolve_open_parameters(&action, parameters)
                .map(|link| format!("Open {link:?}")),
            _ => Ok(format!(
                "UI: {} (no shell command)",
                action.workspace_description()
            )),
        }
    }
}
