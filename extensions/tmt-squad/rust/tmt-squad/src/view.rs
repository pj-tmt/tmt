//! Factory pane arrangements and board-only commands; workflow layouts stay independent.

use crate::{config::Config, core::SquadError, squad};
use clap::{Arg, ArgMatches, Command};
use serde_json::{Value, json};
use tmt_cli_style::{
    CommandSpec, Example, OutputModes, Terminal, Token,
    list::Section,
    message,
    table::{Cell, Column, Table},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewName {
    Members,
    Team,
    Focus,
    Notes,
    Detail,
    Wide,
}

impl ViewName {
    pub const ALL: [Self; 6] = [
        Self::Members,
        Self::Team,
        Self::Focus,
        Self::Notes,
        Self::Detail,
        Self::Wide,
    ];
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.name() == name)
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Members => "members",
            Self::Team => "team",
            Self::Focus => "focus",
            Self::Notes => "notes",
            Self::Detail => "detail",
            Self::Wide => "wide",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::Members => "boxed members, inline detail and lead notes",
            Self::Team => "previous side panes: detail, replies and notes",
            Self::Focus => "rows only",
            Self::Notes => "read the lead's notes",
            Self::Detail => "one member up close",
            Self::Wide => "three columns, 180+",
        }
    }
    /// One catalog owns all factory arrangements in the existing split/fold grammar.
    pub fn settings(self) -> &'static dyn toml_edit::TableLike {
        static VIEWS: std::sync::OnceLock<toml_edit::DocumentMut> = std::sync::OnceLock::new();
        VIEWS.get_or_init(|| r#"
[members]
layout = { direction = "top-bottom", sizes = [60, 40], panes = ["rows", "notes"] }
[team]
fold_below = { width = 100, panes = ["detail", "replies"] }
layout = { direction = "top-bottom", sizes = [60, 40], panes = [{ direction = "left-right", sizes = [62, 38], panes = ["rows", { direction = "top-bottom", sizes = [50, 50], panes = ["detail", "replies"] }] }, "notes"] }
[focus]
layout = { direction = "top-bottom", sizes = [70, 10, 10, 10], panes = ["rows", "detail", "replies", "notes"] }
collapsed = ["detail", "replies", "notes"]
[notes]
layout = { direction = "top-bottom", sizes = [80, 10, 10], panes = [{ direction = "left-right", sizes = [35, 65], panes = ["rows", "notes"] }, "detail", "replies"] }
collapsed = ["detail", "replies"]
[detail]
layout = { direction = "top-bottom", sizes = [90, 10], panes = [{ direction = "top-bottom", sizes = [35, 65], panes = ["rows", { direction = "left-right", sizes = [50, 50], panes = ["detail", "replies"] }] }, "notes"] }
collapsed = ["notes"]
fold_below = { width = 100, panes = ["replies"] }
[wide]
layout = { direction = "left-right", sizes = [40, 30, 30], panes = ["rows", { direction = "top-bottom", sizes = [50, 50], panes = ["detail", "replies"] }, "notes"] }
fold_below = { width = 180, panes = ["detail", "replies"] }
"#.parse().expect("factory views are valid TOML"))[self.name()].as_table_like().expect("factory view table")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewScope {
    Board,
    Squad(String),
}
impl ViewScope {
    pub fn squad(&self) -> Option<&str> {
        match self {
            Self::Board => None,
            Self::Squad(name) => Some(name),
        }
    }
    pub fn label(&self) -> String {
        self.squad()
            .map_or_else(|| "all boards".into(), |name| format!("squad {name}"))
    }
    fn parse(name: Option<&str>) -> Result<Self, SquadError> {
        match name {
            Some(name) if squad::valid_name(name) => Ok(Self::Squad(name.into())),
            Some(name) => Err(squad::name_invalid(name)),
            None => Ok(Self::Board),
        }
    }
}

