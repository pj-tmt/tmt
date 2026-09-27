use std::ffi::OsString;

use clap::{Arg, Command};

use crate::invocation::OutputMode;

/// Recover only presentation mode after Clap rejects an invocation. Clap stops
/// at unknown options, whereas the reference parser can retain later flags.
/// Arity and scope come from the same grammar; this never constructs requests,
/// repairs invalid input, dispatches commands or inspects option-value contents.
pub fn error_mode(definition: &Command, argv: &[OsString]) -> OutputMode {
    inspect(definition, argv).0
}

/// Help intent uses the same grammar-aware value boundaries as error mode.
/// It never treats an option value or data after `--` as a help request.
pub fn help_intent(definition: &Command, argv: &[OsString]) -> Option<(Vec<String>, OutputMode)> {
    let (mode, path) = inspect(definition, argv);
    path.map(|path| (path, mode))
}

fn inspect(definition: &Command, argv: &[OsString]) -> (OutputMode, Option<Vec<String>>) {
    let mut mode = OutputMode::default();
    let mut help = None;
    let mut valid = true;
    let mut options = Vec::new();
    let mut commands = vec![definition];
    let mut tokens = argv.iter().peekable();
    let mut can_descend = true;
    while let Some(token) = tokens.next() {
        // Once a trailing positional starts, the rest is opaque command data.
        // Derive this boundary from Clap rather than naming a special command.
        if commands.last().is_some_and(|command| {
            command
                .get_positionals()
                .any(|arg| arg.is_trailing_var_arg_set())
        }) && token.to_str().is_none_or(|value| !value.starts_with('-'))
        {
            break;
        }
        let Some(token) = token.to_str() else {
            continue;
        };
        if token == "--" {
            break;
        }
        if let Some(long) = token.strip_prefix("--") {
            let (name, inline) = long
                .split_once('=')
                .map_or((long, false), |(name, _)| (name, true));
            if let Some(option) = find_option(&commands, |arg| arg.get_long() == Some(name)) {
                options.push(option.get_id().as_str());
                if option.get_action().takes_values() {
                    if !inline {
                        let available = tokens.peek().is_some_and(|value| {
                            option.is_allow_hyphen_values_set()
                                || value
                                    .to_str()
                                    .is_none_or(|value| !value.starts_with('-') || value == "-")
                        });
                        if available {
                            tokens.next();
                        } else if option
                            .get_num_args()
                            .is_none_or(|range| range.min_values() > 0)
                        {
                            valid = false;
                        }
                    }
                } else if !inline {
                    record(&mut mode, option, &commands, valid, &mut help);
                } else {
                    valid = false;
                }
            } else {
                valid = false;
            }
        } else if let Some(short) = token.strip_prefix('-') {
            let mut chars = short.chars().peekable();
            while let Some(short) = chars.next() {
                if let Some(option) = find_option(&commands, |arg| arg.get_short() == Some(short)) {
                    options.push(option.get_id().as_str());
                    if option.get_action().takes_values() {
                        if chars.peek().is_none() {
                            tokens.next();
                        }
                        break;
                    }
                    record(&mut mode, option, &commands, valid, &mut help);
                } else {
                    valid = false;
                }
            }
        } else if can_descend {
            if let Some(child) = commands
                .last()
                .and_then(|command| command.find_subcommand(token))
            {
                commands.push(child);
            } else {
                // Later positional data must not select a command after an
                // unknown root operand or the start of a leaf's operands.
                can_descend = false;
                if commands
                    .last()
                    .is_some_and(|command| command.has_subcommands())
                {
                    valid = false;
                }
            }
        }
    }
    if let Some(path) = &help {
        let selected = path
            .iter()
            .try_fold(definition, |command, name| command.find_subcommand(name));
        valid &= selected.is_some_and(|command| {
            options.iter().all(|id| {
                *id == "help"
                    || if path.is_empty() {
                        crate::grammar::root_allowed(id)
                            && definition
                                .get_arguments()
                                .any(|argument| argument.get_id() == *id && !argument.is_hide_set())
                    } else {
                        command
                            .get_arguments()
                            .any(|argument| argument.get_id() == *id && !argument.is_hide_set())
                    }
            })
        });
    }
    (mode, if valid { help } else { None })
}

fn find_option<'a>(commands: &[&'a Command], predicate: impl Fn(&Arg) -> bool) -> Option<&'a Arg> {
    commands
        .iter()
        .rev()
        .enumerate()
        .find_map(|(index, command)| {
            command
                .get_arguments()
                .find(|arg| (index == 0 || arg.is_global_set()) && predicate(arg))
        })
}

fn record(
    mode: &mut OutputMode,
    option: &Arg,
    commands: &[&Command],
    valid: bool,
    help: &mut Option<Vec<String>>,
) {
    if option.get_id() == "json" {
        mode.json = true;
    }
    if option.get_id() == "help" && valid && help.is_none() {
        *help = Some(
            commands
                .iter()
                .skip(1)
                .map(|command| command.get_name().to_owned())
                .collect(),
        );
    }
}
