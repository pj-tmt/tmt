use std::ffi::OsString;

use clap::{ArgMatches, Command, parser::ValueSource};
use tmt_core::limits::{
    MAX_CAPTURE_LINES, MAX_JS_SAFE_INTEGER, is_valid_observer_timeout_seconds,
    is_valid_timer_delay_ms,
};

use crate::{grammar::grammar, invocation::*};

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "help_tests.rs"]
mod help_tests;

pub fn parse(argv: &[OsString]) -> Result<Parsed, ParseError> {
    if let Some(result) = crate::grammar::extensions::candidate(&grammar(), argv) {
        return result;
    }
    parse_core(argv)
}

pub fn parse_core(argv: &[OsString]) -> Result<Parsed, ParseError> {
    let definition = grammar();
    if let Some((path, mode)) = crate::diagnostics::help_intent(&definition, argv) {
        return help(path, mode);
    }
    let mut parser = definition.clone();
    let arguments = std::iter::once(OsString::from("tmt")).chain(argv.iter().cloned());
    let matches = match parser.try_get_matches_from_mut(arguments) {
        Ok(matches) => matches,
        Err(error) => {
            let mode = crate::diagnostics::error_mode(&definition, argv);
            return Err(ParseError {
                code: "USAGE_ERROR",
                message: error.to_string(),
                mode,
            });
        }
    };
    let mode = mode(&matches);
    let fail = |message: String| ParseError {
        code: "USAGE_ERROR",
        message,
        mode,
    };
    let mut leaf = &matches;
    let mut command = &definition;
    let mut path = Vec::new();
    let mut chain = vec![&matches];
    while let Some((name, submatches)) = leaf.subcommand() {
        path.push(name);
        command = command
            .find_subcommand(name)
            .expect("parsed command belongs to grammar");
        leaf = submatches;
        chain.push(leaf);
    }
    if supplied(leaf, "wait") {
        return Err(fail("The --wait option is retired. talk waits for a durable reply by default; use --timeout or --detach.".into()));
    }
    validate_options(command, &chain, &path).map_err(fail)?;
    if path.first() == Some(&"team") || text(leaf, "team").is_some() {
        return Err(ParseError {
            code: "UNSUPPORTED_TEAM",
            message: "Team workflows are not supported.".into(),
            mode,
        });
    }
    let invocation = translate(&path, leaf).map_err(fail)?;
    finish_parse(invocation, mode)
}

fn finish_parse(invocation: Invocation, mode: OutputMode) -> Result<Parsed, ParseError> {
    if mode.json
        && matches!(
            invocation,
            Invocation::Help(_)
                | Invocation::Version
                | Invocation::Completion(_)
                | Invocation::Complete(_)
                | Invocation::Learn { .. }
                | Invocation::Run { .. }
                | Invocation::Resume { .. }
        )
    {
        return Err(ParseError {
            code: "JSON_UNSUPPORTED",
            message: "This command does not support --json.".into(),
            mode,
        });
    }
    Ok(Parsed { invocation, mode })
}

fn validate_options(command: &Command, chain: &[&ArgMatches], path: &[&str]) -> Result<(), String> {
    for matches in chain {
        for id in matches.ids() {
            if !supplied(matches, id.as_str()) {
                continue;
            }
            let allowed = if path.is_empty() {
                crate::grammar::root_allowed(id.as_str())
            } else {
                command.get_arguments().any(|arg| arg.get_id() == id)
                    || command.get_groups().any(|group| group.get_id() == id)
            };
            // Parent subcommand IDs are not options.
            if !allowed && matches.subcommand_name() != Some(id.as_str()) {
                let selected = path
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect::<Vec<_>>();
                let message = format!(
                    "Unknown option or argument '{id}' for {}.",
                    command.get_name()
                );
                if let Ok(mut public) = crate::grammar::help_command(&selected) {
                    let example = if path.is_empty() {
                        "tmt help".to_owned()
                    } else {
                        format!("tmt help {}", path.join(" "))
                    };
                    return Err(format!(
                        "{message}\n{}\nTry `{example}` for supported options.",
                        public.render_usage()
                    ));
                }
                return Err(message);
            }
        }
    }
    Ok(())
}

fn help(path: Vec<String>, mode: OutputMode) -> Result<Parsed, ParseError> {
    crate::grammar::help_command(&path).map_err(|message| ParseError {
        code: "USAGE_ERROR",
        message,
        mode,
    })?;
    finish_parse(Invocation::Help(path), mode)
}

