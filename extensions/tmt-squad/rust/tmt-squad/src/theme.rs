//! Board-only theme commands and scope values shared with the picker.

use crate::{config::Config, core::SquadError, squad};
use clap::{Arg, ArgMatches, Command};
use serde_json::{Value, json};
use tmt_cli_style::{
    Base, CommandSpec, Example, OutputModes, Terminal, Token,
    list::Section,
    message,
    table::{Cell, Column, Table},
};

pub const HINT: &str = "board only; tmt config show shows the CLI theme and its file";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeScope {
    Board,
    Squad(String),
}

impl ThemeScope {
    pub fn squad(&self) -> Option<&str> {
        match self {
            Self::Board => None,
            Self::Squad(name) => Some(name),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Board => "all boards".into(),
            Self::Squad(name) => format!("squad {name}"),
        }
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
        .help("Use this squad's board theme instead of the all-boards default")
}

pub fn grammar() -> Command {
    spec("theme", "Choose a board theme; CLI colours stay unchanged", &[
        Example { command: "tmt squad theme", note: "List themes and mark the current board default" },
        Example { command: "tmt squad theme set tmt-light", note: "Choose a default for all boards" },
    ], "Without a subcommand, list themes. Board choices do not change CLI colours.")
    .arg(scope_option())
    .subcommand(spec("ls", "List board themes and the current base's source", &[
        Example { command: "tmt squad theme ls", note: "List themes for all boards" },
        Example { command: "tmt squad theme ls --squad product", note: "Inspect one squad's effective theme" },
    ], "The base source is default, cli, board or squad; token overrides layer separately.").alias("list").arg(scope_option()))
    .subcommand(spec("set", "Set a board theme base; keep token overrides and CLI colours", &[
        Example { command: "tmt squad theme set tmt-light", note: "Use a light theme for all boards" },
        Example { command: "tmt squad theme set mono --squad product", note: "Choose a theme for one squad" },
    ], "Only the base is replaced in squad.toml; token overrides and CLI colours stay unchanged.")
        .arg(Arg::new("name").required(true).help("Built-in theme name"))
        .arg(scope_option()))
    .subcommand(spec("rm", "Remove a board base override; retain token overrides and CLI colours", &[
        Example { command: "tmt squad theme rm", note: "Let all boards inherit the CLI theme" },
        Example { command: "tmt squad theme rm --squad product", note: "Remove one squad's base override" },
    ], "Removes only base from the selected theme table. Token overrides and CLI colours are retained.").arg(scope_option()))
}

pub fn parse_base(name: &str) -> Result<Base, SquadError> {
    Base::parse(name).ok_or_else(|| {
        SquadError::hinted(
            "SQUAD_THEME_UNKNOWN",
            &format!("Unknown theme '{name}'"),
            "; ",
            &format!(
                "tmt sq theme set <name>; choose {}",
                Base::ALL.map(Base::name).join(", ")
            ),
        )
    })
}

pub fn report(config: &Config, scope: &ThemeScope) -> Result<Value, SquadError> {
    let squad = scope.squad().unwrap_or("");
    let (theme, notice) = config.theme(squad)?;
    let source = config.theme_source(squad)?;
    let mut bases = Base::ALL;
    bases.sort_by_key(|base| base.name());
    Ok(json!({
        "action": "ls", "scope": scope.squad(), "boardOnly": true,
        "effective": { "base": theme.base.name(), "source": source },
        "themes": bases.map(|base| json!({ "name": base.name(), "description": base.description(), "current": base == theme.base })),
        "notice": notice,
    }))
}

pub fn run(config: &mut Config, flags: &ArgMatches) -> Result<Value, SquadError> {
    let parent = flags;
    let (action, flags) = flags.subcommand().unwrap_or(("ls", flags));
    let scope = ThemeScope::parse(
        flags
            .get_one::<String>("squad")
            .or_else(|| parent.get_one::<String>("squad"))
            .map(String::as_str),
    )?;
    match action {
        "set" => {
            let base = parse_base(flags.get_one::<String>("name").expect("required name"))?;
            let changed = config.set_theme_base(&scope, base)?;
            Ok(
                json!({ "action": "set", "scope": scope.squad(), "base": base.name(), "changed": changed, "boardOnly": true }),
            )
        }
        "rm" => {
            let changed = config.remove_theme_base(&scope)?;
            Ok(
                json!({ "action": "rm", "scope": scope.squad(), "changed": changed, "boardOnly": true }),
            )
        }
        _ => report(config, &scope),
    }
}

pub fn text(document: &Value, terminal: Terminal) -> String {
    let scope = document["scope"]
        .as_str()
        .map_or(ThemeScope::Board, |name| ThemeScope::Squad(name.into()));
    let mut output = Vec::new();
    match document["action"].as_str() {
        Some("ls") => {
            let mut rows =
                Table::new(&[Column::Fixed, Column::Name, Column::Detail, Column::Fixed]);
            for base in document["themes"].as_array().into_iter().flatten() {
                let current = base["current"] == true;
                rows.row([
                    Cell::from(if current { "●" } else { "" }),
                    Cell::from(base["name"].as_str().unwrap_or_default()),
                    Cell::styled(base["description"].as_str().unwrap_or_default(), Token::Dim),
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
            let _ = Section {
                title: "THEMES",
                count: Some(Base::ALL.len()),
                rows,
                note: document["notice"].as_str(),
                hint: Some(HINT),
            }
            .write(&mut output, terminal);
        }
        Some("set") => {
            let base = document["base"].as_str().unwrap_or_default();
            let verb = if document["changed"] == true {
                "Set"
            } else {
                "Kept"
            };
            let _ = message::success(
                &mut output,
                terminal,
                &format!("{verb} theme {base} for {} (board only)", scope.label()),
            );
        }
        Some("rm") => {
            let text = if document["changed"] == true {
                format!(
                    "Removed board theme for {}; token overrides kept",
                    scope.label()
                )
            } else {
                format!(
                    "No board theme override for {}; token overrides kept",
                    scope.label()
                )
            };
            let _ = message::success(&mut output, terminal, &text);
        }
        _ => unreachable!("theme output action"),
    }
    String::from_utf8(output).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_list_alias_and_scoped_list_have_consistent_human_and_json_results() {
        let directory = std::env::temp_dir().join(format!("tmt-theme-list-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("squad.toml");
        std::fs::write(
            &path,
            "[board.theme]\nbase = \"terminal\"\n[squad.product.theme]\nbase = \"mono\"\n",
        )
        .unwrap();
        let mut config = Config::read(path).unwrap();
        let run_words = |config: &mut Config, words: &[&str]| {
            let argv: Vec<_> = std::iter::once("squad")
                .chain(words.iter().copied())
                .collect();
            let parsed = crate::grammar().try_get_matches_from(argv).unwrap();
            run(config, parsed.subcommand().unwrap().1).unwrap()
        };
        let bare = run_words(&mut config, &["theme"]);
        assert_eq!(bare, run_words(&mut config, &["theme", "ls"]));
        assert_eq!(bare, run_words(&mut config, &["theme", "list", "--json"]));
        assert_eq!(
            bare["effective"],
            json!({"base":"terminal", "source":"board"})
        );
        assert_eq!(bare["themes"].as_array().unwrap().len(), Base::ALL.len());
        assert_eq!(
            bare["themes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|base| base["current"] == true)
                .count(),
            1
        );
        let human = text(&bare, Terminal::PLAIN);
        assert!(human.starts_with("THEMES 4\n"));
        assert_eq!(human.matches('●').count(), 1);
        assert_eq!(human.matches("board\n").count(), 1);
        assert!(human.ends_with(&format!("hint: {HINT}\n")));
        let scoped = run_words(&mut config, &["theme", "ls", "--squad", "product"]);
        assert_eq!(
            scoped,
            run_words(&mut config, &["theme", "--squad", "product", "ls"])
        );
        assert_eq!(
            scoped["effective"],
            json!({"base":"mono", "source":"squad"})
        );
        let set = run_words(
            &mut config,
            &["theme", "set", "tmt-light", "--squad", "product"],
        );
        assert_eq!(
            text(&set, Terminal::PLAIN),
            "✓ Set theme tmt-light for squad product (board only)\n"
        );
        let kept = run_words(
            &mut config,
            &["theme", "set", "tmt-light", "--squad", "product"],
        );
        assert_eq!(
            text(&kept, Terminal::PLAIN),
            "✓ Kept theme tmt-light for squad product (board only)\n"
        );
        let removed = run_words(&mut config, &["theme", "rm", "--squad", "product"]);
        assert_eq!(
            text(&removed, Terminal::PLAIN),
            "✓ Removed board theme for squad product; token overrides kept\n"
        );
        let noop = run_words(&mut config, &["theme", "rm", "--squad", "product"]);
        assert_eq!(noop["changed"], false);
        assert!(text(&noop, Terminal::PLAIN).contains("No board theme override"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unknown_theme_reports_the_registry_and_never_edits() {
        let error = parse_base("dark").unwrap_err();
        let (message, hint) = error.human();
        assert_eq!(message, "Unknown theme 'dark'");
        let hint = hint.unwrap();
        for base in Base::ALL {
            assert!(hint.contains(base.name()));
        }
    }
}
