//! Explicit machine operations and owner-browser recovery under the members command.
use super::*;
use tmt_remote::members::terminal_machines::{self, MemberMachineMutation, MemberMachineTarget};

pub(super) fn commands() -> Vec<Command> {
    vec![
        target(CommandSpec {name:"rename-machine",summary:"Rename one machine",examples:&[Example {command:"tmt remote members rename-machine <client-id> <name> --json",note:"Keep the person name and authority unchanged"}],outputs:OutputModes::HumanAndJson,details:"Use an exact machine client ID shown by members machines. This does not rename the person."}).arg(Arg::new("name").required(true)),
        target(CommandSpec {name:"remove-machine",summary:"Revoke one exact machine",examples:&[Example {command:"tmt remote members remove-machine <client-id> --json",note:"Keep the person's other machines"}],outputs:OutputModes::HumanAndJson,details:"Only the selected machine is revoked. The person is revoked only when no machine remains active."}),
        target(CommandSpec {name:"designate",summary:"Designate the owner's browser",examples:&[Example {command:"tmt remote members designate <client-id> --json",note:"Authorize this exact local browser to manage Remote"}],outputs:OutputModes::HumanAndJson,details:"Only a live browser for this door can be designated. Local owner only; no automatic designation. Pairing and agent policy stay unchanged."}),
        revisions(tmt_cli_style::command(&CommandSpec {name:"undesignate",summary:"Remove browser management authority",examples:&[Example {command:"tmt remote members undesignate --json",note:"Keep browser pairing and agent grants"}],outputs:OutputModes::HumanAndJson,details:"Local recovery works while Remote is stopped. The captured designation and its member/machine revisions are checked before removal."})),
    ]
}
fn revisions(command: Command) -> Command {
    command
        .arg(
            Arg::new("member-revision")
                .long("member-revision")
                .value_parser(clap::value_parser!(u64))
                .help("Require this observed member revision"),
        )
        .arg(
            Arg::new("grant-revision")
                .long("grant-revision")
                .value_parser(clap::value_parser!(u64))
                .help("Require this observed machine grant revision"),
        )
}
fn target(spec: CommandSpec) -> Command {
    revisions(tmt_cli_style::command(&spec)).arg(
        Arg::new("client-id")
            .required(true)
            .help("Exact machine client ID"),
    )
}
pub(super) fn handles(action: &str) -> bool {
    matches!(
        action,
        "rename-machine" | "remove-machine" | "designate" | "undesignate"
    )
}
fn mutation(
    action: &str,
    words: &ArgMatches,
    selected: &serde_json::Value,
) -> Result<MemberMachineMutation, RemoteError> {
    let target = if selected["target"].is_null() {
        None
    } else {
        let mut target: MemberMachineTarget =
            serde_json::from_value(selected["target"].clone()).map_err(|_| invalid())?;
        if let Some(value) = words.get_one::<u64>("member-revision") {
            target.member.member_revision = *value;
        }
        if let Some(value) = words.get_one::<u64>("grant-revision") {
            target.grant_revision = *value;
        }
        Some(target)
    };
    if action == "undesignate" {
        if target.is_none()
            && (words.get_one::<u64>("member-revision").is_some()
                || words.get_one::<u64>("grant-revision").is_some())
        {
            return Err(invalid());
        }
        return Ok(MemberMachineMutation::Undesignate { expected: target });
    }
    let target = target.ok_or_else(invalid)?;
    Ok(match action {
        "rename-machine" => MemberMachineMutation::Rename {
            target,
            name: words.get_one::<String>("name").unwrap().clone(),
        },
        "remove-machine" => MemberMachineMutation::Remove { target },
        "designate" => MemberMachineMutation::Designate { target },
        _ => unreachable!("typed exact-machine grammar"),
    })
}
pub(super) fn run(
    parent: &ArgMatches,
    action: &str,
    words: &ArgMatches,
) -> Result<(), RemoteError> {
    no_page_options(parent)?;
    let read = if action == "undesignate" {
        MemberTerminalRead::Designation
    } else {
        MemberTerminalRead::ResolveMachine {
            client_id: words.get_one::<String>("client-id").unwrap().clone(),
        }
    };
    let root = CoreClient::discover()?.storage_root(&AtomicBool::new(false))?;
    let answer = match control::members_read(&root.join("remote"), &read)? {
        Some(selected) => {
            let request = mutation(action, words, &selected)?;
            control::members_machine_mutate(&root.join("remote"),&request)?.ok_or_else(||RemoteError::new("REMOTE_NOT_RUNNING","Remote stopped before the change. Read tmt remote members before running it again."))?
        }
        None => offline(&root, |devices| {
            let selected = terminal::read(devices, &NoMemberObservations, &read)?;
            terminal_machines::mutate(devices, &mutation(action, words, &selected)?, None)
        })?,
    };
    if answer["state"] != "committed" || !answer["changed"].is_boolean() {
        return Err(unconfirmed());
    }
    if action == "designate" {
        if answer["designatedClientId"].as_str()
            != words.get_one::<String>("client-id").map(String::as_str)
        {
            return Err(unconfirmed());
        }
    } else if action == "undesignate" {
        if !answer
            .as_object()
            .is_some_and(|r| r.contains_key("designatedClientId"))
            || !answer["designatedClientId"].is_null()
        {
            return Err(unconfirmed());
        }
    } else if answer["machine"]["clientId"].as_str()
        != words.get_one::<String>("client-id").map(String::as_str)
        || answer["machine"]["revision"].as_u64().is_none()
        || !answer["machine"]["revoked"].is_boolean()
        || action == "rename-machine"
            && answer["machine"]["name"].as_str()
                != words.get_one::<String>("name").map(String::as_str)
        || action == "remove-machine" && answer["machine"]["revoked"] != true
    {
        return Err(unconfirmed());
    }
    let json = parent.get_flag("json") || words.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json);
    if json {
        writeln!(output, "{answer}")?;
    } else {
        let terminal = output.terminal();
        let fields = if action == "designate" || action == "undesignate" {
            vec![(
                "browser",
                answer["designatedClientId"]
                    .as_str()
                    .unwrap_or("None")
                    .to_owned(),
            )]
        } else {
            if !answer["machine"].is_object() {
                return Err(unconfirmed());
            }
            vec![
                (
                    "name",
                    answer["machine"]["name"].as_str().unwrap_or("").to_owned(),
                ),
                (
                    "client",
                    answer["machine"]["clientId"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned(),
                ),
                ("revision", answer["machine"]["revision"].to_string()),
            ]
        };
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            if answer["changed"] == true {
                "Machine management updated"
            } else {
                "Machine management unchanged"
            },
            &fields,
        )?;
    }
    Ok(())
}
