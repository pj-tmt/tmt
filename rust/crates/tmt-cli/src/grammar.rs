use clap::{Arg, ArgAction, Command};
use tmt_cli_style::CommandSpec;
use tmt_core::driver::descriptor::DriverDescriptor;

/// A command's help: its summary and one to three examples, which the
/// grammar walk parses (docs/cli-style.md#help).
macro_rules! spec {
    ($name:literal, $summary:literal, [$($note:literal => $command:literal),+ $(,)?]) => {
        &tmt_cli_style::CommandSpec {
            name: $name,
            summary: $summary,
            examples: &[$(tmt_cli_style::Example {
                command: $command,
                note: $note,
            }),+],
            outputs: tmt_cli_style::OutputModes::Human,
            details: "",
        }
    };
    ($name:literal, $summary:literal, details = $details:literal, [$($note:literal => $command:literal),+ $(,)?]) => {
        &tmt_cli_style::CommandSpec {
            details: $details,
            ..*spec!($name, $summary, [$($note => $command),+])
        }
    };
}

pub mod completion;
pub mod extensions;

mod identity;
mod installation;
mod launch;
mod presence;
mod requests;
mod rooms;
mod settings;

/// The root's help; root help adds the extensions discovered on `PATH`.
pub const ROOT: &CommandSpec = spec!(
    "tmt",
    "TMT native alpha: collaborate with terminal agents through durable exchanges",
    [
        "Set up agent skills" => "tmt install",
        "Ask an agent and wait for its reply" => "tmt talk worker \"Run the tests\"",
        "Update a managed installation" => "tmt upgrade",
    ]
);

// This tree owns recognition, help, completion and allowed-option validation.
// Root-recognized options are inherited for placement, not universal permission.
pub fn grammar() -> Command {
    grammar_for(&tmt_core::driver::ALL)
}

