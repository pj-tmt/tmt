//! The j editor holds its binding context and agent sample with the parent's Config draft/writer.
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
    pub squad: Option<String>,
    pub scope: OpsKeyScope,
    pub invoker_tmux: bool,
}

pub(super) struct HostAction {
    pub selection: Selection,
    pub board_pane: Option<String>,
    pub source: String,
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
        let meaning = self.meaning(text);
        let command = self
            .preview(text)
            .unwrap_or_else(|error| format!("Preview unavailable: {error}"));
        let error = overlay
            .notice
            .as_ref()
            .filter(|notice| notice.mark == tmt_cli_style::mark::Mark::Failed)
            .map(|notice| notice.display(body.width.saturating_sub(4), &overlay.settings.path))
            .unwrap_or_default();
        let width = body.width.saturating_sub(4);
        // Borders, scroll position and footer reserve four rows; Custom also has a query row.
        let height = [&meaning, &command]
            .iter()
            .map(|line| tmt_tui::text::lines(line, width, tmt_tui::style::TextFlow::Wrap).len())
            .sum::<usize>()
            .saturating_add(if error.is_empty() {
                0
            } else {
                tmt_tui::text::lines(&error, width, tmt_tui::style::TextFlow::Wrap).len()
            })
            .saturating_add(if readonly { 4 } else { 5 })
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
            r#"<tmt-view version="1"><tmt-modal id="settings-edit" title="{title}" placement="docked" class="w-{} h-{height}"><tmt-scroll id="body"><tmt-text bind="$.meaning" token="text" wrap="true"/><tmt-text bind="$.command" token="dim" wrap="true"/></tmt-scroll>{query}{status}<tmt-text slot="footer" bind="$.footer" token="muted"/></tmt-modal></tmt-view>"#,
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
            cache.replace(super::CachedTemplate {
                key,
                template: crate::board::picker_surface::compile(super::FILE, &markup, schema),
            });
        }
        overlay.prompt.borrow_mut().render_modal(super::FILE, &cache.as_ref().unwrap().template, json!({"rows":[],"query":format!("{text}▏"),"meaning":meaning,"command":command,"status":error,"footer":if readonly {"s save · Esc cancel"} else {"Enter save · Esc cancel · Ctrl-U clear"}}), frame, look, body);
    }

    pub fn settings(&mut self, config: &Config) -> Result<BoardSettings, SquadError> {
        let (action, source) = config.host_action_binding(
            self.selection.scope,
            self.selection.row.as_ref(),
            self.selection.recipient.as_ref(),
            self.selection.squad.as_deref(),
        )?;
        self.source = source.clone();
        let mut settings = config.settings(None, false, None)?;
        settings.entries.clear();
        settings.push("bind.j", json!(action.text), source);
        settings.entries[0].editable = true;
        settings.entries[0].description = Some(format!(
            "Preview uses {} · {}",
            self.name(),
            self.scope_label()
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

    fn scope_label(&self) -> String {
        match self.source.as_str() {
            "workspace default" | "bind.j" => "All agents".into(),
            source => source
                .strip_prefix("bind.")
                .and_then(|source| source.strip_suffix(".j"))
                .map(|host| format!("Host {host}"))
                .unwrap_or_else(|| {
                    self.source
                        .strip_suffix(".bind.j")
                        .unwrap_or(&self.source)
                        .into()
                }),
        }
    }

    pub fn meaning(&self, line: &str) -> String {
        let name = self.name();
        match Action::parse(line) {
            Ok(action) if action.verb == Verb::Jump && action.args.is_empty() => {
                format!("Focus {name}")
            }
            Ok(action) if action.verb == Verb::Jump => action.workspace_description(),
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
        let row = self
            .selection
            .recipient
            .as_ref()
            .ok_or("Select an agent.")?;
        row["name"].as_str().ok_or("This row has no agent.")?;
        match action.verb {
            Verb::Run => Ok(format!(
                "argv {:?}",
                action.argv_with_board_pane(row, self.board_pane.as_deref())?
            )),
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
            _ => Err("Host action takes jump, swap <direction> or run <program>….".into()),
        }
    }
}
