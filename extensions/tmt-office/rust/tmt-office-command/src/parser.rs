//! Typed Office subtree translation shared by both entry points.

use crate::invocation::*;
use clap::{ArgMatches, Command, parser::ValueSource};
use std::ffi::OsString;

#[derive(Debug, PartialEq)]
pub struct ParsedOffice {
    pub prefix: Option<String>,
    pub operation: OfficeOperation,
    pub mode: OutputMode,
}

#[derive(Debug, PartialEq)]
pub struct ParseOfficeError {
    pub code: &'static str,
    pub message: String,
    pub mode: OutputMode,
}

/// Direct invocation and the reserved core subtree share this grammar and
/// typed translation. The core root still owns its own non-Office commands.
pub fn parse_public(argv: &[OsString]) -> Result<ParsedOffice, ParseOfficeError> {
    let mut definition = crate::grammar::grammar();
    let arguments = std::iter::once(OsString::from("tmt-office")).chain(argv.iter().cloned());
    let matches = definition
        .try_get_matches_from_mut(arguments)
        .map_err(|error| ParseOfficeError {
            code: "USAGE_ERROR",
            message: error.to_string(),
            mode: OutputMode {
                json: argv.iter().any(|word| word == "--json"),
            },
        })?;
    let mut leaf = &matches;
    let mut command = &definition;
    let mut chain = vec![leaf];
    let mut path = vec!["office"];
    while let Some((name, next)) = leaf.subcommand() {
        path.push(name);
        command = command
            .find_subcommand(name)
            .expect("parsed Office command belongs to grammar");
        leaf = next;
        chain.push(leaf);
    }
    let mode = OutputMode {
        json: chain.iter().any(|matches| flag(matches, "json")),
    };
    if supplied(leaf, "wait") {
        return Err(ParseOfficeError { code: "USAGE_ERROR", message: "The --wait option is retired. talk waits for a durable reply by default; use --timeout or --detach.".into(), mode });
    }
    validate_options(command, &chain).map_err(|message| ParseOfficeError {
        code: "USAGE_ERROR",
        message,
        mode,
    })?;
    if text(leaf, "team").is_some() {
        return Err(ParseOfficeError {
            code: "UNSUPPORTED_TEAM",
            message: "Team workflows are not supported.".into(),
            mode,
        });
    }
    let operation = translate(&path, leaf).map_err(|message| ParseOfficeError {
        code: "USAGE_ERROR",
        message,
        mode,
    })?;
    Ok(ParsedOffice {
        prefix: text(leaf, "prefix"),
        operation,
        mode,
    })
}

fn supplied(matches: &ArgMatches, id: &str) -> bool {
    matches.try_contains_id(id).unwrap_or(false)
        && matches.value_source(id) == Some(ValueSource::CommandLine)
}

fn validate_options(command: &Command, chain: &[&ArgMatches]) -> Result<(), String> {
    for matches in chain {
        for id in matches.ids() {
            if !supplied(matches, id.as_str()) {
                continue;
            }
            let allowed = command.get_arguments().any(|arg| arg.get_id() == id)
                || command.get_groups().any(|group| group.get_id() == id);
            if !allowed && matches.subcommand_name() != Some(id.as_str()) {
                return Err(format!(
                    "Unknown option or argument '{id}' for {}.",
                    command.get_name()
                ));
            }
        }
    }
    Ok(())
}