fn mode(matches: &ArgMatches) -> OutputMode {
    let mut leaf = matches;
    while let Some((_, child)) = leaf.subcommand() {
        leaf = child;
    }
    OutputMode {
        json: flag(leaf, "json"),
    }
}

fn supplied(matches: &ArgMatches, id: &str) -> bool {
    matches.try_contains_id(id).unwrap_or(false)
        && matches.value_source(id) == Some(ValueSource::CommandLine)
}

fn flag(matches: &ArgMatches, id: &str) -> bool {
    matches
        .try_get_one::<bool>(id)
        .ok()
        .flatten()
        .copied()
        .unwrap_or(false)
}

fn text(matches: &ArgMatches, id: &str) -> Option<String> {
    matches.try_get_one::<String>(id).ok().flatten().cloned()
}

fn required(matches: &ArgMatches, id: &str) -> String {
    text(matches, id).expect("required grammar operand was validated")
}

fn texts(matches: &ArgMatches, id: &str) -> Vec<String> {
    matches
        .try_get_many::<String>(id)
        .ok()
        .flatten()
        .map(|values| values.cloned().collect())
        .unwrap_or_default()
}

fn translate(path: &[&str], m: &ArgMatches) -> Result<Invocation, String> {
    Ok(match path {
        ["api"] => Invocation::Api,
        [] if flag(m, "version") => Invocation::Version,
        [] | ["team"] => Invocation::Help(Vec::new()),
        ["help"] => {
            let path = texts(m, "command-path");
            crate::grammar::help_command(&path)?;
            Invocation::Help(path)
        }
        ["setup"] => Invocation::Setup {
            provider: text(m, "provider"),
            remove: flag(m, "remove"),
            yes: flag(m, "yes"),
        },
        ["__hook"] => Invocation::ProviderHook {
            provider: text(m, "provider").expect("required provider"),
            worker: flag(m, "worker"),
        },
        ["__request-observer"] => Invocation::RequestObserver {
            request_id: text(m, "request-id").expect("required request ID"),
        },
        ["completion"] => Invocation::Completion(text(m, "shell")),
        ["__complete"] => Invocation::Complete(
            m.get_many::<OsString>("words")
                .map(|values| values.cloned().collect())
                .unwrap_or_default(),
        ),
        ["learn"] => Invocation::Learn {
            skill: text(m, "skill"),
        },
        ["init"] => Invocation::Init,
        ["run"] => {
            let values = m
                .get_many::<OsString>("run-argv")
                .expect("required run operands")
                .cloned()
                .collect::<Vec<_>>();
            let name = values[0]
                .to_str()
                .ok_or("Identity name must be valid UTF-8.")?
                .to_owned();
            let resume = flag(m, "resume");
            let command = values[1..].to_vec();
            if command
                .first()
                .is_some_and(|word| word.as_encoded_bytes().starts_with(b"-"))
            {
                return Err("The command must not start with '-'. TMT options go before the name: tmt run -s Alice <command>.".into());
            }
            if resume && !command.is_empty() {
                return Err("Use `tmt run --resume <name>` to resume, or pass the command's own flags after the name; do not combine both forms.".into());
            }
            Invocation::Run {
                name,
                command,
                resume,
                save: flag(m, "save"),
            }
        }
        ["resume"] => Invocation::Resume {
            name: text(m, "name").expect("required resume name"),
            forget: flag(m, "forget"),
            retry: flag(m, "retry"),
        },
        ["whoami"] if flag(m, "context") => Invocation::WhoamiContext,
        ["whoami"] => Invocation::Whoami,
        ["unbind"] => Invocation::Unbind,
        ["upgrade"] => Invocation::Upgrade {
            channel: text(m, "channel")
                .and_then(|value| tmt_core::native_install::Channel::parse(&value)),
            exact: text(m, "to"),
            unpin: flag(m, "unpin"),
        },
        ["office", ..] => Invocation::Office {
            prefix: text(m, "prefix"),
            operation: crate::office_facade::parser::translate(path, m)?,
        },
        ["__native-refresh-skills"] => Invocation::NativeRefreshSkills,
        ["__native-install"] => Invocation::NativeInstall {
            product: tmt_core::native_install::Product::parse(&required(m, "product"))
                .expect("product was validated by grammar"),
            archive: required(m, "archive"),
            manifest: required(m, "manifest"),
            prefix: required(m, "prefix"),
            channel: tmt_core::native_install::Channel::parse(&required(m, "channel"))
                .expect("channel was validated by grammar"),
            pin: if flag(m, "pin") {
                tmt_core::native_install::PinAction::PinCandidate
            } else if flag(m, "unpin") {
                tmt_core::native_install::PinAction::Clear
            } else {
                tmt_core::native_install::PinAction::Preserve
            },
        },
        ["list"] => Invocation::List {
            target: text(m, "target"),
            room: text(m, "room"),
        },
        ["room", "create"] => Invocation::Room(RoomOperation::Create(required(m, "name"))),
        ["room", "list"] => Invocation::Room(RoomOperation::List),
        ["room", "show"] => Invocation::Room(RoomOperation::Show(required(m, "room"))),
        ["room", "retire"] => Invocation::Room(RoomOperation::Retire(required(m, "room"))),
        ["room", action @ ("send" | "broadcast")] => Invocation::Room(RoomOperation::Dispatch {
            room: required(m, "room"),
            message: required(m, "message"),
            identity: text(m, "identity"),
            operation_id: text(m, "operation-id"),
            kind: if *action == "send" {
                tmt_core::request::RequestKind::Request
            } else {
                tmt_core::request::RequestKind::Announcement
            },
        }),
        ["room", action @ ("join" | "leave")] => Invocation::Room(RoomOperation::Membership {
            room: required(m, "room"),
            identity: text(m, "identity"),
            change: if *action == "join" {
                tmt_core::room::MembershipChange::Join
            } else {
                tmt_core::room::MembershipChange::Leave
            },
        }),
        ["name"] | ["add"] => Invocation::Bind {
            pane: text(m, "pane-target"),
            name: required(m, "name"),
            save: flag(m, "save"),
        },
        ["marked"] => Invocation::BindMarked {
            name: required(m, "name"),
            save: flag(m, "save"),
        },
        ["rm"] => Invocation::Remove {
            name: required(m, "name"),
            force: flag(m, "force"),
        },
        ["talk"] => {
            let timeout = text(m, "timeout")
                .map(|value| duration(&value))
                .transpose()?;
            let delay = text(m, "delay").map(|value| duration(&value)).transpose()?;
            if flag(m, "detach") && timeout.is_some() {
                return Err("Use either --timeout or --detach, not both.".into());
            }
            if timeout.is_some_and(|value| !is_valid_observer_timeout_seconds(value)) {
                return Err(
                    "Talk timeout must be finite, positive, and no greater than 24 hours.".into(),
                );
            }
            if delay.is_some_and(|value| !is_valid_timer_delay_ms(value * 1000.0)) {
                return Err("Talk delay exceeds the supported timer limit.".into());
            }
            Invocation::Talk {
                target: required(m, "target"),
                message: required(m, "message"),
                originator: text(m, "identity"),
                options: TalkOptions {
                    room: text(m, "room"),
                    inbox: flag(m, "inbox"),
                    force: flag(m, "force"),
                    detach: flag(m, "detach"),
                    delay_seconds: delay,
                    timeout_seconds: timeout,
                    no_preamble: flag(m, "no-preamble"),
                },
            }
        }
        ["focus"] if flag(m, "client") => Invocation::FocusClient,
        ["focus"] => Invocation::Focus {
            target: required(m, "target"),
        },
        ["check"] => {
            let positional = text(m, "capture-lines")
                .map(|value| integer(&value, "lines", 0, MAX_CAPTURE_LINES))
                .transpose()?;
            let flagged = text(m, "lines")
                .map(|value| integer(&value, "lines", 0, MAX_CAPTURE_LINES))
                .transpose()?;
            Invocation::Check {
                target: required(m, "target"),
                lines: positional.or(flagged),
            }
        }
        ["extension", "hooks", "enable"] => {
            Invocation::ExtensionHooks(ExtensionHooksRequest::Enable(required(m, "name")))
        }
        ["extension", "hooks", "disable"] => {
            Invocation::ExtensionHooks(ExtensionHooksRequest::Disable(required(m, "name")))
        }
        ["extension", "hooks", "list"] => Invocation::ExtensionHooks(ExtensionHooksRequest::List),
        ["config"] | ["config", "show"] => Invocation::Config(ConfigRequest::Show),
        ["config", "set"] => Invocation::Config(ConfigRequest::Set {
            key: required(m, "key"),
            value: required(m, "value"),
            global: flag(m, "global"),
        }),
        ["config", "clear"] => Invocation::Config(ConfigRequest::Clear {
            key: text(m, "key"),
        }),
        ["identity", "create"] => {
            Invocation::Identity(IdentityRequest::Create(required(m, "name")))
        }
        ["identity", "show"] => Invocation::Identity(IdentityRequest::Show(text(m, "name"))),
        ["identity", "list"] => {
            let mut filters = Vec::new();
            for expression in texts(m, "where") {
                let Some((key, value)) = expression.split_once('=') else {
                    return Err("--where requires KEY=VALUE.".into());
                };
                filters.push(IdentityFilterRequest::Equals {
                    key: key.into(),
                    value: value.into(),
                });
            }
            filters.extend(texts(m, "has").into_iter().map(IdentityFilterRequest::Has));
            Invocation::Identity(IdentityRequest::List(filters))
        }
        ["identity", "status", operation] => Invocation::Identity(IdentityRequest::Status {
            identity: text(m, "identity"),
            operation: match *operation {
                "show" => IdentityStatusRequest::Show,
                "clear" => IdentityStatusRequest::Clear,
                "set" => {
                    use tmt_core::identity_status::{
                        DEFAULT_STATUS_TTL_MS, MAX_STATUS_TTL_MS, MIN_STATUS_TTL_MS,
                    };
                    let millis = text(m, "for")
                        .map(|value| duration(&value).map(|seconds| seconds * 1000.0))
                        .transpose()?
                        .unwrap_or(DEFAULT_STATUS_TTL_MS as f64);
                    if !millis.is_finite()
                        || !(MIN_STATUS_TTL_MS as f64..=MAX_STATUS_TTL_MS as f64).contains(&millis)
                    {
                        return Err("Status duration must be 1 second through 24 hours.".into());
                    }
                    IdentityStatusRequest::Set {
                        activity: required(m, "activity"),
                        mood: text(m, "mood"),
                        ttl_ms: millis.round() as u64,
                    }
                }
                _ => unreachable!(),
            },
        }),
        ["identity", "meta", operation] => Invocation::Identity(IdentityRequest::Metadata {
            identity: text(m, "identity"),
            operation: match *operation {
                "set" => IdentityMetadataRequest::Set {
                    key: required(m, "key"),
                    value: required(m, "value"),
                },
                "get" => IdentityMetadataRequest::Get {
                    key: required(m, "key"),
                },
                "list" => IdentityMetadataRequest::List,
                "rm" => IdentityMetadataRequest::Remove {
                    key: required(m, "key"),
                },
                _ => unreachable!(),
            },
        }),
        ["notes", "path"] => Invocation::NotesPath {
            identity: text(m, "identity"),
        },
        ["preamble"] | ["preamble", "show"] => {
            Invocation::Preamble(PreambleRequest::Show(text(m, "agent")))
        }
        ["preamble", "clear"] => Invocation::Preamble(PreambleRequest::Clear(required(m, "agent"))),
        ["preamble", "set"] => Invocation::Preamble(PreambleRequest::Set {
            name: required(m, "agent"),
            content: m
                .get_many::<String>("content")
                .expect("required content")
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
        }),
        ["role", operation] => Invocation::Role {
            identity: text(m, "identity"),
            operation: match *operation {
                "show" => RoleOperation::Show,
                "clear" => RoleOperation::Clear,
                "set" => RoleOperation::Set(content(m, "content", false)?),
                _ => unreachable!(),
            },
        },
        ["x"] | ["x", "list"] => Invocation::Exchange {
            identity: text(m, "identity"),
            operation: ExchangeOperation::List {
                limit: text(m, "limit")
                    .map(|value| {
                        integer(
                            &value,
                            "--limit",
                            1,
                            tmt_core::request::attention::MAX_LIST_LIMIT,
                        )
                    })
                    .transpose()?,
                after: text(m, "after")
                    .map(|value| integer(&value, "--after", 0, MAX_JS_SAFE_INTEGER))
                    .transpose()?,
            },
        },
        ["x", "show"] => Invocation::Exchange {
            identity: text(m, "identity"),
            operation: ExchangeOperation::Show {
                request_id: required(m, "request-id"),
                incoming: flag(m, "incoming"),
            },
        },
        ["x", "ackall"] => Invocation::Exchange {
            identity: text(m, "identity"),
            operation: ExchangeOperation::Ackall {
                incoming: flag(m, "incoming"),
            },
        },
        ["x", "ack"] => Invocation::Exchange {
            identity: text(m, "identity"),
            operation: ExchangeOperation::Ack {
                request_id: required(m, "request-id"),
                revision: integer(
                    &required(m, "revision"),
                    "--revision",
                    1,
                    MAX_JS_SAFE_INTEGER,
                )?,
                incoming: flag(m, "incoming"),
            },
        },
        ["x", "listen"] => {
            let timeout_seconds = text(m, "timeout")
                .map(|v| duration(&v))
                .transpose()?
                .unwrap_or(900.0);
            let debounce_seconds = text(m, "debounce")
                .map(|v| duration(&v))
                .transpose()?
                .unwrap_or(10.0);
            if !is_valid_observer_timeout_seconds(timeout_seconds)
                || !is_valid_observer_timeout_seconds(debounce_seconds)
            {
                return Err("Listen timeout and debounce must be finite, positive, and no greater than 24 hours.".into());
            }
            Invocation::Exchange {
                identity: text(m, "identity"),
                operation: ExchangeOperation::Listen {
                    room: text(m, "room"),
                    timeout_seconds,
                    debounce_seconds,
                },
            }
        }
        ["reply"] => Invocation::Reply {
            request_id: request_id(m)?,
            receipt: required(m, "receipt"),
            input: content(m, "message", true)?,
        },
        ["result"] => Invocation::Result {
            request_id: request_id(m)?,
        },
        ["install"] => {
            let target = text(m, "agent");
            let directory = text(m, "dir");
            if directory
                .as_ref()
                .is_some_and(|value| value.trim().is_empty())
            {
                return Err("Install directory must not be empty.".into());
            }
            if target.is_some() && directory.is_some() {
                return Err("The --dir option cannot be combined with an agent or all.".into());
            }
            Invocation::Install {
                target,
                directory,
                force: flag(m, "force"),
            }
        }
        _ => unreachable!("grammar and typed translation must agree"),
    })
}

