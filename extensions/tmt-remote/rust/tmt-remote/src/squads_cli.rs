//! Public cache/delivery reads compose the running Remote owner, never a transport.
use clap::{Arg, ArgAction, ArgMatches, Command};
use std::io::Write;
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_remote::{canonical, control, error::RemoteError, remote_catalog};

pub(crate) fn command(delivery: bool) -> Command {
    let spec = CommandSpec {
        name: if delivery { "delivery" } else { "squads" },
        summary: if delivery {
            "Observe one original cloud delivery"
        } else {
            "List cached squads on other machines"
        },
        examples: if delivery {
            &[Example {
                command: "tmt remote delivery <operation-id> --remote <alias> --json",
                note: "One read-only original-ID observation",
            }]
        } else {
            &[Example {
                command: "tmt remote squads --json",
                note: "Retained verified observations, not presence",
            }]
        },
        outputs: OutputModes::Json,
        details: if delivery {
            "Requires the original operation UUID and a paired Remote alias. Observes once without resending. Unknown and unconfirmed are not delivery failures. Nothing opens, signs in or starts automatically."
        } else {
            "Reads the running owner's retained verified cache only. Lists other paired machines, with per-machine unavailable/incompatible states and original observation timestamps. A cached summary is not current presence. No provider request or pairing is started."
        },
    };
    let mut command = tmt_cli_style::command(&spec).arg(
        Arg::new("json")
            .long("json")
            .required(true)
            .action(ArgAction::SetTrue),
    );
    if delivery {
        command = command.arg(Arg::new("operation-id").required(true));
    }
    command
}
pub(crate) fn run(
    args: &ArgMatches,
    delivery: bool,
    remote: Option<&str>,
) -> Result<(), RemoteError> {
    let operation = if delivery {
        args.get_one::<String>("operation-id").map(String::as_str)
    } else {
        None
    };
    if delivery
        && (operation.is_none_or(|id| !canonical::is_core_id(id))
            || remote.is_none_or(|name| name == "local" || !remote_catalog::name_ok(name)))
        || !delivery && remote.is_some()
    {
        return Err(RemoteError::new(
            "REMOTE_INPUT_INVALID",
            "Use a canonical operation UUID and an explicit paired Remote alias for delivery; squads takes no selector.",
        ));
    }
    let root = super::discovery_root()?;
    let answer = if delivery {
        control::squad_delivery(&root.join("remote"), remote.unwrap(), operation.unwrap())?
    } else {
        control::squad_list(&root.join("remote"))?
    };
    writeln!(tmt_cli_style::stream::stdout(true), "{answer}")?;
    Ok(())
}
