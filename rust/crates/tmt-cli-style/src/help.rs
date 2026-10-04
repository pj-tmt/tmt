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
    /// Safety or boundary facts that must be visible in help, shown in a
    /// `Details` section just before `Examples`; empty for none. Rare by
    /// design: never a place for a longer description.
    pub details: &'static str,
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
    let mut command = apply(
        Command::new(spec.name)
            .disable_help_flag(true)
            .disable_version_flag(true)
            .disable_help_subcommand(true),
        spec,
        sections,
    )
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

/// The hidden `-V`/`--version` flag. [`command`] disables clap's own, so a
/// command opts in: [`ArgAction::Version`] prints `<name> <version>` and
/// exits 0; a parser that answers itself passes [`ArgAction::SetTrue`].
pub fn version_arg(action: ArgAction) -> Arg {
    Arg::new("version")
        .short('V')
        .long("version")
        .help("Show version")
        .action(action)
        .hide(true)
}

/// The spec's summary, template, palette and `Examples` on an existing
/// command, for a CLI that owns its own help and `--json` options (core's
/// parser resolves help itself). [`command`] is this plus those options.
///
/// # Panics
/// Like [`command`], when the spec has no summary or no examples.
pub fn apply(command: Command, spec: &CommandSpec, sections: &[HelpSection]) -> Command {
    assert!(!spec.summary.is_empty(), "{} has no summary", spec.name);
    assert!(
        (1..=3).contains(&spec.examples.len()),
        "{} needs one to three examples",
        spec.name
    );
    frame(command)
        .about(spec.summary)
        .after_help(after_help(sections, spec.details, spec.examples))
}

/// The shared template and palette, for a CLI that rebuilds commands from a
/// definition (such as a help projection) and carries `about`/`after_help` over.
pub fn frame(command: Command) -> Command {
    command.help_template(TEMPLATE).styles(help_styles())
}

/// What `-h`, `--help` and `help <command>` all print.
pub fn help_text(command: &Command, terminal: Terminal) -> String {
    rendered_help(&command.clone().render_help(), terminal)
}

/// Format clap's already-rendered help, including a `DisplayHelp` error,
/// through the same path as [`help_text`]. Prose wraps only at known terminal
/// widths of at least 40 cells; Usage, commands and Examples remain intact.
pub fn rendered_help(help: &clap::builder::StyledStr, terminal: Terminal) -> String {
    let text = if terminal.color {
        help.ansi().to_string()
    } else {
        help.to_string()
    };
    let Some(width) = terminal.width.filter(|width| *width >= 40) else {
        return text;
    };
    let plain = help.to_string();
    let program = plain.lines().find_map(|line| {
        line.trim_start()
            .strip_prefix("Usage: ")?
            .split_whitespace()
            .next()
    });
    let mut output = String::with_capacity(text.len());
    let mut in_examples = false;
    for line in text.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let visible = anstream::adapter::strip_str(body).to_string();
        let trimmed = visible.trim_start();
        in_examples |= trimmed == "Examples:";
        let is_command = program.is_some_and(|program| {
            trimmed == program
                || trimmed
                    .strip_prefix(program)
                    .is_some_and(|tail| tail.starts_with(char::is_whitespace))
        });
        if in_examples
            || trimmed.starts_with("Usage:")
            || is_command
            || visible.width() <= usize::from(width)
        {
            output.push_str(line);
            continue;
        }
        wrap_prose(body, usize::from(width), &mut output);
        if line.ends_with('\n') {
            output.push('\n');
        }
    }
    output
}

/// Retain each word and its ANSI bytes; replace only the whitespace at a wrap.
fn wrap_prose(line: &str, width: usize, output: &mut String) {
    let visible = anstream::adapter::strip_str(line).to_string();
    let indent = &visible[..visible.len() - visible.trim_start().len()];
    let mut column = 0;
    let mut has_word = false;
    let mut cursor = 0;
    while let Some(offset) = line[cursor..].find(|c: char| !c.is_whitespace()) {
        let start = cursor + offset;
        let end = start
            + line[start..]
                .find(char::is_whitespace)
                .unwrap_or(line.len() - start);
        let word = &line[start..end];
        let word_width = anstream::adapter::strip_str(word).to_string().width();
        let gap = &line[cursor..start];
        if word_width > 0 && has_word && column + gap.width() + word_width > width {
            output.push('\n');
            output.push_str(indent);
            column = indent.width();
        } else {
            output.push_str(gap);
            column += gap.width();
        }
        output.push_str(word);
        column += word_width;
        has_word |= word_width > 0;
        cursor = end;
    }
    output.push_str(&line[cursor..]);
}

/// What a `help [command...]` request resolves to.
#[derive(Debug, Clone)]
pub enum Route {
    /// Not a `help` request; the words belong to the CLI's own parser.
    Other,
    /// Print this command's help: the text `-h` prints for it.
    Help(Box<Command>),
    /// `help` named a word that is no visible command at that point.
    Unknown(String),
}

/// Resolves `help [command...]` for a CLI built with [`command`] that has no
/// `help` subcommand of its own, so one route serves every such CLI and
/// `<cli> help status` prints what `<cli> status -h` prints. `words` are the
/// arguments after the program name; `root` is the CLI's whole grammar.
///
/// Only `help` followed by command names is resolved. `<command> -h` and
/// `--help` stay with clap, so an operand that is data, such as a message
/// that reads `-h`, is never mistaken for a help request. Hidden commands do
/// not resolve.
pub fn route(root: &Command, words: &[String]) -> Route {
    let Some((first, path)) = words.split_first() else {
        return Route::Other;
    };
    if first != "help" {
        return Route::Other;
    }
    // Building propagates the program name and global options into every
    // subcommand, so its usage line names the whole path, as `-h` does.
    let mut root = root.clone();
    root.build();
    let mut command = &root;
    for word in path {
        let found = command.get_subcommands().find(|child| {
            !child.is_hide_set()
                && (child.get_name() == word || child.get_all_aliases().any(|alias| alias == word))
        });
        match found {
            Some(child) => command = child,
            None => return Route::Unknown(word.clone()),
        }
    }
    Route::Help(Box::new(command.clone()))
}

/// Styled with the help palette; clap strips the codes for plain output.
fn after_help(sections: &[HelpSection], details: &str, examples: &[Example]) -> String {
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
    if !details.is_empty() {
        text.push_str(&title("Details"));
        for line in details.lines() {
            let _ = writeln!(text, "  {line}");
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
