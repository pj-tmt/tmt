//! Settings are projections of existing readers. Source paths annotate their results.
use super::*;
use crate::{
    settings::BoardSettings,
    split::{Size, Split},
    tabs,
};
use serde_json::{Value, json};

fn split_value(split: &Split) -> Value {
    match split {
        Split::Pane(pane) => json!(pane.title()),
        Split::Group {
            direction,
            children,
        } => json!({
            "direction": match direction { Direction::LeftRight => "left-right", Direction::TopBottom => "top-bottom" },
            "sizes": children.iter().map(|(size, _)| match size {
                Size::Percent(n) => json!(n), Size::Grow(n) => json!(format!("{n}fr")),
            }).collect::<Vec<_>>(),
            "panes": children.iter().map(|(_, child)| split_value(child)).collect::<Vec<_>>(),
        }),
    }
}

fn theme_path(squad: &str, name: &str, source: &str) -> String {
    match source {
        "squad" => format!("squad.{squad}.theme.{name}"),
        "board" => format!("board.theme.{name}"),
        "cli" => format!("CLI theme.{name} (tmt config show)"),
        _ => "default:auto".into(),
    }
}

impl Config {
    fn setting_item(&self, path: &[&str]) -> Option<&Item> {
        path.iter()
            .try_fold(self.document.as_item(), |item, key| item.get(key))
    }
    fn setting_source(&self, path: &[&str], fallback: &str) -> String {
        if self.setting_item(path).is_some() {
            path.join(".")
        } else {
            fallback.into()
        }
    }

