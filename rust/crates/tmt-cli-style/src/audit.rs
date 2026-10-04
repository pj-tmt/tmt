//! The grammar walk: every visible command in a CLI follows the help rules in
//! `design/cli-style.md`, and its examples parse through the CLI's real grammar.
//! The CLI supplies a [`Probe`] over its own parser and help rendering, so the
//! walk checks what a user sees without dispatching any command.

use crate::help::examples;
use clap::Command;
use std::collections::{BTreeMap, BTreeSet};

/// A CLI's real, non-dispatching entry points. Arguments exclude `program`.
pub struct Probe<'a> {
    /// The words that invoke the CLI, such as `["tmt"]` or `["tmt", "sq"]`.
    /// Every example starts with them.
    pub program: &'a [&'a str],
    /// The help text a user sees for `arguments`, or why it is not help.
    pub help: &'a dyn Fn(&[String]) -> Result<String, String>,
    /// Parses `arguments` through the real grammar without running them.
    pub parse: &'a dyn Fn(&[String]) -> Result<(), String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rule {
    /// The command has a one-line summary.
    Summary,
    /// `-h`, `--help` and `help <command>` print the same text.
    HelpForms,
    /// Summary, `Usage`, `Commands`, `Arguments`, `Options`, other
    /// sections, an optional `Details` just before `Examples`, then
    /// `Examples` last.
    Template,
    /// One to three well-formed examples.
    Examples,
    /// An example invokes this command.
    ExampleTarget,
    /// An example parses through the real grammar.
    ExampleParse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub path: Vec<String>,
    pub rule: Rule,
    pub detail: String,
}

impl Violation {
    /// The command as a user types it, such as `tmt identity show`.
    pub fn command(&self, program: &[&str]) -> String {
        command_name(program, &self.path)
    }
}

