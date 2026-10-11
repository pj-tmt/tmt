use std::ffi::OsString;

use clap::error::{ContextKind, ContextValue};
use clap::{Arg, Command};

use crate::invocation::OutputMode;

/// Recover presentation mode and a public help path after Clap rejects input. Clap stops
/// at unknown options, whereas the reference parser can retain later flags.
/// Arity and scope come from the same grammar; this never constructs requests,
/// repairs invalid input, dispatches commands or inspects option-value contents.
pub fn error_context(definition: &Command, argv: &[OsString]) -> (OutputMode, Vec<String>) {
    let (mode, _, path, _) = inspect(definition, argv);
    (mode, path)
}

/// Remove only grammar-owned hidden candidates before Clap formats the error.
/// The original offending token and public candidates remain intact. A removed
/// last candidate lets the caller use its existing public-scope help hint.
pub fn parser_error_context(
    definition: &Command,
    argv: &[OsString],
    error: &mut clap::Error,
) -> (OutputMode, Vec<String>, bool) {
    let (mode, _, path, command) = inspect(definition, argv);
    let mut all_hidden = false;
    if let Some(ContextValue::Strings(suggestions)) = error.get(ContextKind::SuggestedSubcommand) {
        let public: Vec<_> = suggestions
            .iter()
            .filter(|name| {
                command
                    .find_subcommand(name.as_str())
                    .is_none_or(|candidate| !candidate.is_hide_set())
            })
            .cloned()
            .collect();
        if public.len() != suggestions.len() {
            all_hidden = public.is_empty();
            if all_hidden {
                error.remove(ContextKind::SuggestedSubcommand);
            } else {
                error.insert(
                    ContextKind::SuggestedSubcommand,
                    ContextValue::Strings(public),
                );
            }
        }
    }
    (mode, path, all_hidden)
}

/// Help intent uses the same grammar-aware value boundaries as error mode.
/// It never treats an option value or data after `--` as a help request.
pub fn help_intent(definition: &Command, argv: &[OsString]) -> Option<(Vec<String>, OutputMode)> {
    let (mode, path, _, _) = inspect(definition, argv);
    path.map(|path| (path, mode))
}

fn inspect<'a>(
    definition: &'a Command,
    argv: &[OsString],
) -> (OutputMode, Option<Vec<String>>, Vec<String>, &'a Command) {
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
                matches!(*id, "help" | "json")
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
    let public_path = commands
        .iter()
        .skip(1)
        .take_while(|command| !command.is_hide_set())
        .map(|command| command.get_name().to_owned())
        .collect();
    (
        mode,
        if valid { help } else { None },
        public_path,
        commands.last().expect("root command is always present"),
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestion_visibility_uses_the_selected_scope_and_hidden_aliases() {
        let definition = Command::new("tmt").subcommand(
            Command::new("public")
                .alias("p")
                .subcommand(Command::new("secret").hide(true).alias("private")),
        );
        for typo in ["secre", "privat"] {
            let argv = [OsString::from("p"), typo.into()];
            let mut error = definition
                .clone()
                .try_get_matches_from(["tmt", "p", typo])
                .unwrap_err();
            assert!(error.get(ContextKind::SuggestedSubcommand).is_some());
            let (mode, path, all_hidden) = parser_error_context(&definition, &argv, &mut error);
            assert_eq!(mode, OutputMode::default());
            assert_eq!(path, ["public"]);
            assert!(all_hidden);
            assert!(error.get(ContextKind::SuggestedSubcommand).is_none());
            assert!(
                definition
                    .clone()
                    .try_get_matches_from(["tmt", "p", "private"])
                    .is_ok()
            );
        }
    }
}
