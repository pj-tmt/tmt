//! Hidden host protocol. Remote alone discovers/adopts its private control path.
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::json;
use std::{
    io::Write,
    process::ExitCode,
    sync::atomic::AtomicBool,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tmt_remote::{
    control::{self, ActivationFailure},
    core::CoreClient,
    error::RemoteError,
    state::Layout,
};

pub(super) fn command() -> Command {
    Command::new("__activate")
        .hide(true)
        .arg(Arg::new("extension").long("extension").required(true))
        .arg(Arg::new("deadline-ms").long("deadline-ms").required(true))
        .arg(
            Arg::new("json")
                .long("json")
                .action(ArgAction::SetTrue)
                .required(true),
        )
}
fn invalid() -> ActivationFailure {
    ActivationFailure::Refused(RemoteError::new(
        "REMOTE_INPUT_INVALID",
        "Invalid internal native activation input.",
    ))
}
fn deadline(value: &str) -> Result<Instant, ActivationFailure> {
    let absolute = value
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0 && n.to_string() == value)
        .ok_or_else(invalid)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid())?
        .as_millis();
    let remaining = u128::from(absolute)
        .checked_sub(now)
        .filter(|n| *n > 0)
        .ok_or(ActivationFailure::Deadline)?;
    Instant::now()
        .checked_add(Duration::from_millis(
            u64::try_from(remaining).map_err(|_| invalid())?,
        ))
        .ok_or_else(invalid)
}
fn activate(args: &ArgMatches) -> Result<String, ActivationFailure> {
    let extension = args.get_one::<String>("extension").ok_or_else(invalid)?;
    if !tmt_remote::canonical::extension_name(extension)
        || !args.get_flag("json")
        || args
            .try_get_one::<String>("remote")
            .ok()
            .flatten()
            .is_some()
    {
        return Err(invalid());
    }
    // A spent deadline makes no filesystem lookup or core/control invocation.
    let until = deadline(args.get_one::<String>("deadline-ms").ok_or_else(invalid)?)?;
    let client = CoreClient::discover().map_err(ActivationFailure::Refused)?;
    let root = client
        .storage_root_until(&AtomicBool::new(false), until)
        .map_err(|error| {
            if Instant::now() >= until {
                ActivationFailure::Deadline
            } else {
                ActivationFailure::Refused(error)
            }
        })?;
    if Instant::now() >= until {
        return Err(ActivationFailure::Deadline);
    }
    let layout = Layout::existing(&root)
        .map_err(ActivationFailure::Refused)?
        .ok_or_else(|| {
            ActivationFailure::Refused(RemoteError::new(
                "REMOTE_NOT_RUNNING",
                "Remote is not running.",
            ))
        })?;
    control::activate_native_until(&layout.directory, extension, until)
}
pub(super) fn run(args: &ArgMatches) -> ExitCode {
    let (value, exit) = match activate(args) {
        Ok(generation) => (json!({"generation":generation}), ExitCode::SUCCESS),
        Err(ActivationFailure::Deadline) => (
            json!({"error":{"code":"REMOTE_STATE_UNAVAILABLE","message":"Native sync activation deadline elapsed.","state":"deadline"}}),
            ExitCode::FAILURE,
        ),
        Err(ActivationFailure::Refused(error)) => (
            json!({"error":{"code":error.code,"message":error.json_message()}}),
            ExitCode::FAILURE,
        ),
    };
    if writeln!(tmt_cli_style::stream::stdout(true), "{value}").is_err() {
        ExitCode::FAILURE
    } else {
        exit
    }
}