/// The grammar with `drivers` as the agent values of `install`, `setup` and
/// the internal hook, which completion reads.
pub fn grammar_for(drivers: &[&'static DriverDescriptor]) -> Command {
    let names: Vec<&'static str> = drivers.iter().map(|driver| driver.name).collect();
    let hooked: Vec<&'static str> = drivers
        .iter()
        .filter(|driver| driver.hooks.is_some())
        .map(|driver| driver.name)
        .collect();
    let mut root = base(ROOT);
    for id in [
        "json",
        "force",
        "config",
        "delay",
        "wait",
        "detach",
        "timeout",
        "lines",
        "no-preamble",
        "team",
        "help",
        "version",
    ] {
        root = root.arg(option(id).global(true));
    }
    root = root.subcommand(crate::office_facade::grammar::grammar().after_help(
        "To install, update or remove any official extension, see: tmt extension --help",
    ));
    root = root.subcommand(storage(spec!(
        "api",
        "Versioned JSON extension interface (one request on stdin)",
        [
            "Answer one JSON request read from stdin" => "tmt api",
        ]
    )));
    root = root.subcommand(rooms::room());
    root.subcommand(
        general(spec!(
            "help",
            "Show help for a command",
            [
                "Show help for a command" => "tmt help talk",
                "Show help for a subcommand" => "tmt help identity show",
            ]
        ))
        .arg(operand("command-path", false).num_args(0..)),
    )
    .subcommand(
        internal("team", "Retired command")
            .hide(true)
            .arg(operand("scope", false)),
    )
    .subcommand(settings::init())
    .subcommand(launch::run())
    .subcommand(launch::resume())
    .subcommand(presence::list_command())
    .subcommand(presence::add())
    .subcommand(presence::name())
    .subcommand(presence::marked())
    .subcommand(presence::remove())
    .subcommand(identity::rename())
    .subcommand(requests::talk())
    .subcommand(presence::check())
    .subcommand(presence::focus())
    .subcommand(presence::whoami())
    .subcommand(presence::unbind())
    .subcommand(installation::extension())
    .subcommand(settings::config())
    .subcommand(identity::preamble())
    .subcommand(requests::exchanges())
    .subcommand(identity::identity())
    .subcommand(identity::notes())
    .subcommand(identity::role())
    .subcommand(requests::reply_command())
    .subcommand(requests::result())
    .subcommand(requests::inbox())
    .subcommand(requests::answer())
    .subcommand(installation::install(names))
    .subcommand(installation::setup(hooked.clone()))
    .subcommand(installation::hook(hooked))
    .subcommand(launch::channel_server())
    .subcommand(requests::request_observer())
    .subcommand(
        general(spec!(
            "completion",
            "Generate shell completion",
            [
                "Print the zsh completion script" => "tmt completion zsh",
            ]
        ))
        .arg(operand("shell", false)),
    )
    .subcommand(
        internal("__complete", "Internal shell completion context")
            .hide(true)
            .arg(
                Arg::new("words")
                    .num_args(0..)
                    .trailing_var_arg(true)
                    .value_parser(clap::builder::OsStringValueParser::new()),
            ),
    )
    .subcommand(installation::upgrade())
    .subcommand(installation::uninstall())
    .subcommand(installation::refresh_skills())
    .subcommand(installation::upgrade_extensions())
    .subcommand(installation::native_install())
    .subcommand(installation::learn())
}

fn bare(name: &'static str) -> Command {
    Command::new(name)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .args_override_self(true)
}

fn base(spec: &CommandSpec) -> Command {
    tmt_cli_style::apply(bare(spec.name), spec, &[])
}

fn storage(spec: &CommandSpec) -> Command {
    base(spec).arg(option("json"))
}

fn general(spec: &CommandSpec) -> Command {
    with_options(storage(spec), &["wait", "team"])
}

/// Hidden internal entry points: no help page, so no examples.
fn internal(name: &'static str, about: &'static str) -> Command {
    with_options(
        bare(name).about(about).arg(option("json")),
        &["wait", "team"],
    )
}

fn with_options(mut command: Command, ids: &[&'static str]) -> Command {
    for id in ids {
        command = command.arg(option(id));
    }
    command
}

/// Consented extension installation: the extension name, --yes and --prefix.
fn extension_target(command: Command) -> Command {
    command
        .arg(operand("name", true))
        .arg(Arg::new("yes").long("yes").action(ArgAction::SetTrue))
        .arg(Arg::new("prefix").long("prefix"))
}

fn channel_option() -> Arg {
    Arg::new("channel")
        .long("channel")
        .value_parser(tmt_core::native_install::Channel::ALL.map(|channel| channel.as_str()))
}

/// A `tmt ls` filter; filters narrow the list, never a single identity.
fn list_filter(id: &'static str, help: &'static str) -> Arg {
    Arg::new(id)
        .long(id)
        .help(help)
        .action(ArgAction::SetTrue)
        .conflicts_with("target")
}

fn operand(id: &'static str, required: bool) -> Arg {
    Arg::new(id).required(required)
}

pub fn root_allowed(id: &str) -> bool {
    matches!(id, "json" | "help" | "version" | "team")
}

/// Completion generators include hidden and inherited arguments. Project only
/// advertised, command-owned arguments so rejection-only syntax is never offered.
pub fn public_grammar(definition: &Command, root: bool) -> Command {
    let mut result = tmt_cli_style::frame(Command::new(definition.get_name().to_owned()))
        .subcommand_required(definition.is_subcommand_required_set())
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .aliases(
            definition
                .get_all_aliases()
                .filter(|alias| {
                    !definition
                        .get_visible_aliases()
                        .any(|visible| visible == *alias)
                })
                .map(str::to_owned),
        )
        .visible_aliases(definition.get_visible_aliases().map(str::to_owned));
    if let Some(about) = definition.get_about() {
        result = result.about(about.clone());
    }
    if let Some(after_help) = definition.get_after_help() {
        result = result.after_help(after_help.clone());
    }
    for argument in definition.get_arguments() {
        if !argument.is_hide_set() && (!root || root_allowed(argument.get_id().as_str())) {
            result = result.arg(argument.clone().global(false));
        }
    }
    for child in definition
        .get_subcommands()
        .filter(|child| !child.is_hide_set())
    {
        result = result.subcommand(public_grammar(child, false));
    }
    for group in definition.get_groups() {
        if group.get_args().next().is_some()
            && group.get_args().all(|id| {
                result
                    .get_arguments()
                    .any(|argument| argument.get_id() == id)
            })
        {
            result = result.group(group.clone());
        }
    }
    result.arg(option("help").hide(false).global(false))
}

/// Resolve public command paths without dispatching any runtime operation.
pub fn help_command(path: &[String]) -> Result<Command, String> {
    let definition = grammar();
    let mut selected = &definition;
    let mut names = vec!["tmt".to_owned()];
    for name in path {
        selected = selected
            .find_subcommand(name)
            .filter(|command| !command.is_hide_set())
            .ok_or_else(|| format!("Unknown command '{name}' after {}.", names.join(" ")))?;
        names.push(selected.get_name().to_owned());
    }
    Ok(public_grammar(selected, path.is_empty()).bin_name(names.join(" ")))
}

fn option(id: &'static str) -> Arg {
    let flag = |description: &'static str| {
        Arg::new(id)
            .long(id)
            .help(description)
            .action(ArgAction::SetTrue)
    };
    let value = |description: &'static str| {
        Arg::new(id)
            .long(id)
            .help(description)
            .action(ArgAction::Set)
            .allow_hyphen_values(matches!(
                id,
                "config" | "delay" | "timeout" | "lines" | "team"
            ))
    };
    match id {
        "json" => flag("Output one JSON document"),
        "force" => {
            flag("Authorize the command's documented protected replacement or removal").short('f')
        }
        "save" => flag("Preserve this identity after its pane is gone").short('s'),
        "help" => flag("Show help").short('h').hide(true),
        "version" => tmt_cli_style::version_arg(ArgAction::SetTrue),
        "wait" => flag("Retired; use timeout or detach").hide(true),
        "detach" => flag("Return after sending"),
        "inbox" => flag("Queue only for recipient pull; use plain talk for live notification"),
        "incoming" => flag("Use recipient-facing request attention"),
        "no-preamble" => flag("Skip the recipient preamble"),
        "stdin" => flag("Read complete input through EOF"),
        "skill" => Arg::new(id)
            .long(id)
            .help("Print an exact bundled skill (default: tmux-team)")
            .num_args(0..=1)
            .default_missing_value(tmt_core::skill_catalog::MAIN)
            .value_parser(
                tmt_core::skill_catalog::BUNDLED
                    .iter()
                    .map(|skill| skill.name)
                    .collect::<Vec<_>>(),
            ),
        "global" => flag("Edit global settings").short('g'),
        "config" => value("Unsupported path override").hide(true),
        "team" => value("Retired scope").hide(true),
        "timeout" => value("Observer timeout in seconds or with ms/s/m suffix"),
        "delay" => value("Pre-send delay in seconds or with ms/s/m suffix"),
        "debounce" => value("Trailing quiet interval in seconds or with ms/s/m suffix"),
        "mood" => value("Optional self-reported mood, up to 32 UTF-8 bytes"),
        "for" => value("Status duration: seconds or ms/s/m; default 60m, range 1s to 1440m"),
        "lines" => value("Diagnostic capture line count"),
        "identity" => value("Select an explicit identity").global(true),
        "room" => value("Restrict to a room UUID or unique exact name"),
        "file" => value("Read content from a regular file"),
        "message" => value("Submit exact inline content"),
        "receipt" => value("Receipt supplied by talk"),
        "dir" => value("Custom skills directory"),
        "limit" => value("Maximum exchanges, 1 through 200").global(true),
        "after" => value("List revisions after this cursor").global(true),
        "revision" => value("Observed revision to acknowledge"),
        "from" => value("Only requests from this identity"),
        "request" => value("Answer this open request ID"),
        "where" => Arg::new(id)
            .long(id)
            .help("Require exact metadata KEY=VALUE")
            .action(ArgAction::Append),
        "has" => Arg::new(id)
            .long(id)
            .help("Require a metadata KEY")
            .action(ArgAction::Append),
        _ => unreachable!("unknown grammar option"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar_is_internally_consistent() {
        grammar().debug_assert();
    }
}
