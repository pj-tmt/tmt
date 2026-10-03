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
    pub fn settings(
        &self,
        context: Option<&str>,
        tmux: bool,
        section: Option<usize>,
    ) -> Result<BoardSettings, SquadError> {
        let key = context.unwrap_or("");
        if let Some(name) = tabs::user_name(key)
            && !self.tabs()?.user.iter().any(|tab| tab.name == name)
        {
            return Err(SquadError::new("SQUAD_TAB_NOT_FOUND", "Unknown tab key."));
        }
        let mut out = BoardSettings {
            path: self.path.display().to_string(),
            context: context.map(str::to_owned),
            host: if tmux { "tmux" } else { "plain" },
            entries: Vec::new(),
            notices: Vec::new(),
        };
        self.arrangement_settings(key, &mut out)?;
        self.row_settings(key, &mut out)?;
        self.notebook_settings(key, &mut out)?;
        self.meter_settings(key, &mut out)?;
        self.theme_settings(key, &mut out)?;
        self.policy_settings(key, &mut out)?;
        let board = if tabs::aggregate(key) {
            Board::simple(
                BoardMode::Split,
                Direction::LeftRight,
                vec![Pane::Rows],
                &[100],
            )
        } else {
            self.board(key)?
        };
        for (key, action, source) in self.binding_settings(key, tmux, &board.panes, section)? {
            out.push(key, json!(action.text), source);
        }
        Ok(out)
    }
    fn arrangement_settings(&self, key: &str, out: &mut BoardSettings) -> Result<(), SquadError> {
        let aggregate = tabs::aggregate(key);
        let (board, sources) = if aggregate {
            (
                Board::simple(
                    BoardMode::Split,
                    Direction::LeftRight,
                    vec![Pane::Rows],
                    &[100],
                ),
                ["mode", "layout", "panes", "collapsed", "fold_below"]
                    .into_iter()
                    .map(|key| (key.into(), "aggregate tab".into()))
                    .collect(),
            )
        } else {
            self.board_setting(key)?
        };
        if !aggregate {
            let (layout, source) = self.layout_setting(key)?;
            out.push("layout", json!(layout.as_str()), source);
            out.push(
                "board.view",
                json!(self.view_source(key)?.0.map(|view| view.name())),
                &sources["view"],
            );
        }
        for (name, value) in [("mode", json!(if board.mode == BoardMode::Split { "split" } else { "tabs" })), ("layout", split_value(&board.split)), ("panes", json!(board.panes.iter().map(|p| p.title()).collect::<Vec<_>>())), ("collapsed", json!(board.collapsed.iter().map(|p| p.title()).collect::<Vec<_>>())), ("fold_below", json!(board.fold_below.as_ref().map(|f| json!({"width":f.width,"panes":f.panes.iter().map(|p| p.title()).collect::<Vec<_>>()}))))] { out.push(format!("board.{name}"), value, &sources[name]); }
        let (refresh, source) = self.refresh_setting(key)?;
        out.push(
            "board.refresh",
            json!(refresh.map_or_else(|| "off".into(), |d| format!("{}s", d.as_secs()))),
            source,
        );
        Ok(())
    }
    fn row_settings(&self, key: &str, out: &mut BoardSettings) -> Result<(), SquadError> {
        if tabs::aggregate(key) {
            out.push("rows", crate::tab_view::rows(key).value(), "aggregate tab");
        } else {
            let (rows, source, _) = self.rows_setting(key)?;
            out.push("rows", rows.value(), source);
        }
        Ok(())
    }
    fn notebook_settings(&self, key: &str, out: &mut BoardSettings) -> Result<(), SquadError> {
        if tabs::aggregate(key) {
            out.push("notes.render", json!("markdown"), "aggregate tab");
            return Ok(());
        }
        let (render, source) = self.notes_setting(key)?;
        out.push(
            "notes.render",
            json!(if render == NotesRender::Plain {
                "plain"
            } else {
                "markdown"
            }),
            source,
        );
        for (key, value, source) in self.state_settings(key)? {
            out.push(key, value, source);
        }
        let (reminders, sources) = self.reminder_settings(key)?;
        out.push(
            "reminders.enabled",
            json!(reminders.enabled),
            &sources["enabled"],
        );
        out.push(
            "reminders.stale_after",
            json!(format!("{}s", reminders.stale_after.as_secs())),
            &sources["stale_after"],
        );
        for (provider, source) in self.provider_settings(key)? {
            out.push(
                format!("fields.{}", provider.name),
                provider.settings(),
                source,
            );
        }
        Ok(())
    }
    fn meter_settings(&self, key: &str, out: &mut BoardSettings) -> Result<(), SquadError> {
        if tabs::aggregate(key) {
            out.push("board.token_rate.enabled", json!(false), "aggregate tab");
        } else {
            let (rate, sources) = self.rate_setting(key)?;
            for (name, value) in [
                ("enabled", json!(rate.enabled)),
                ("every", json!(format!("{}s", rate.every.as_secs()))),
                ("window", json!(rate.window.label())),
                ("reduced_motion", json!(rate.reduced_motion)),
            ] {
                out.push(format!("board.token_rate.{name}"), value, &sources[name]);
            }
        }
        Ok(())
    }
    fn theme_settings(&self, key: &str, out: &mut BoardSettings) -> Result<(), SquadError> {
        let squad = if tabs::aggregate(key) { "" } else { key };
        let (theme, notice) = self.theme(squad)?;
        if let Some(notice) = notice {
            out.notices.push(notice);
        }
        let base_source = theme_path(squad, "base", self.theme_source(squad)?);
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
        Ok(())
    }
    fn policy_settings(&self, key: &str, out: &mut BoardSettings) -> Result<(), SquadError> {
        let (policy, sources) = self.tab_settings()?;
        for (name, value) in [
            ("order", json!(policy.order)),
            ("pin", json!(policy.pin)),
            ("hide", json!(policy.hide)),
            ("colors.waiting", json!(policy.colors.waiting)),
            ("colors.blocked", json!(policy.colors.blocked)),
        ] {
            out.push(format!("tabs.{name}"), value, &sources[name]);
        }
        if key != tabs::ALL {
            for name in ["opener", "clipboard"] {
                let (value, source) = self.program_setting(name)?;
                out.push(name, json!(value), source);
            }
        }
        Ok(())
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