fn content(matches: &ArgMatches, inline: &str, stdin: bool) -> Result<ContentInput, String> {
    let mut sources = Vec::new();
    if let Some(value) = text(matches, inline) {
        sources.push(ContentInput::Inline(value));
    }
    if let Some(value) = text(matches, "file") {
        sources.push(ContentInput::File(value));
    }
    if stdin && flag(matches, "stdin") {
        sources.push(ContentInput::Stdin);
    }
    if sources.len() != 1 {
        return Err(
            "Usage: select exactly one inline content, --file, or supported --stdin source.".into(),
        );
    }
    Ok(sources.remove(0))
}

fn request_id(matches: &ArgMatches) -> Result<String, String> {
    let value = required(matches, "request-id");
    if value.is_empty() || value.len() > 256 {
        return Err("Request ID must contain 1 through 256 UTF-8 bytes.".into());
    }
    Ok(value)
}

fn integer(value: &str, name: &str, minimum: u64, maximum: u64) -> Result<u64, String> {
    if !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && let Ok(number) = value.parse::<u64>()
        && (minimum..=maximum).contains(&number)
    {
        return Ok(number);
    }
    Err(format!(
        "{name} must be a decimal integer between {minimum} and {maximum}."
    ))
}

fn duration(value: &str) -> Result<f64, String> {
    let lower = value.to_ascii_lowercase();
    let (digits, divisor) = if let Some(digits) = lower.strip_suffix("ms") {
        (digits, 1000.0)
    } else if let Some(digits) = lower.strip_suffix('m') {
        (digits, 1.0 / 60.0)
    } else {
        (lower.strip_suffix('s').unwrap_or(&lower), 1.0)
    };
    let parts = digits.split('.').collect::<Vec<_>>();
    if parts.len() <= 2
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && let Ok(number) = digits.parse::<f64>()
        && number.is_finite()
    {
        return Ok(number / divisor);
    }
    Err(format!(
        "Invalid time format: {value}. Use number (seconds) or number with ms/s/m suffix."
    ))
}