pub fn translate(path: &[&str], m: &ArgMatches) -> Result<OfficeOperation, String> {
    Ok(match path.last().copied() {
        Some("validate") if path.get(1) == Some(&"extension") => {
            OfficeOperation::ExtensionValidate {
                file: required(m, "file"),
                instance: required(m, "instance"),
            }
        }
        Some("validate") if path.get(1) == Some(&"avatar") => {
            OfficeOperation::Avatar(OfficeAvatarOperation::Validate {
                file: required(m, "file"),
            })
        }
        Some("preview") if path.get(1) == Some(&"avatar") => {
            OfficeOperation::Avatar(OfficeAvatarOperation::Preview {
                file: required(m, "file"),
            })
        }
        Some("install") if path.get(1) == Some(&"avatar") => {
            OfficeOperation::Avatar(OfficeAvatarOperation::Install {
                file: required(m, "file"),
                if_revision: *m
                    .get_one::<u64>("if-revision")
                    .expect("grammar supplies avatar revision"),
            })
        }
        Some("remove") if path.get(1) == Some(&"avatar") => {
            OfficeOperation::Avatar(OfficeAvatarOperation::Remove {
                digest: required(m, "avatar-digest"),
                if_revision: *m
                    .get_one::<u64>("if-revision")
                    .expect("grammar supplies avatar revision"),
            })
        }
        Some("list") if path.get(1) == Some(&"avatar") => {
            OfficeOperation::Avatar(OfficeAvatarOperation::List {
                limit: *m
                    .get_one::<u64>("avatar-limit")
                    .expect("grammar supplies avatar list limit"),
                cursor: text(m, "cursor"),
            })
        }
        Some("show") if path.get(1) == Some(&"avatar") => {
            OfficeOperation::Avatar(OfficeAvatarOperation::Show {
                digest: required(m, "avatar-digest"),
            })
        }
        Some("validate") if path.get(1) == Some(&"prop") => {
            OfficeOperation::Prop(OfficePropOperation::Validate {
                file: required(m, "file"),
            })
        }
        Some("show" | "export")
            if path.get(1) == Some(&"whiteboard") && path.get(2) == Some(&"snapshot") =>
        {
            let reference = required(m, "snapshot-reference");
            if tmt_office_model::office_whiteboard::snapshot::resolve_snapshot_reference(&reference)
                .is_none()
            {
                return Err("Expected a canonical snapshot UUID or tmt:whiteboard:snapshot:<uuid> reference.".into());
            }
            OfficeOperation::WhiteboardSnapshot {
                reference,
                output: text(m, "output"),
            }
        }
        Some("preview") if path.get(1) == Some(&"prop") => {
            OfficeOperation::Prop(OfficePropOperation::Preview {
                file: required(m, "file"),
            })
        }
        Some("install") if path.get(1) == Some(&"prop") => {
            OfficeOperation::Prop(OfficePropOperation::Install {
                file: required(m, "file"),
                if_revision: *m
                    .get_one::<u64>("if-revision")
                    .expect("grammar supplies prop revision"),
            })
        }
        Some("remove") if path.get(1) == Some(&"prop") => {
            OfficeOperation::Prop(OfficePropOperation::Remove {
                digest: required(m, "prop-digest"),
                if_revision: *m
                    .get_one::<u64>("if-revision")
                    .expect("grammar supplies prop revision"),
            })
        }
        Some("list") if path.get(1) == Some(&"prop") => {
            OfficeOperation::Prop(OfficePropOperation::List {
                limit: *m
                    .get_one::<u64>("prop-limit")
                    .expect("grammar supplies prop list limit"),
                cursor: text(m, "cursor"),
            })
        }
        Some("show") if path.get(1) == Some(&"prop") => {
            OfficeOperation::Prop(OfficePropOperation::Show {
                digest: required(m, "prop-digest"),
            })
        }
        Some("post") if path.get(1) == Some(&"board") => {
            OfficeOperation::Board(OfficeBoardOperation::Post {
                category: board_category(m),
                actor: board_actor(m),
                title: required(m, "title"),
                body: board_body(m)?.expect("required board body"),
                operation_id: text(m, "operation-id"),
            })
        }
        Some("list") if path.get(1) == Some(&"board") => {
            OfficeOperation::Board(OfficeBoardOperation::List {
                category: board_category(m),
                view: required(m, "view"),
                author_id: text(m, "author-id"),
                owner: flag(m, "owner"),
                since: text(m, "since"),
                limit: *m
                    .get_one::<u32>("board-limit")
                    .expect("grammar supplies board limit"),
                cursor: text(m, "cursor"),
            })
        }
        Some("show") if path.get(1) == Some(&"board") => {
            OfficeOperation::Board(OfficeBoardOperation::Show {
                thread_id: required(m, "thread-id"),
                reply_limit: *m
                    .get_one::<u32>("reply-limit")
                    .expect("grammar supplies reply limit"),
                reply_cursor: text(m, "reply-cursor"),
            })
        }
        Some("reply") if path.get(1) == Some(&"board") => {
            OfficeOperation::Board(OfficeBoardOperation::Reply {
                thread_id: required(m, "thread-id"),
                actor: board_actor(m),
                body: board_body(m)?.expect("required board body"),
                operation_id: text(m, "operation-id"),
            })
        }
        Some("edit") if path.get(1) == Some(&"board") => {
            OfficeOperation::Board(OfficeBoardOperation::Edit {
                entry_id: required(m, "entry-id"),
                actor: board_actor(m),
                title: text(m, "title"),
                body: board_body(m)?,
                if_revision: *m
                    .get_one::<u64>("if-revision")
                    .expect("grammar supplies board revision"),
                operation_id: text(m, "operation-id"),
            })
        }
        Some("delete") if path.get(1) == Some(&"board") => {
            OfficeOperation::Board(OfficeBoardOperation::Delete {
                entry_id: required(m, "entry-id"),
                actor: board_actor(m),
                moderate: flag(m, "moderate"),
                if_revision: *m
                    .get_one::<u64>("if-revision")
                    .expect("grammar supplies board revision"),
                operation_id: text(m, "operation-id"),
            })
        }
        Some("sync") => OfficeOperation::Sync,
        Some("show") if path.get(1) == Some(&"layout") => {
            OfficeOperation::Layout(OfficeLayoutOperation::Show)
        }
        Some("apply") if path.get(1) == Some(&"layout") => {
            OfficeOperation::Layout(OfficeLayoutOperation::Apply {
                file: required(m, "file"),
                if_revision: *m
                    .get_one::<u64>("if-revision")
                    .expect("grammar supplies world revision"),
                legacy_basis: text(m, "legacy-basis"),
            })
        }
        Some("show") if path.get(1) == Some(&"block") => OfficeOperation::Block {
            target: office_block_target(m),
            identity: text(m, "identity"),
            operation: OfficeBlockOperation::Show {
                block_id: text(m, "block-id"),
            },
        },
        Some("apply") if path.get(1) == Some(&"block") => OfficeOperation::Block {
            target: office_block_target(m),
            identity: text(m, "identity"),
            operation: OfficeBlockOperation::Apply {
                block_id: text(m, "block-id"),
                file: required(m, "file"),
                if_revision: *m
                    .get_one::<u64>("if-revision")
                    .expect("grammar supplies block revision"),
            },
        },
        Some("show") if path.get(1) == Some(&"profile") => OfficeOperation::Profile {
            identity: text(m, "identity"),
            operation: OfficeProfileOperation::Show,
        },
        Some("apply") if path.get(1) == Some(&"profile") => OfficeOperation::Profile {
            identity: text(m, "identity"),
            operation: OfficeProfileOperation::Apply {
                file: required(m, "file"),
                if_revision: *m
                    .get_one::<u64>("if-revision")
                    .expect("grammar supplies profile revision"),
            },
        },
        Some("unpair") => OfficeOperation::Unpair {
            world: required(m, "world"),
            identity: text(m, "identity"),
            emulator: flag(m, "emulator"),
        },
        Some("inspect") => OfficeOperation::Inspect {
            world: required(m, "world"),
            identity: text(m, "identity"),
            emulator: flag(m, "emulator"),
        },
        Some("status") => match text(m, "world") {
            Some(world) => OfficeOperation::PairStatus {
                world,
                identity: text(m, "identity"),
                emulator: flag(m, "emulator"),
            },
            None => OfficeOperation::Status,
        },
        Some("pair") => OfficeOperation::Pair {
            world: required(m, "world"),
            identity: text(m, "identity"),
            emulator: flag(m, "emulator"),
            read_only: flag(m, "read-only"),
            timeout_seconds: *m
                .get_one::<u64>("timeout")
                .expect("grammar supplies pairing timeout"),
        },
        Some("install") => OfficeOperation::Install {
            yes: flag(m, "yes"),
            force: flag(m, "force"),
            archive: text(m, "archive"),
            manifest: text(m, "manifest"),
            channel: text(m, "channel")
                .and_then(|value| tmt_core::native_install::Channel::parse(&value)),
        },
        Some("upgrade") => OfficeOperation::Upgrade {
            force: flag(m, "force"),
            channel: text(m, "channel")
                .and_then(|value| tmt_core::native_install::Channel::parse(&value)),
        },
        Some("uninstall") => OfficeOperation::Uninstall {
            yes: flag(m, "yes"),
        },
        Some("start") => OfficeOperation::Start {
            port: m.get_one::<u16>("port").copied(),
        },
        Some("stop") => OfficeOperation::Stop,
        _ => OfficeOperation::Open,
    })
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
    text(matches, id).expect("required Office grammar operand was validated")
}