    pub fn settings(
        &self,
        context: Option<&str>,
        tmux: bool,
        section: Option<usize>,
    ) -> Result<BoardSettings, SquadError> {
        let key = context.unwrap_or("");
        let aggregate = tabs::aggregate(key);
        let squad = if aggregate { "" } else { key };
        let layout = self.layout(squad)?;
        let preset = format!("preset:{}", layout.as_str());
        let mut out = BoardSettings {
            path: self.path.display().to_string(),
            context: context.map(str::to_owned),
            host: if tmux { "tmux" } else { "plain" },
            entries: Vec::new(),
            notices: Vec::new(),
        };
        let policy = self.tabs()?;
        let user = policy
            .user
            .iter()
            .find(|tab| Some(tab.name.as_str()) == tabs::user_name(key));
        if tabs::user_name(key).is_some() && user.is_none() {
            return Err(SquadError::new("SQUAD_TAB_NOT_FOUND", "Unknown tab key."));
        }
        let board = if aggregate {
            Board::simple(
                BoardMode::Split,
                Direction::LeftRight,
                vec![Pane::Rows],
                &[100],
            )
        } else {
            self.board(squad)?
        };
        let (view, view_source) = self.view_source(squad)?;
        let arrangement = if aggregate {
            "aggregate tab".into()
        } else {
            match view_source {
                "custom" => format!("squad.{squad}.board"),
                "squad" => format!("squad.{squad}.board.view"),
                "board" => "board.view".into(),
                _ => preset.clone(),
            }
        };
        if !aggregate {
            out.push(
                "layout",
                json!(layout.as_str()),
                self.setting_source(
                    &["squad", squad, "layout"],
                    if layout == Layout::Crew {
                        "simple-board compatibility default"
                    } else {
                        &preset
                    },
                ),
            );
            out.push(
                "board.view",
                json!(view.map(|view| view.name())),
                &arrangement,
            );
        }
        for (name, value) in [
            ("mode", json!(if board.mode == BoardMode::Split { "split" } else { "tabs" })),
            ("layout", split_value(&board.split)),
            ("panes", json!(board.panes.iter().map(|pane| pane.title()).collect::<Vec<_>>())),
            ("collapsed", json!(board.collapsed.iter().map(|pane| pane.title()).collect::<Vec<_>>())),
            ("fold_below", json!(board.fold_below.as_ref().map(|fold| json!({"width": fold.width, "panes": fold.panes.iter().map(|pane| pane.title()).collect::<Vec<_>>()})))),
        ] {
            let fallback = match (name, view_source) {
                ("mode", "custom") => &preset,
                ("collapsed", "custom") => "default:[]",
                ("fold_below", "custom") => "default:none",
                _ => &arrangement,
            };
            let source = if aggregate {
                arrangement.clone()
            } else if name == "layout" && ["direction", "panes", "sizes"].iter().any(|key| {
                self.setting_item(&["squad", squad, "board", key]).is_some()
            }) {
                format!("squad.{squad}.board (simple split over {preset})")
            } else if name == "panes"
                && self.setting_item(&["squad", squad, "board", "layout"]).is_some()
            {
                format!("squad.{squad}.board.layout")
            } else {
                self.setting_source(&["squad", squad, "board", name], fallback)
            };
            out.push(format!("board.{name}"), value, source);
        }
        // Aggregate readers use the tab key for refresh, but never acquire squad panes/providers.
        let refresh = self.refresh(key)?;
        out.push(
            "board.refresh",
            json!(
                refresh.map_or_else(|| "off".to_owned(), |every| format!("{}s", every.as_secs()))
            ),
            self.setting_source(
                &["squad", key, "board", "refresh"],
                &self.setting_source(&["board", "refresh"], "default:5s"),
            ),
        );
        let rows = if aggregate {
            crate::tab_view::rows(key)
        } else {
            self.rows(squad)?
        };
        let row_source = if aggregate {
            "aggregate tab".into()
        } else {
            self.setting_source(
                &["squad", squad, "rows"],
                &self.setting_source(&["squad", squad, "columns"], &preset),
            )
        };
        out.push("rows", rows.value(), row_source);
        out.push(
            "notes.render",
            json!(
                if aggregate || self.notes_render(squad)? == NotesRender::Markdown {
                    "markdown"
                } else {
                    "plain"
                }
            ),
            if aggregate {
                "aggregate tab".into()
            } else {
                self.setting_source(&["squad", squad, "notes", "render"], "default:markdown")
            },
        );
        if !aggregate {
            let states = self.states(squad, layout)?;
            let mut names: std::collections::BTreeSet<&str> =
                layout.states().iter().copied().collect();
            let own = self.squad_table(squad)?;
            if let Some(table) = own
                .and_then(|table| table.get("states"))
                .and_then(Item::as_table_like)
            {
                names.extend(table.iter().map(|(name, _)| name));
            }
            for name in names {
                out.push(
                    format!("states.{name}.color"),
                    json!(states.color(Some(name))),
                    self.setting_source(&["squad", squad, "states", name, "color"], &preset),
                );
            }
            let reminders = self.reminders(squad)?;
            out.push(
                "reminders.enabled",
                json!(reminders.enabled),
                self.setting_source(&["squad", squad, "reminders", "enabled"], &preset),
            );
            out.push(
                "reminders.stale_after",
                json!(format!("{}s", reminders.stale_after.as_secs())),
                self.setting_source(&["squad", squad, "reminders", "stale_after"], "default:30m"),
            );
            if let Some(item) = own.and_then(|table| table.get("state_patterns")) {
                out.push(
                    "state_patterns",
                    json!(item.to_string()),
                    format!("squad.{squad}.state_patterns"),
                );
            }
            for provider in self.providers(squad)? {
                out.push(
                    format!("fields.{}", provider.name),
                    provider.settings(),
                    self.setting_source(&["squad", squad, "fields", &provider.name], &preset),
                );
            }
            let rate = self.token_rate(squad)?;
            for (name, value) in [
                ("enabled", json!(rate.enabled)),
                ("every", json!(format!("{}s", rate.every.as_secs()))),
                ("window", json!(rate.window.label())),
                ("reduced_motion", json!(rate.reduced_motion)),
            ] {
                out.push(
                    format!("board.token_rate.{name}"),
                    value,
                    self.setting_source(
                        &["squad", squad, "board", "token_rate", name],
                        &self.setting_source(&["board", "token_rate", name], &preset),
                    ),
                );
            }
        } else {
            out.push("board.token_rate.enabled", json!(false), "aggregate tab");
        }
        let (theme, notice) = self.theme(squad)?;
        if let Some(notice) = notice {
            out.notices.push(notice);
        }
        let theme_source = self.theme_source(squad)?;
        let base_source = theme_path(squad, "base", theme_source);
        out.push("theme.base", json!(theme.base.name()), &base_source);
        if theme.base == tmt_cli_style::Base::Auto {
            out.notices
                .push("auto matches the terminal when the board opens".into());
        }
        for role in tmt_cli_style::Role::ALL {
            let name = role.name();
            let (value, source) = self.theme_setting(squad, name)?;
            let source = if value.is_some() {
                theme_path(squad, name, source)
            } else {
                base_source.clone()
            };
            out.push(
                format!("theme.{name}"),
                json!(value.unwrap_or_else(|| format!("base:{}", theme.base.name()))),
                source,
            );
        }
        for (name, value) in [
            ("order", json!(policy.order)),
            ("pin", json!(policy.pin)),
            ("hide", json!(policy.hide)),
            ("colors.waiting", json!(policy.colors.waiting)),
            ("colors.blocked", json!(policy.colors.blocked)),
        ] {
            let path: Vec<_> = std::iter::once("tabs").chain(name.split('.')).collect();
            out.push(
                format!("tabs.{name}"),
                value,
                self.setting_source(&path, "tab defaults"),
            );
        }
        if key != tabs::ALL {
            for name in ["opener", "clipboard"] {
                out.push(
                    name,
                    json!(self.program(name)?),
                    self.setting_source(&[name], "system default"),
                );
            }
        }
        let sections = if let Some(tab) = user {
            tab.sections.clone()
        } else if aggregate {
            Vec::new()
        } else {
            self.sections(squad)?
        };
        let bindings = crate::action::effective_bindings(
            self.bindings_for_tab(key, tmux, &board.panes)?,
            section
                .and_then(|index| sections.get(index))
                .map(|section| &section.bind),
            !aggregate && self.token_rate(squad)?.enabled,
        );
        let tab_name = if key == tabs::ALL {
            Some("all")
        } else if key == tabs::LEADS {
            Some("leads")
        } else {
            tabs::user_name(key)
        };
        for (event, action) in bindings {
            let fallback = if key == tabs::ALL {
                "all-tab preset"
            } else if tmux {
                "host preset:tmux"
            } else {
                "host preset:plain"
            };
            let global = if key == tabs::ALL {
                fallback.into()
            } else {
                self.setting_source(&["bind", &event], fallback)
            };
            let mut source = tab_name.map_or(global.clone(), |tab| {
                self.setting_source(&["tabs", tab, "bind", &event], &global)
            });
            if let Some(index) = section.filter(|index| {
                sections
                    .get(*index)
                    .is_some_and(|s| s.bind.contains_key(&event))
            }) {
                source = format!(
                    "{}.section[{index}].bind.{event}",
                    tab_name.map_or_else(|| format!("squad.{squad}"), |tab| format!("tabs.{tab}"))
                );
            }
            out.push(format!("bind.{event}"), json!(action.text), source);
        }
        for (index, section) in sections.iter().enumerate() {
            for (event, action) in &section.bind {
                out.push(
                    format!("section[{index}].bind.{event}"),
                    json!(action.text),
                    tab_name.map_or_else(
                        || format!("squad.{squad}.section[{index}].bind.{event}"),
                        |tab| format!("tabs.{tab}.section[{index}].bind.{event}"),
                    ),
                );
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(text: &str) -> Config {
        Config {
            path: "/fixture/squad.toml".into(),
            original: None,
            document: text.parse().unwrap(),
            global_theme: Vec::new(),
            theme_error: None,
        }
    }
    fn entry<'a>(shown: &'a BoardSettings, key: &str) -> &'a crate::settings::BoardSetting {
        shown.entries.iter().find(|entry| entry.key == key).unwrap()
    }
    #[test]
    fn sources_follow_independent_layers_and_preserve_the_document() {
        let cfg = config(
            "# kept\n[board]\nview='notes'\nrefresh='1m'\n[board.token_rate]\nevery='10s'\nwindow='5s'\n[squad.x.board]\nrefresh='off'\n[squad.x.theme]\nbase='mono'\nworking='blue'\n[bind]\no='run touch /never-execute'\n",
        );
        let before = cfg.document.to_string();
        let shown = cfg.settings(Some("x"), true, None).unwrap();
        assert_eq!(entry(&shown, "board.refresh").value, "off");
        assert_eq!(
            entry(&shown, "board.refresh").source,
            "squad.x.board.refresh"
        );
        assert_eq!(entry(&shown, "board.view").source, "board.view");
        assert_eq!(entry(&shown, "theme.base").source, "squad.x.theme.base");
        assert_eq!(entry(&shown, "theme.working").value, "blue");
        assert_eq!(entry(&shown, "board.token_rate.window").value, "1m");
        assert_eq!(entry(&shown, "fields.pr").value["run"][3], "{pr_link}");
        assert!(shown.entries.iter().all(|entry| !entry.editable));
        assert_eq!(entry(&shown, "rows").value, cfg.rows("x").unwrap().value());
        let custom = config("[squad.x.board]\npanes=['rows','notes']\n")
            .settings(Some("x"), false, None)
            .unwrap();
        assert_eq!(entry(&custom, "board.mode").source, "preset:crew");
        assert_eq!(entry(&custom, "board.collapsed").source, "default:[]");
        assert!(
            entry(&custom, "board.layout")
                .source
                .starts_with("squad.x.board")
        );
        assert_eq!(cfg.document.to_string(), before);
    }
    #[test]
    fn aggregate_and_section_bindings_use_the_same_owners_as_the_board() {
        let cfg = config(
            "[bind]\nenter='run touch /global'\n[tabs.all.bind]\nenter='refresh'\n[tabs.mine]\nfilter='pending'\n[tabs.mine.bind]\nenter='open'\n[[tabs.mine.section]]\ntitle='Pending'\nfilter='pending'\n[tabs.mine.section.bind]\nenter='copy'\n",
        );
        let all = cfg.settings(Some(tabs::ALL), true, None).unwrap();
        assert_eq!(entry(&all, "bind.enter").value, "refresh");
        assert_eq!(entry(&all, "bind.enter").source, "tabs.all.bind.enter");
        assert_eq!(
            entry(&all, "rows").value,
            crate::tab_view::rows(tabs::ALL).value()
        );
        assert!(
            !all.entries
                .iter()
                .any(|entry| entry.key.starts_with("fields."))
        );
        let mine = cfg.settings(Some("@tab:mine"), false, Some(0)).unwrap();
        assert_eq!(entry(&mine, "bind.enter").value, "copy");
        assert_eq!(
            entry(&mine, "bind.enter").source,
            "tabs.mine.section[0].bind.enter"
        );
        assert_eq!(
            entry(&mine, "rows").value,
            crate::tab_view::rows("@tab:mine").value()
        );
        assert_eq!(entry(&all, "bind.,").value, "settings");
        assert!(cfg.settings(Some("@tab:missing"), false, None).is_err());
    }
}
