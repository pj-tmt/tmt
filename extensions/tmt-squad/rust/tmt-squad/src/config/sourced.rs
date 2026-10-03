//! Reader results carry provenance; presentation never reinterprets the document.
use super::*;
use crate::action::Action;
use std::collections::BTreeMap;
pub(super) type Sources = BTreeMap<String, String>;

impl Config {
    fn source_item(&self, path: &[&str]) -> Option<&Item> {
        path.iter()
            .try_fold(self.document.as_item(), |item, key| item.get(key))
    }
    fn source(&self, path: &[&str], fallback: &str) -> String {
        if self.source_item(path).is_some() {
            path.join(".")
        } else {
            fallback.into()
        }
    }
    pub(super) fn layout_setting(&self, squad: &str) -> Result<(Layout, String), SquadError> {
        let layout = self.resolve_layout(squad)?;
        let fallback = if layout == Layout::Crew {
            "simple-board compatibility default".into()
        } else {
            format!("preset:{}", layout.as_str())
        };
        Ok((layout, self.source(&["squad", squad, "layout"], &fallback)))
    }
    pub(super) fn board_setting(&self, squad: &str) -> Result<(Board, Sources), SquadError> {
        let board = self.board(squad)?;
        let preset = format!("preset:{}", self.resolve_layout(squad)?.as_str());
        let (_, source) = self.view_source(squad)?;
        let arrangement = match source {
            "custom" => format!("squad.{squad}.board"),
            "squad" => format!("squad.{squad}.board.view"),
            "board" => "board.view".into(),
            _ => preset.clone(),
        };
        let mut sources = Sources::from([("view".into(), arrangement.clone())]);
        for name in [
            "mode",
            "layout",
            "panes",
            "collapsed",
            "fold_below",
            "direction",
            "sizes",
        ] {
            let fallback = match (name, source) {
                ("mode", "custom") => &preset,
                ("collapsed", "custom") => "default:[]",
                ("fold_below", "custom") => "default:none",
                _ => &arrangement,
            };
            let place = if name == "layout"
                && ["direction", "panes", "sizes"]
                    .iter()
                    .any(|key| self.source_item(&["squad", squad, "board", key]).is_some())
            {
                format!("squad.{squad}.board (simple split over {preset})")
            } else if name == "panes"
                && self
                    .source_item(&["squad", squad, "board", "layout"])
                    .is_some()
            {
                format!("squad.{squad}.board.layout")
            } else {
                self.source(&["squad", squad, "board", name], fallback)
            };
            sources.insert(name.into(), place);
        }
        Ok((board, sources))
    }
    pub(super) fn rows_setting(
        &self,
        squad: &str,
    ) -> Result<(crate::rows::Rows, String, String), SquadError> {
        let rows = self.rows(squad)?;
        let preset = format!("preset:{}", self.resolve_layout(squad)?.as_str());
        Ok((
            rows,
            self.source(
                &["squad", squad, "rows"],
                &self.source(&["squad", squad, "columns"], &preset),
            ),
            self.source(&["squad", squad, "board", "hidden_columns"], "default:[]"),
        ))
    }
    pub(super) fn refresh_setting(
        &self,
        squad: &str,
    ) -> Result<(Option<Duration>, String), SquadError> {
        Ok((
            self.refresh(squad)?,
            self.source(
                &["squad", squad, "board", "refresh"],
                &self.source(&["board", "refresh"], "default:5s"),
            ),
        ))
    }
    pub(super) fn notes_setting(&self, squad: &str) -> Result<(NotesRender, String), SquadError> {
        Ok((
            self.notes_render(squad)?,
            self.source(&["squad", squad, "notes", "render"], "default:markdown"),
        ))
    }
    pub(super) fn state_settings(
        &self,
        squad: &str,
    ) -> Result<Vec<(String, serde_json::Value, String)>, SquadError> {
        let layout = self.layout(squad)?;
        let states = self.states(squad, layout)?;
        let mut names: std::collections::BTreeSet<&str> = layout.states().iter().copied().collect();
        if let Some(table) = self
            .squad_table(squad)?
            .and_then(|s| s.get("states"))
            .and_then(Item::as_table_like)
        {
            names.extend(table.iter().map(|(name, _)| name));
        }
        let mut values: Vec<_> = names
            .into_iter()
            .map(|name| {
                (
                    format!("states.{name}.color"),
                    serde_json::json!(states.color(Some(name))),
                    self.source(
                        &["squad", squad, "states", name, "color"],
                        &format!("preset:{}", layout.as_str()),
                    ),
                )
            })
            .collect();
        if let Some(item) = self.source_item(&["squad", squad, "state_patterns"]) {
            values.push((
                "state_patterns".into(),
                serde_json::json!(item.to_string()),
                format!("squad.{squad}.state_patterns"),
            ));
        }
        Ok(values)
    }
    pub(super) fn provider_settings(
        &self,
        squad: &str,
    ) -> Result<Vec<(crate::provider::Provider, String)>, SquadError> {
        let preset = format!("preset:{}", self.resolve_layout(squad)?.as_str());
        Ok(self
            .providers(squad)?
            .into_iter()
            .map(|provider| {
                let source = self.source(&["squad", squad, "fields", &provider.name], &preset);
                (provider, source)
            })
            .collect())
    }
    pub(super) fn reminder_settings(
        &self,
        squad: &str,
    ) -> Result<(Reminders, Sources), SquadError> {
        Ok((
            self.reminders(squad)?,
            Sources::from([
                (
                    "enabled".into(),
                    self.source(
                        &["squad", squad, "reminders", "enabled"],
                        &format!("preset:{}", self.resolve_layout(squad)?.as_str()),
                    ),
                ),
                (
                    "stale_after".into(),
                    self.source(&["squad", squad, "reminders", "stale_after"], "default:30m"),
                ),
            ]),
        ))
    }
    pub(super) fn rate_setting(&self, squad: &str) -> Result<(TokenRate, Sources), SquadError> {
        let preset = format!("preset:{}", self.resolve_layout(squad)?.as_str());
        let sources = ["enabled", "every", "window", "reduced_motion"]
            .into_iter()
            .map(|name| {
                (
                    name.into(),
                    self.source(
                        &["squad", squad, "board", "token_rate", name],
                        &self.source(&["board", "token_rate", name], &preset),
                    ),
                )
            })
            .collect();
        Ok((self.token_rate(squad)?, sources))
    }
    pub(super) fn tab_settings(&self) -> Result<(Tabs, Sources), SquadError> {
        let sources = ["order", "pin", "hide", "colors.waiting", "colors.blocked"]
            .into_iter()
            .map(|name| {
                let path: Vec<_> = std::iter::once("tabs").chain(name.split('.')).collect();
                (name.into(), self.source(&path, "tab defaults"))
            })
            .collect();
        Ok((self.tabs()?, sources))
    }
    pub(super) fn program_setting(
        &self,
        name: &str,
    ) -> Result<(Option<Vec<String>>, String), SquadError> {
        Ok((self.program(name)?, self.source(&[name], "system default")))
    }
    pub(super) fn binding_settings(
        &self,
        key: &str,
        tmux: bool,
        panes: &[Pane],
        section: Option<usize>,
    ) -> Result<Vec<(String, Action, String)>, SquadError> {
        let aggregate = crate::tabs::aggregate(key);
        let policy = self.tabs()?;
        let tab = if key == crate::tabs::ALL {
            Some("all")
        } else if key == crate::tabs::LEADS {
            Some("leads")
        } else {
            crate::tabs::user_name(key)
        };
        let sections = if let Some(user) = policy
            .user
            .iter()
            .find(|t| Some(t.name.as_str()) == crate::tabs::user_name(key))
        {
            user.sections.clone()
        } else if aggregate {
            Vec::new()
        } else {
            self.sections(key)?
        };
        let bindings = crate::action::effective_bindings(
            self.bindings_for_tab(key, tmux, panes)?,
            section.and_then(|i| sections.get(i)).map(|s| &s.bind),
            !aggregate && self.token_rate(key)?.enabled,
        );
        let owner = tab.map_or_else(|| format!("squad.{key}"), |tab| format!("tabs.{tab}"));
        let mut values = Vec::new();
        for (event, action) in bindings {
            let fallback = if key == crate::tabs::ALL {
                "all-tab preset"
            } else if tmux {
                "host preset:tmux"
            } else {
                "host preset:plain"
            };
            let global = if key == crate::tabs::ALL {
                fallback.into()
            } else {
                self.source(&["bind", &event], fallback)
            };
            let source = if let Some(index) = section.filter(|i| {
                sections
                    .get(*i)
                    .is_some_and(|s| s.bind.contains_key(&event))
            }) {
                format!("{owner}.section[{index}].bind.{event}")
            } else {
                tab.map_or(global.clone(), |tab| {
                    self.source(&["tabs", tab, "bind", &event], &global)
                })
            };
            values.push((format!("bind.{event}"), action, source));
        }
        for (index, section) in sections.iter().enumerate() {
            for (event, action) in &section.bind {
                values.push((
                    format!("section[{index}].bind.{event}"),
                    action.clone(),
                    format!("{owner}.section[{index}].bind.{event}"),
                ));
            }
        }
        Ok(values)
    }
}
