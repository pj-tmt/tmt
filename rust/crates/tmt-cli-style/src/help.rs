//! The registration contract. Every TMT command, core or extension, is built
//! by [`command`] from a [`CommandSpec`], so help has one template, one
//! palette and one `-h`/`--help` behavior, and no command exists without
//! examples.

use crate::palette::{Terminal, Token, help_styles};
use clap::{Arg, ArgAction, Command};
use std::fmt::Write as _;
use unicode_width::UnicodeWidthStr;

/// Summary, `Usage`, `Arguments`/`Options`/`Commands`, then extra sections
/// and `Examples` (clap's after-help).
const TEMPLATE: &str = "\
{about-with-newline}
{usage-heading} {usage}

{all-args}{after-help}";

/// One runnable line and what it does. `command` is the full argv as a user
/// types it, starting with the program name; quote operands with spaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Example {
    pub command: &'static str,
    pub note: &'static str,
}

/// Which output a command offers; `HumanAndJson` adds the shared `--json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputModes {
    Human,
    Json,
    HumanAndJson,
}

#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    pub name: &'static str,
    pub summary: &'static str,
    /// The common use first; one to three.
    pub examples: &'static [Example],
    pub outputs: OutputModes,
}

/// A titled list rendered in help before `Examples`, such as the extensions
/// discovered on `PATH`.
#[derive(Debug, Clone)]
pub struct HelpSection {
    pub title: String,
    pub entries: Vec<(String, String)>,
}

/// # Panics
/// When the spec has no summary or no examples: registration is refused, and
/// the grammar walk reaches every command, so tests fail rather than users.
pub fn command(spec: &CommandSpec) -> Command {
    command_with_sections(spec, &[])
}

/// [`command`] with sections the caller discovers at run time.
pub fn command_with_sections(spec: &CommandSpec, sections: &[HelpSection]) -> Command {
    assert!(!spec.summary.is_empty(), "{} has no summary", spec.name);
    assert!(
        (1..=3).contains(&spec.examples.len()),
        "{} needs one to three examples",
        spec.name
    );
    let mut command = Command::new(spec.name)
        .about(spec.summary)
        .help_template(TEMPLATE)
        .styles(help_styles())
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .after_help(after_help(sections, spec.examples))
        .arg(
            Arg::new("help")
                .short('h')
                .long("help")
                .help("Print help")
                .action(ArgAction::HelpShort),
        );
    if spec.outputs == OutputModes::HumanAndJson {
        command = command.arg(
            Arg::new("json")
                .long("json")
                .help("Output one JSON document")
                .action(ArgAction::SetTrue),
        );
    }
    command
}

/// What `-h`, `--help` and `help <command>` all print.
pub fn help_text(command: &Command, terminal: Terminal) -> String {
    let help = command.clone().render_help();
    if terminal.color {
        help.ansi().to_string()
    } else {
        help.to_string()
    }
}

/// Styled with the help palette; clap strips the codes for plain output.
fn after_help(sections: &[HelpSection], examples: &[Example]) -> String {
    let title = |text: &str| {
        let style = Token::Title.style();
        format!("{style}{text}:{style:#}\n")
    };
    let mut text = String::new();
    for section in sections {
        text.push_str(&title(&section.title));
        let width = section
            .entries
            .iter()
            .map(|(name, _)| name.width())
            .max()
            .unwrap_or(0);
        for (name, summary) in &section.entries {
            let literal = Token::Literal.style();
            let pad = " ".repeat(width - name.width());
            let _ = writeln!(text, "  {literal}{name}{literal:#}{pad}  {summary}");
        }
        text.push('\n');
    }
    text.push_str(&title("Examples"));
    let dim = Token::Dim.style();
    for (index, example) in examples.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        let _ = writeln!(text, "  {dim}# {}{dim:#}", example.note);
        let _ = write!(text, "  {}", example.command);
        if index + 1 < examples.len() {
            text.push('\n');
        }
    }
    text
}

impl Example {
    /// The argv a POSIX shell would pass, for parsing examples through the
    /// real grammar. Unbalanced quoting is an error.
    pub fn argv(&self) -> Result<Vec<String>, String> {
        split(self.command)
    }
}

fn split(command: &str) -> Result<Vec<String>, String> {
    shlex::split(command).ok_or_else(|| format!("unbalanced quoting: {command}"))
}

/// An example as a reader of the help sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownExample {
    pub note: String,
    pub command: String,
}

impl ShownExample {
    pub fn argv(&self) -> Result<Vec<String>, String> {
        split(&self.command)
    }
}

/// Reads the `Examples` section back from plain help text: the inverse of
/// what [`command`] writes, so the grammar walk checks the examples a user
/// actually sees. The section must be last and hold only note/command pairs.
pub fn examples(help: &str) -> Result<Vec<ShownExample>, String> {
    let lines: Vec<&str> = help.lines().collect();
    let start = lines
        .iter()
        .rposition(|line| *line == "Examples:")
        .ok_or("no Examples section")?;
    let mut shown = Vec::new();
    let mut rest = lines[start + 1..].iter().filter(|line| !line.is_empty());
    while let Some(line) = rest.next() {
        let note = line
            .strip_prefix("  # ")
            .ok_or_else(|| format!("expected an example note, found {line:?}"))?;
        let command = rest
            .next()
            .and_then(|line| line.strip_prefix("  "))
            .filter(|command| !command.is_empty() && !command.starts_with('#'))
            .ok_or_else(|| format!("example {note:?} has no command"))?;
        shown.push(ShownExample {
            note: note.to_owned(),
            command: command.to_owned(),
        });
    }
    Ok(shown)
}

#[cfg(test)]
mod tests;
