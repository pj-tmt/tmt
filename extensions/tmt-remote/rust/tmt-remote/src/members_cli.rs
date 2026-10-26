mod machines;
// Owner-terminal people and machine views over one serving/offline read adapter.
use clap::{Arg, ArgMatches, Command};
use std::{
    io::Write,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
use tmt_cli_style::{
    CommandSpec, Example, OutputModes,
    table::{Cell, Column, Table},
};
use tmt_remote::{
    control,
    core::CoreClient,
    devices::Devices,
    error::RemoteError,
    members::{
        observations::NoMemberObservations,
        terminal::{self, MemberTerminalMutation, MemberTerminalRead, MemberTerminalTarget},
    },
    state::Layout,
    store::Store,
};

pub(crate) fn command() -> Command {
    let page = |command: Command| {
        command
            .arg(
                Arg::new("cursor")
                    .long("cursor")
                    .help("Continue the same bounded view"),
            )
            .arg(
                Arg::new("limit")
                    .long("limit")
                    .default_value("50")
                    .value_parser(
                        clap::value_parser!(u64)
                            .range(1..=tmt_remote::limits::MANAGEMENT_PAGE as u64),
                    )
                    .help("Maximum rows on this page"),
            )
    };
    let root = page(tmt_cli_style::command(&CommandSpec {
        name: "members", summary: "List people paired with this Remote",
        examples: &[Example {command:"tmt remote members --json",note:"Read the people roster and current revisions"}],
        outputs: OutputModes::HumanAndJson,
        details: "Local owner only. Works while Remote is stopped. Person names and machine names are separate; ambiguous names require a member ID.",
    }))
    .subcommand(page(tmt_cli_style::command(&CommandSpec {
        name: "machines", summary: "List one member's machines",
        examples: &[Example {command:"tmt remote members machines <member-id> --json",note:"Read machine IDs, scopes and activity"}],
        outputs: OutputModes::HumanAndJson,
        details: "Use a stable member ID or an exact unique person name. A disabled or expired machine remains an observation, not sending authority.",
    })).arg(Arg::new("member").required(true).help("Stable member ID or unique person name")));
    root
        .subcommand(member_action(CommandSpec { name:"rename", summary:"Rename one person", examples:&[Example {command:"tmt remote members rename <member-id> <name> --json",note:"Keep machine names and authority unchanged"}], outputs:OutputModes::HumanAndJson, details:"Use a stable member ID or unique person name. The current member revision is checked in the effect transaction." }).arg(Arg::new("name").required(true)))
        .subcommand(member_action(CommandSpec { name:"sending", summary:"Enable or disable sending for one person", examples:&[Example {command:"tmt remote members sending <member-id> on --json",note:"Change only that person's sending choice"}], outputs:OutputModes::HumanAndJson, details:"Other grant policy and explicit hold stay unchanged. Re-read members after an interrupted command before running it again." }).arg(Arg::new("enabled").required(true).value_parser(["on","off"])))
        .subcommand(member_action(CommandSpec { name:"revoke", summary:"Revoke one person and their machines", examples:&[Example {command:"tmt remote members revoke <member-id> --json",note:"End the person's access"}], outputs:OutputModes::HumanAndJson, details:"This revokes all machines belonging to the selected person. Re-read members after an interrupted command before running it again. An already-revoked member is reported without another change." }))
        .subcommands(machines::commands())
}

fn member_action(spec: CommandSpec) -> Command {
    tmt_cli_style::command(&spec)
        .arg(
            Arg::new("member")
                .required(true)
                .help("Stable member ID or exact unique person name"),
        )
        .arg(
            Arg::new("member-revision")
                .long("member-revision")
                .value_parser(clap::value_parser!(u64))
                .help("Require this observed member revision"),
        )
}
fn offline<T>(
    root: &std::path::Path,
    action: impl FnOnce(&Devices) -> Result<T, RemoteError>,
) -> Result<T, RemoteError> {
    let serving = Layout::open(root)?.serve_lock()?;
    let mut store = Store::open(&serving)?;
    store.machine()?;
    let devices = Devices::new(Arc::new(Mutex::new(store)), None);
    action(&devices)
}
fn mutation(
    action: &str,
    words: &ArgMatches,
    selected: &serde_json::Value,
) -> Result<MemberTerminalMutation, RemoteError> {
    let mut target: MemberTerminalTarget =
        serde_json::from_value(selected["target"].clone()).map_err(|_| invalid())?;
    if let Some(expected) = words.get_one::<u64>("member-revision") {
        target.member_revision = *expected;
    }
    Ok(match action {
        "rename" => MemberTerminalMutation::Rename {
            target,
            name: words.get_one::<String>("name").unwrap().clone(),
        },
        "sending" => MemberTerminalMutation::Sending {
            target,
            enabled: words.get_one::<String>("enabled").unwrap() == "on",
        },
        "revoke" => MemberTerminalMutation::Revoke { target },
        _ => unreachable!("typed member action"),
    })
}
fn no_page_options(parent: &ArgMatches) -> Result<(), RemoteError> {
    if parent.get_one::<String>("cursor").is_some()
        || parent.value_source("limit") == Some(clap::parser::ValueSource::CommandLine)
    {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn unconfirmed() -> RemoteError {
    RemoteError::new(
        "REMOTE_MEMBER_UNCONFIRMED",
        "Could not verify the change answer. Read tmt remote members before running it again.",
    )
}

fn member_effect(parent: &ArgMatches, action: &str, words: &ArgMatches) -> Result<(), RemoteError> {
    no_page_options(parent)?;
    let root = CoreClient::discover()?.storage_root(&AtomicBool::new(false))?;
    let read = MemberTerminalRead::Resolve {
        reference: words.get_one::<String>("member").unwrap().clone(),
    };
    let (request, answer) = match control::members_read(&root.join("remote"), &read)? {
        Some(selected) => {
            let request = mutation(action, words, &selected)?;
            let answer = control::members_mutate(&root.join("remote"), &request)?.ok_or_else(|| RemoteError::new("REMOTE_NOT_RUNNING", "Remote stopped before the change. Read tmt remote members before running it again."))?;
            (request, answer)
        }
        None => offline(&root, |devices| {
            let selected = terminal::read(devices, &NoMemberObservations, &read)?;
            let request = mutation(action, words, &selected)?;
            let answer = terminal::mutate(devices, &request)?;
            Ok((request, answer))
        })?,
    };
    let target = match &request {
        MemberTerminalMutation::Rename { target, .. }
        | MemberTerminalMutation::Sending { target, .. }
        | MemberTerminalMutation::Revoke { target } => target,
    };
    let person = &answer["member"];
    let matches_effect = match &request {
        MemberTerminalMutation::Rename { name, .. } => {
            person["displayName"].as_str() == Some(name.as_str())
        }
        MemberTerminalMutation::Sending { enabled, .. } => {
            person["sendingEnabled"].as_bool() == Some(*enabled)
        }
        MemberTerminalMutation::Revoke { .. } => person["revokedAtMs"].as_u64().is_some(),
    };
    if answer["state"] != "committed"
        || !answer["changed"].is_boolean()
        || !answer["alreadyRevoked"].is_boolean()
        || person["remoteId"].as_str() != Some(target.remote_id.as_str())
        || person["memberId"].as_str() != Some(target.member_id.as_str())
        || !person["revision"]
            .as_u64()
            .is_some_and(|r| r >= target.member_revision)
        || !matches_effect
    {
        return Err(unconfirmed());
    }
    let json = parent.get_flag("json") || words.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json);
    if json {
        writeln!(output, "{answer}")?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            if answer["alreadyRevoked"] == true {
                "Member already revoked"
            } else if answer["changed"] == true {
                "Member updated"
            } else {
                "Member unchanged"
            },
            &[
                (
                    "name",
                    answer["member"]["displayName"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned(),
                ),
                (
                    "member",
                    answer["member"]["memberId"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned(),
                ),
                ("revision", answer["member"]["revision"].to_string()),
            ],
        )?;
    }
    Ok(())
}

pub(crate) fn run(arguments: &ArgMatches) -> Result<(), RemoteError> {
    if let Some((action, words)) = arguments.subcommand()
        && machines::handles(action)
    {
        return machines::run(arguments, action, words);
    }
    if let Some((action @ ("rename" | "sending" | "revoke"), words)) = arguments.subcommand() {
        return member_effect(arguments, action, words);
    }
    let (member, words) = match arguments.subcommand() {
        Some(("machines", words)) => (Some(words.get_one::<String>("member").unwrap()), words),
        None => (None, arguments),
        _ => unreachable!("typed members grammar"),
    };
    let cursor = words
        .get_one::<String>("cursor")
        .or_else(|| arguments.get_one::<String>("cursor"))
        .cloned();
    let page = if words.value_source("limit") == Some(clap::parser::ValueSource::CommandLine) {
        words
    } else {
        arguments
    };
    let limit = usize::try_from(*page.get_one::<u64>("limit").unwrap()).map_err(|_| invalid())?;
    let request = match member {
        Some(reference) => MemberTerminalRead::Machines {
            reference: reference.clone(),
            cursor,
            limit,
        },
        None => MemberTerminalRead::Roster { cursor, limit },
    };
    let root = CoreClient::discover()?.storage_root(&AtomicBool::new(false))?;
    let answer = match control::members_read(&root.join("remote"), &request)? {
        Some(answer) => answer,
        None => offline(&root, |devices| {
            terminal::read(devices, &NoMemberObservations, &request)
        })?,
    };
    let json = arguments.get_flag("json") || words.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json);
    if json {
        writeln!(output, "{answer}")?;
        return Ok(());
    }
    let machines = member.is_some();
    let key = if machines { "machines" } else { "members" };
    let rows = answer[key].as_array().ok_or_else(invalid)?;
    let mut sorted: Vec<_> = rows.iter().collect();
    let name = if machines { "name" } else { "displayName" };
    sorted.sort_by_key(|row| row[name].as_str().unwrap_or(""));
    let mut table = Table::new(&[Column::Name, Column::Detail, Column::Fixed, Column::Detail]);
    for row in sorted {
        let identity = if machines { "clientId" } else { "memberId" };
        let policy = if machines {
            "sendingScope"
        } else {
            "sendingEnabled"
        };
        let sending = if row[policy] == true {
            "Sending on"
        } else {
            "Sending off"
        };
        let state = if machines {
            if row["revoked"] == true {
                "Revoked"
            } else if row["grantLive"] == false {
                "Inactive"
            } else {
                "Active"
            }
        } else if row["revokedAtMs"].is_number() {
            "Revoked"
        } else {
            "Active"
        };
        table.row([
            Cell::from(row[name].as_str().unwrap_or("")),
            Cell::from(row[identity].as_str().unwrap_or("")),
            Cell::from(sending),
            Cell::from(state),
        ]);
    }
    let terminal = output.terminal();
    tmt_cli_style::list::Section {
        title: if machines { "MACHINES" } else { "MEMBERS" },
        count: Some(rows.len()),
        rows: table,
        note: None,
        hint: None,
    }
    .write(&mut output, terminal)?;
    if let Some(cursor) = answer["nextCursor"].as_str() {
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            "Next page",
            &[("cursor", cursor.to_owned())],
        )?;
    }
    Ok(())
}
fn invalid() -> RemoteError {
    RemoteError::new(
        "REMOTE_INPUT_INVALID",
        "Invalid member command input or result.",
    )
}