fn spec(
    name: &'static str,
    summary: &'static str,
    examples: &'static [Example],
    details: &'static str,
) -> Command {
    tmt_cli_style::command(&CommandSpec {
        name,
        summary,
        examples,
        outputs: OutputModes::Human,
        details,
    })
}
fn scope_option() -> Arg {
    Arg::new("squad")
        .long("squad")
        .value_name("NAME")
        .help("Use this squad's view instead of the all-boards default")
}
pub fn grammar() -> Command {
    spec("view", "Choose a board pane arrangement and fold defaults", &[
        Example { command: "tmt squad view", note: "List factory views" },
        Example { command: "tmt squad view set focus", note: "Choose a view for all boards" },
    ], "Without a subcommand, list views. Views change panes and folds only; workflow states, rows and providers stay unchanged.")
    .arg(scope_option())
    .subcommand(spec("ls", "List factory views and the effective arrangement's source", &[
        Example { command: "tmt squad view ls", note: "List views for all boards" },
        Example { command: "tmt squad view ls --squad product", note: "Inspect one squad's view" },
    ], "Custom layouts take precedence; views arrange panes and fold defaults only.").alias("list").arg(scope_option()))
    .subcommand(spec("set", "Set a board view without changing workflow settings", &[
        Example { command: "tmt squad view set notes", note: "Read lead notes on all boards" },
        Example { command: "tmt squad view set wide --squad product", note: "Choose one squad's view" },
    ], "Writes only view in squad.toml. A hand-written board.layout or panes must be removed manually first.").arg(Arg::new("name").required(true).help("Factory view name")).arg(scope_option()))
    .subcommand(spec("rm", "Remove only a view override and inherit the arrangement", &[
        Example { command: "tmt squad view rm", note: "Reset the all-boards view" },
        Example { command: "tmt squad view rm --squad product", note: "Reset one squad's view" },
    ], "Removes only view from the chosen layer; custom layouts and all other settings are retained.").arg(scope_option()))
}

pub fn run(config: &mut Config, parent: &ArgMatches) -> Result<Value, SquadError> {
    let (action, flags) = parent.subcommand().unwrap_or(("ls", parent));
    let scope = ViewScope::parse(
        flags
            .get_one::<String>("squad")
            .or_else(|| parent.get_one::<String>("squad"))
            .map(String::as_str),
    )?;
    match action {
        "set" => {
            let name = flags.get_one::<String>("name").expect("required name");
            let view = ViewName::parse(name).ok_or_else(|| {
                SquadError::hinted(
                    "SQUAD_VIEW_UNKNOWN",
                    &format!("Unknown view '{name}'"),
                    "; ",
                    "tmt sq view ls lists factory views",
                )
            })?;
            let changed = config.set_view(&scope, view)?;
            Ok(
                json!({ "action": "set", "scope": scope.squad(), "view": view.name(), "changed": changed }),
            )
        }
        "rm" => Ok(
            json!({ "action": "rm", "scope": scope.squad(), "changed": config.remove_view(&scope)? }),
        ),
        _ => {
            let squad = scope.squad().unwrap_or("");
            config.board(squad)?;
            let (view, source) = config.view_source(squad)?;
            Ok(
                json!({ "action": "ls", "scope": scope.squad(), "effective": { "view": view.map(ViewName::name), "source": source, "layout": config.layout(squad)?.as_str() }, "views": ViewName::ALL.map(|entry| json!({ "name": entry.name(), "description": entry.description(), "current": Some(entry) == view })) }),
            )
        }
    }
}