fn command_name(program: &[&str], path: &[String]) -> String {
    program
        .iter()
        .copied()
        .chain(path.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every visible command under `root`, `root` included, with its violations.
pub fn walk(root: &Command, probe: &Probe) -> Vec<Violation> {
    let mut violations = Vec::new();
    visit(root, Vec::new(), probe, &mut violations);
    violations
}

/// The commands with at least one violation, as a user types them.
pub fn failing(violations: &[Violation], program: &[&str]) -> BTreeSet<String> {
    violations
        .iter()
        .map(|violation| violation.command(program))
        .collect()
}

/// What to change so a migration allowlist equals the walk's result: a
/// command breaking rules other than those listed, or a listed command that
/// now follows the style. Empty when they agree, so the list only shrinks.
pub fn allowlist_report(
    violations: &[Violation],
    program: &[&str],
    allowlist: &[(&str, &[Rule])],
) -> Vec<String> {
    let mut failing: BTreeMap<String, BTreeSet<Rule>> = BTreeMap::new();
    for violation in violations {
        failing
            .entry(violation.command(program))
            .or_default()
            .insert(violation.rule);
    }
    let listed: BTreeMap<String, BTreeSet<Rule>> = allowlist
        .iter()
        .map(|(command, rules)| ((*command).to_owned(), rules.iter().copied().collect()))
        .collect();
    let mut report: Vec<String> = failing
        .iter()
        .filter(|(command, rules)| listed.get(*command) != Some(*rules))
        .map(|(command, rules)| format!("list ({command:?}, &{:?})", Vec::from_iter(rules)))
        .collect();
    report.extend(
        listed
            .keys()
            .filter(|command| !failing.contains_key(*command))
            .map(|command| format!("remove {command:?}: it now follows the style")),
    );
    report
}

/// Every visible command path under `root`, `root` included.
pub fn commands(root: &Command, program: &[&str]) -> BTreeSet<String> {
    fn collect(command: &Command, path: Vec<String>, program: &[&str], out: &mut BTreeSet<String>) {
        out.insert(command_name(program, &path));
        for child in command
            .get_subcommands()
            .filter(|child| !child.is_hide_set())
        {
            let mut path = path.clone();
            path.push(child.get_name().to_owned());
            collect(child, path, program, out);
        }
    }
    let mut out = BTreeSet::new();
    collect(root, Vec::new(), program, &mut out);
    out
}

/// Recursively enforce the public listing spelling, including nested extension
/// trees: `ls` is primary and `list` remains an accepted, hidden alias.
pub fn list_spelling_report(root: &Command, program: &[&str]) -> Vec<String> {
    fn visit(command: &Command, path: String, out: &mut Vec<String>) {
        let listing = command.get_name() == "list"
            || command.get_name() == "ls"
            || command.get_all_aliases().any(|alias| alias == "list");
        if listing
            && (command.get_name() != "ls"
                || !command.get_all_aliases().any(|alias| alias == "list")
                || command.get_visible_aliases().any(|alias| alias == "list"))
        {
            out.push(format!("{path}: use primary ls with hidden alias list"));
        }
        for child in command.get_subcommands() {
            visit(child, format!("{path} {}", child.get_name()), out);
        }
    }
    let mut out = Vec::new();
    visit(root, program.join(" "), &mut out);
    out
}

/// Hidden subcommands (and any `__` command) in a CLI's grammar. Hidden means
/// "not in help or completion"; it is for protocol entry points that hooks,
/// installers and hosts invoke, never for a user action or a way around the
/// rule that agents call only commands shown in help
/// (`design/cli-style.md#hidden-commands`). Every hidden subcommand, at any
/// depth, needs an `allowlist` entry `(command as typed, reason)`, and every
/// subcommand named `__*` must be hidden. Empty when the grammar and the list
/// agree, so a hidden command cannot appear unreviewed and a removed one
/// cannot leave its entry behind. Hidden aliases and options are not commands
/// and are outside this check.
pub fn hidden_report(root: &Command, program: &[&str], allowlist: &[(&str, &str)]) -> Vec<String> {
    fn visit(
        command: &Command,
        path: Vec<String>,
        inherited: bool,
        program: &[&str],
        found: &mut Vec<String>,
        report: &mut Vec<String>,
    ) {
        for child in command.get_subcommands() {
            let mut path = path.clone();
            path.push(child.get_name().to_owned());
            let hidden = inherited || child.is_hide_set();
            let name = command_name(program, &path);
            if child.get_name().starts_with("__") && !hidden {
                report.push(format!("{name}: named __ but visible; hide it"));
            }
            if hidden {
                found.push(name);
            }
            visit(child, path, hidden, program, found, report);
        }
    }
    let mut found = Vec::new();
    let mut report = Vec::new();
    visit(root, Vec::new(), false, program, &mut found, &mut report);
    let mut listed = BTreeSet::new();
    for (command, reason) in allowlist {
        if !listed.insert(*command) {
            report.push(format!("{command:?}: listed twice"));
        }
        if reason.trim().is_empty() {
            report.push(format!("{command:?}: the allowlist entry needs a reason"));
        }
    }
    for command in &found {
        if !listed.contains(command.as_str()) {
            report.push(format!(
                "{command}: hidden without an allowlist entry; list (\"{command}\", \"<why hosts need it>\") or make it visible"
            ));
        }
    }
    for command in listed {
        if !found.iter().any(|found| found == command) {
            report.push(format!("remove {command:?}: it is not a hidden command"));
        }
    }
    report
}

#[cfg(test)]
mod hidden_tests {
    use super::*;

    fn grammar() -> Command {
        Command::new("tool")
            .subcommand(Command::new("ls"))
            .subcommand(Command::new("__complete").hide(true))
            .subcommand(
                Command::new("group")
                    .subcommand(Command::new("__sweep").hide(true))
                    .subcommand(Command::new("show")),
            )
    }

    const LISTED: &[(&str, &str)] = &[
        ("tool __complete", "shells call it for completion"),
        ("tool group __sweep", "the scheduler calls it"),
    ];

    #[test]
    fn a_grammar_whose_hidden_commands_are_all_listed_is_clean() {
        assert_eq!(
            hidden_report(&grammar(), &["tool"], LISTED),
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_unlisted_hidden_command_fails_at_any_depth() {
        let report = hidden_report(&grammar(), &["tool"], &LISTED[..1]);
        assert_eq!(report.len(), 1, "{report:?}");
        assert!(report[0].starts_with("tool group __sweep: hidden without an allowlist entry"));
        let retired = Command::new("tool").subcommand(Command::new("team").hide(true));
        let report = hidden_report(&retired, &["tool"], &[]);
        assert!(
            report[0].starts_with("tool team: hidden without"),
            "{report:?}"
        );
    }

    #[test]
    fn a_double_underscore_command_must_be_hidden() {
        let visible = Command::new("tool").subcommand(Command::new("__annotate"));
        let report = hidden_report(&visible, &["tool"], &[]);
        assert_eq!(report, ["tool __annotate: named __ but visible; hide it"]);
    }

    #[test]
    fn a_child_of_a_hidden_command_is_hidden_too() {
        let tree = Command::new("tool").subcommand(
            Command::new("__hook")
                .hide(true)
                .subcommand(Command::new("run")),
        );
        let report = hidden_report(&tree, &["tool"], &[("tool __hook", "hosts")]);
        assert_eq!(report.len(), 1, "{report:?}");
        assert!(report[0].starts_with("tool __hook run: hidden without"));
    }

    #[test]
    fn stale_duplicate_and_reasonless_entries_fail() {
        let mut list = LISTED.to_vec();
        list.push(("tool __gone", "no longer exists"));
        list.push(("tool __complete", "again"));
        let report = hidden_report(&grammar(), &["tool"], &list);
        assert!(report.contains(&"\"tool __complete\": listed twice".to_owned()));
        assert!(report.contains(&"remove \"tool __gone\": it is not a hidden command".to_owned()));
        let report = hidden_report(
            &grammar(),
            &["tool"],
            &[LISTED[0], ("tool group __sweep", " ")],
        );
        assert_eq!(
            report,
            ["\"tool group __sweep\": the allowlist entry needs a reason"]
        );
    }

    #[test]
    fn hidden_aliases_are_not_commands() {
        let tree = Command::new("tool").subcommand(Command::new("ls").alias("list"));
        assert!(hidden_report(&tree, &["tool"], &[]).is_empty());
    }
}

#[cfg(test)]
mod listing_tests {
    use super::*;

    #[test]
    fn listing_guard_reaches_nested_commands_and_checks_both_spellings() {
        let root = Command::new("test").subcommand(
            Command::new("extension")
                .subcommand(Command::new("list").alias("ls"))
                .subcommand(Command::new("nested").subcommand(Command::new("ls")))
                .subcommand(
                    Command::new("other").subcommand(Command::new("ls").visible_alias("list")),
                ),
        );
        assert_eq!(list_spelling_report(&root, &["test"]).len(), 3);
        let valid = Command::new("test")
            .subcommand(Command::new("nested").subcommand(Command::new("ls").alias("list")));
        assert!(list_spelling_report(&valid, &["test"]).is_empty());
    }
}

fn visit(command: &Command, path: Vec<String>, probe: &Probe, out: &mut Vec<Violation>) {
    let mut fail = |rule, detail: String| {
        out.push(Violation {
            path: path.clone(),
            rule,
            detail,
        })
    };
    if command
        .get_about()
        .is_none_or(|about| about.to_string().trim().is_empty())
    {
        fail(Rule::Summary, "no summary".into());
    }
    let with = |extra: &[&str], leading: &[&str]| -> Vec<String> {
        leading
            .iter()
            .map(|word| (*word).to_owned())
            .chain(path.iter().cloned())
            .chain(extra.iter().map(|word| (*word).to_owned()))
            .collect()
    };
    let forms = [
        ("-h", with(&["-h"], &[])),
        ("--help", with(&["--help"], &[])),
        ("help <command>", with(&[], &["help"])),
    ];
    let mut texts = Vec::new();
    for (form, arguments) in forms {
        match (probe.help)(&arguments) {
            Ok(text) => texts.push((form, strip(&text))),
            Err(error) => fail(Rule::HelpForms, format!("{form}: {error}")),
        }
    }
    if let Some((first_form, first)) = texts.first() {
        for (form, text) in &texts[1..] {
            if text != first {
                fail(Rule::HelpForms, format!("{form} differs from {first_form}"));
            }
        }
        if let Err(detail) = template(first) {
            fail(Rule::Template, detail);
        }
        match examples(first) {
            Err(detail) => fail(Rule::Examples, detail),
            Ok(shown) if !(1..=3).contains(&shown.len()) => {
                fail(Rule::Examples, format!("{} examples", shown.len()))
            }
            Ok(shown) => {
                let target = with(&[], probe.program);
                for example in shown {
                    match example.argv() {
                        Err(error) => fail(Rule::ExampleParse, error),
                        Ok(argv) if !argv.starts_with(&target) => fail(
                            Rule::ExampleTarget,
                            format!("{:?} does not start with {:?}", example.command, target),
                        ),
                        Ok(argv) => {
                            if let Err(error) = (probe.parse)(&argv[probe.program.len()..]) {
                                fail(
                                    Rule::ExampleParse,
                                    format!("{:?}: {error}", example.command),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    for child in command
        .get_subcommands()
        .filter(|child| !child.is_hide_set())
    {
        let mut child_path = path.clone();
        child_path.push(child.get_name().to_owned());
        visit(child, child_path, probe, out);
    }
}

fn strip(text: &str) -> String {
    anstream::adapter::strip_str(text).to_string()
}

/// The shared template: a summary line, a blank line, `Usage:`, then section
/// headings in order with `Examples` last.
fn template(help: &str) -> Result<(), String> {
    let lines: Vec<&str> = help.lines().collect();
    match lines.as_slice() {
        [summary, "", usage, ..]
            if !summary.trim().is_empty()
                && !summary.starts_with(' ')
                && usage.starts_with("Usage: ") => {}
        _ => return Err("does not start with summary, blank line, Usage".into()),
    }
    // clap's `{all-args}` renders Commands, then Arguments, then Options.
    let rank = |heading: &str| match heading {
        "Commands:" => 1,
        "Arguments:" => 2,
        "Options:" => 3,
        "Examples:" => 5,
        _ => 4,
    };
    let headings: Vec<&str> = lines[3..]
        .iter()
        .copied()
        .filter(|line| !line.starts_with(' ') && line.ends_with(':'))
        .collect();
    if headings.last() != Some(&"Examples:") {
        return Err("Examples is not the last section".into());
    }
    if let Some(index) = headings.iter().position(|heading| *heading == "Details:")
        && index + 2 != headings.len()
    {
        return Err("Details must come just before Examples".into());
    }
    if headings
        .windows(2)
        .any(|pair| rank(pair[0]) > rank(pair[1]))
    {
        return Err(format!("sections out of order: {}", headings.join(" ")));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