fn office_block_target(matches: &ArgMatches) -> OfficeBlockTarget {
    OfficeBlockTarget {
        world: required(matches, "world"),
        emulator: flag(matches, "emulator"),
    }
}

fn board_category(matches: &ArgMatches) -> BoardCategorySelection {
    if let Some(room) = text(matches, "room") {
        return BoardCategorySelection::Room(room);
    }
    match text(matches, "repo") {
        Some(name) => BoardCategorySelection::Repository(name),
        None => BoardCategorySelection::General,
    }
}
fn board_actor(matches: &ArgMatches) -> BoardActorSelection {
    if flag(matches, "owner") {
        BoardActorSelection::Owner
    } else {
        BoardActorSelection::Identity(text(matches, "identity"))
    }
}
fn board_body(matches: &ArgMatches) -> Result<Option<ContentInput>, String> {
    match (text(matches, "body"), text(matches, "file")) {
        (Some(body), None) => Ok(Some(ContentInput::Inline(body))),
        (None, Some(file)) if file == "-" => Ok(Some(ContentInput::Stdin)),
        (None, Some(file)) => Ok(Some(ContentInput::File(file))),
        (None, None) => Ok(None),
        _ => Err("Select exactly one of --body or --file.".into()),
    }
}

#[cfg(test)]
mod public_tests {
    use super::*;

    fn parse(words: &[&str]) -> Result<ParsedOffice, ParseOfficeError> {
        parse_public(&words.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn direct_parser_preserves_scope_and_rejects_retired_options() {
        let parsed = parse(&[
            "--prefix",
            "/isolated",
            "profile",
            "show",
            "--identity",
            "Alice",
            "--local",
            "--json",
        ])
        .unwrap();
        assert_eq!(parsed.prefix.as_deref(), Some("/isolated"));
        assert!(parsed.mode.json);
        assert_eq!(
            parsed.operation,
            OfficeOperation::Profile {
                identity: Some("Alice".into()),
                operation: OfficeProfileOperation::Show,
            }
        );
        assert_eq!(
            parse(&["status", "--team", "old", "--json"])
                .unwrap_err()
                .code,
            "UNSUPPORTED_TEAM"
        );
        assert!(
            parse(&["status", "--wait"])
                .unwrap_err()
                .message
                .contains("retired")
        );
        assert!(parse(&["status", "--absent"]).is_err());
    }
}