pub fn text(document: &Value, terminal: Terminal) -> String {
    let scope = document["scope"]
        .as_str()
        .map_or(ViewScope::Board, |name| ViewScope::Squad(name.into()));
    let mut output = Vec::new();
    if document["action"] == "ls" {
        let mut rows = Table::new(&[Column::Fixed, Column::Name, Column::Detail, Column::Fixed]);
        for view in document["views"].as_array().into_iter().flatten() {
            let current = view["current"] == true;
            rows.row([
                Cell::from(if current { "●" } else { "" }),
                Cell::from(view["name"].as_str().unwrap_or_default()),
                Cell::styled(view["description"].as_str().unwrap_or_default(), Token::Dim),
                Cell::styled(
                    if current {
                        document["effective"]["source"].as_str().unwrap_or_default()
                    } else {
                        ""
                    },
                    Token::Dim,
                ),
            ]);
        }
        let note = format!(
            "{}workflow: {}",
            if document["effective"]["source"] == "custom" {
                "custom layout; "
            } else {
                ""
            },
            document["effective"]["layout"].as_str().unwrap_or_default()
        );
        let _ = Section {
            title: "VIEWS",
            count: Some(ViewName::ALL.len()),
            rows,
            note: Some(&note),
            hint: Some("tmt sq view set <name> [--squad <name>]"),
        }
        .write(&mut output, terminal);
    } else {
        let changed = document["changed"] == true;
        let text = if document["action"] == "set" {
            format!(
                "{} view {} for {}",
                if changed { "Set" } else { "Kept" },
                document["view"].as_str().unwrap_or_default(),
                scope.label()
            )
        } else if changed {
            format!(
                "Removed view override for {}; inherited arrangement applies",
                scope.label()
            )
        } else {
            format!(
                "Kept {} on the inherited arrangement; no override to remove",
                scope.label()
            )
        };
        let _ = message::success(&mut output, terminal, &text);
    }
    String::from_utf8(output).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BoardMode, Pane};
    fn fixture(name: &str, text: &str) -> (std::path::PathBuf, Config) {
        let dir = std::env::temp_dir().join(format!("tmt-view-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("squad.toml");
        std::fs::write(&path, text).unwrap();
        let config = Config::read(path.clone()).unwrap();
        (path, config)
    }
    fn words(config: &mut Config, args: &[&str]) -> Value {
        let argv: Vec<_> = ["squad", "view"]
            .into_iter()
            .chain(args.iter().copied())
            .collect();
        let parsed = crate::grammar().try_get_matches_from(argv).unwrap();
        run(config, parsed.subcommand().unwrap().1).unwrap()
    }
    #[test]
    fn command_aliases_scopes_sources_and_reset_preserve_unrelated_bytes() {
        let original = "# user file\n[board]\nview = 'notes' # retain\nrefresh = 'off'\n[squad.product]\nlayout = 'crew'\n[squad.product.board]\nview = 'focus' # own\n[squad.other]\nlayout = 'minimal'\n";
        let (path, mut config) = fixture("cli", original);
        let listed = words(&mut config, &[]);
        assert_eq!(listed, words(&mut config, &["ls"]));
        assert_eq!(listed, words(&mut config, &["list", "--json"]));
        assert_eq!(listed["effective"]["view"], "notes");
        let human = text(&listed, Terminal::PLAIN);
        assert!(human.starts_with("VIEWS 6\n"));
        assert_eq!(human.matches("board\n").count(), 1);
        assert!(human.contains("workflow: team\n"));
        assert!(human.ends_with("hint: tmt sq view set <name> [--squad <name>]\n"));
        let scoped = words(&mut config, &["ls", "--squad", "product"]);
        assert_eq!(scoped["effective"]["source"], "squad");
        assert_eq!(scoped["effective"]["layout"], "crew");
        assert_eq!(
            words(&mut config, &["set", "wide", "--squad", "product"])["changed"],
            true
        );
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            written,
            original.replace("view = 'focus' # own", "view = \"wide\" # own")
        );
        assert_eq!(
            words(&mut config, &["set", "wide", "--squad", "product"])["changed"],
            false
        );
        assert_eq!(
            words(&mut config, &["rm", "--squad", "product"])["changed"],
            true
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original
                .replace("view = 'focus' # own\n", "")
                .replace("[squad.product.board]\n", "")
        );
        assert_eq!(
            words(&mut config, &["ls", "--squad", "product"])["effective"]["view"],
            "notes"
        );
        words(&mut config, &["rm"]);
        assert_eq!(
            config.board("product").unwrap().panes,
            vec![Pane::Rows, Pane::Notes]
        );
        assert_eq!(
            config.board("other").unwrap().panes,
            vec![Pane::Rows, Pane::Notes]
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original
                .replace("view = 'focus' # own\n", "")
                .replace("view = 'notes' # retain\n", "")
                .replace("[squad.product.board]\n", "")
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn factory_views_use_the_split_reader_without_changing_workflow_projection() {
        for workflow in ["team", "crew", "pr-queue", "minimal"] {
            let (path, mut config) = fixture(
                &format!("projection-{workflow}"),
                &format!(
                    "[squad.product]\nlayout = '{workflow}'\n[squad.product.rows]\ncolumns = [{{ name = 'member', width = 16 }}, {{ name = 'task', grow = 1 }}]\nlines = [[{{ field = 'member' }}, {{ field = 'task' }}]]\n"
                ),
            );
            let baseline = (
                config.layout("product").unwrap(),
                format!("{:?}", config.rows("product").unwrap()),
                format!("{:?}", config.providers("product").unwrap()),
                config.reminders("product").unwrap(),
                config.token_rate("product").unwrap(),
                config
                    .states("product", config.layout("product").unwrap())
                    .unwrap(),
            );
            for name in ViewName::ALL {
                config
                    .set_view(&ViewScope::Squad("product".into()), name)
                    .unwrap();
                assert_eq!(
                    baseline,
                    (
                        config.layout("product").unwrap(),
                        format!("{:?}", config.rows("product").unwrap()),
                        format!("{:?}", config.providers("product").unwrap()),
                        config.reminders("product").unwrap(),
                        config.token_rate("product").unwrap(),
                        config
                            .states("product", config.layout("product").unwrap())
                            .unwrap()
                    )
                );
                let board = config.board("product").unwrap();
                assert_eq!(board.mode, BoardMode::Split);
                assert_eq!(
                    board.panes.len(),
                    if name == ViewName::Members { 2 } else { 4 }
                );
                assert!(board.panes.contains(&Pane::Rows));
                for width in [80, 120, 200] {
                    let mut folds = board.collapsed.clone();
                    if let Some(rule) = &board.fold_below
                        && width < rule.width
                    {
                        folds.extend(&rule.panes);
                    }
                    let expected: std::collections::BTreeSet<_> = match name {
                        ViewName::Team if width < 100 => [Pane::Detail, Pane::Replies].into(),
                        ViewName::Team => Default::default(),
                        ViewName::Focus => [Pane::Detail, Pane::Replies, Pane::Notes].into(),
                        ViewName::Notes => [Pane::Detail, Pane::Replies].into(),
                        ViewName::Detail if width < 100 => [Pane::Notes, Pane::Replies].into(),
                        ViewName::Detail => [Pane::Notes].into(),
                        ViewName::Wide if width < 180 => [Pane::Detail, Pane::Replies].into(),
                        ViewName::Members | ViewName::Wide => Default::default(),
                    };
                    assert_eq!(folds, expected, "{} at {width}", name.name());
                    let solved = crate::board::pane_rectangles(
                        &board,
                        &folds,
                        ratatui::layout::Rect::new(0, 0, width, 30),
                    );
                    assert_eq!(solved.len(), board.panes.len());
                    assert!(
                        solved
                            .iter()
                            .all(|(_, rect)| rect.right() <= width && rect.bottom() <= 30)
                    );
                    let row = solved
                        .iter()
                        .find(|(pane, _)| *pane == Pane::Rows)
                        .unwrap()
                        .1;
                    assert!(row.width > 0 && row.height > 0);
                    if name == ViewName::Focus {
                        assert_eq!(row.height, 27);
                    }
                }
            }
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
    }
    #[test]
    fn custom_precedence_refusal_reset_and_stale_noop_are_safe() {
        let original = "[board]\nview = 'wide'\n[squad.product.board]\npanes = ['rows', 'notes'] # custom\nview = 'focus'\n";
        let (path, mut config) = fixture("custom", original);
        let scope = ViewScope::Squad("product".into());
        assert_eq!(config.view_source("product").unwrap(), (None, "custom"));
        assert!(
            text(
                &words(&mut config, &["ls", "--squad", "product"]),
                Terminal::PLAIN
            )
            .contains("custom layout; workflow: crew")
        );
        assert_eq!(
            config.board("product").unwrap().panes,
            vec![Pane::Rows, Pane::Notes]
        );
        assert_eq!(
            config.set_view(&scope, ViewName::Focus).unwrap_err().code,
            "SQUAD_VIEW_CUSTOM"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        config.remove_view(&scope).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original.replace("view = 'focus'\n", "")
        );
        std::fs::write(&path, "# concurrent edit\n").unwrap();
        assert_eq!(
            config.remove_view(&scope).unwrap_err().code,
            "SQUAD_CONFIG_CHANGED"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# concurrent edit\n"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn reset_drops_only_a_table_emptied_by_reset_and_preserves_header_comments() {
        for (name, original, expected) in [
            ("plain", "[board]\nview = 'focus'\n", ""),
            (
                "decorated",
                "# user table\n[board] # keep\nview = 'focus'\n",
                "# user table\n[board] # keep\n",
            ),
            ("empty", "[board]\n", "[board]\n"),
        ] {
            let (path, mut config) = fixture(name, original);
            config.remove_view(&ViewScope::Board).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
    }
    #[test]
    fn invalid_masked_names_fail_before_writing_and_default_matches_members() {
        for text in [
            "[board]\nview = 'bogus'\n[squad.product.board]\nview = 'focus'\n",
            "[squad.product.board]\nview = 4\npanes = ['rows']\n",
        ] {
            let dir = std::env::temp_dir().join(format!("tmt-view-invalid-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("squad.toml");
            std::fs::write(&path, text).unwrap();
            assert_eq!(
                Config::read(path.clone()).err().unwrap().code,
                "SQUAD_CONFIG_INVALID"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
            std::fs::remove_dir_all(dir).unwrap();
        }
        let (path, mut config) = fixture("default-members", "");
        let baseline = config.board("product").unwrap();
        config
            .set_view(&ViewScope::Board, ViewName::Members)
            .unwrap();
        assert_eq!(config.board("product").unwrap(), baseline);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
