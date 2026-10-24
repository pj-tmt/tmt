//! Terminal adapters for the owner's explicit native pairing ceremony.
use clap::{Arg, ArgMatches, Command};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_remote::{control, error::RemoteError};

pub fn command(join: bool) -> Command {
    let spec = CommandSpec {
        name: if join { "join" } else { "invite" },
        summary: if join {
            "Join another person's machine"
        } else {
            "Invite someone to this machine"
        },
        examples: if join {
            &[Example {
                command: "tmt remote join <code>",
                note: "Compare the four words and confirm in the owner's terminal",
            }]
        } else {
            &[Example {
                command: "tmt remote invite",
                note: "Compare the four words and confirm in the owner's terminal",
            }]
        },
        outputs: OutputModes::HumanAndJson,
        details: "Requires an interactive terminal. Confirmation never happens automatically; sending defaults to off. No --yes or --talk shortcut. The one-time invite expires in ten minutes.",
    };
    let mut cmd = tmt_cli_style::command(&spec);
    if join {
        cmd = cmd.arg(Arg::new("code").required(true));
    }
    cmd
}
pub(crate) fn connect(stop: &AtomicBool) -> Result<UnixStream, RemoteError> {
    let root = super::discovery_root()?;
    match control::connect(&root.join("remote")) {
        Ok(stream) => Ok(stream),
        Err(error) if error.code == "REMOTE_NOT_RUNNING" => {
            super::serve::launch_background(None, stop)?;
            control::connect(&root.join("remote"))
        }
        Err(error) => Err(error),
    }
}
pub fn run(arguments: &ArgMatches, join: bool, remote: Option<&str>) -> Result<(), RemoteError> {
    if join && remote.is_some() {
        return Err(RemoteError::new(
            "REMOTE_SELECTOR_INVALID",
            "join selects its machine from the invite code; omit --remote.",
        ));
    }
    let json_output = arguments.get_flag("json");
    if !tmt_remote::deploy_cli::may_prompt(
        tmt_cli_style::Interaction::detect(json_output),
        std::env::var_os("TMT_AGENT").is_some(),
        std::env::var_os("CI").is_some(),
    ) {
        return Err(RemoteError::new(
            "REMOTE_CONFIRMATION_REQUIRED",
            "Confirmation needs the owner's terminal. Run tmt remote invite there.",
        ));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let signals = [
        signal_hook::flag::register(signal_hook::consts::SIGINT, stop.clone())?,
        signal_hook::flag::register(signal_hook::consts::SIGTERM, stop.clone())?,
    ];
    struct Signals(Vec<signal_hook::SigId>);
    impl Drop for Signals {
        fn drop(&mut self) {
            for id in self.0.drain(..) {
                signal_hook::low_level::unregister(id);
            }
        }
    }
    let _signals = Signals(signals.into());
    let mut stream = connect(&stop)?;
    stream.set_read_timeout(Some(Duration::from_millis(
        tmt_remote::limits::DEPLOY_CONFIRM_POLL_MS as u64,
    )))?;
    let request = if join {
        json!({"op":"cloud.join","code":arguments.get_one::<String>("code").unwrap()})
    } else {
        json!({"op":"cloud.invite","remote":remote})
    };
    writeln!(stream, "{request}")?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    let mut output = tmt_cli_style::stream::stdout(false);
    loop {
        if stop.load(Ordering::Acquire) {
            if !join {
                writeln!(output, "Invite cancelled. The code no longer works.")?;
            }
            return Ok(());
        }
        match reader.read_line(&mut line) {
            Ok(0) => {
                return Err(RemoteError::new(
                    "REMOTE_CLOUD_UNCONFIRMED",
                    "The connection could not be confirmed.",
                ));
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(e) => return Err(e.into()),
            Ok(_) => {}
        }
        if line.len() > tmt_remote::limits::CLOUD_PAIR_CONTROL_BYTES {
            return Err(RemoteError::new(
                "REMOTE_INPUT_INVALID",
                "Invalid pairing response.",
            ));
        }
        let event: Value = serde_json::from_str(&line)
            .map_err(|_| RemoteError::new("REMOTE_INPUT_INVALID", "Invalid pairing response."))?;
        line.clear();
        if let Some(error) = event.get("error") {
            return Err(RemoteError::new(
                error["code"].as_str().unwrap_or("REMOTE_CLOUD_UNAVAILABLE"),
                error["message"]
                    .as_str()
                    .unwrap_or("Cloud pairing is unavailable."),
            ));
        }
        let name = event["name"].as_str().unwrap_or("This machine");
        let words = event["words"]
            .as_array()
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        match event["event"].as_str() {
            Some("invite") => {
                writeln!(
                    output,
                    "Invite someone to this machine\nShare this code privately: {}\nIt expires in 10 minutes and can be used once.\nWaiting for them to join… (Ctrl+C cancels this invite)",
                    event["code"].as_str().unwrap_or("")
                )?;
            }
            Some("joining") => {
                writeln!(
                    output,
                    "Connecting to {name}…\nCompare these words with the owner: {words}\nWaiting for the owner to confirm…"
                )?;
            }
            Some("candidate") => {
                writeln!(
                    output,
                    "{name} wants to connect.\nCompare these words with them: {words}"
                )?;
                output.flush()?;
                let matched = super::confirm_terminal(
                    &mut std::io::stdin().lock(),
                    &mut tmt_cli_style::stream::stderr(),
                    &stop,
                    "Do the words match? [y/N]",
                )?;
                let talk = matched
                    && super::confirm_terminal(
                        &mut std::io::stdin().lock(),
                        &mut tmt_cli_style::stream::stderr(),
                        &stop,
                        &format!("Let {name} send messages to your agents? [y/N]"),
                    )?;
                if stop.load(Ordering::Acquire) {
                    continue;
                }
                writeln!(stream, "{}", json!({"confirm":matched,"talk":talk}))?;
            }
            Some("reverse-permission") => {
                let talk = super::confirm_terminal(
                    &mut std::io::stdin().lock(),
                    &mut tmt_cli_style::stream::stderr(),
                    &stop,
                    &format!("Let {name} send messages to your agents? [y/N]"),
                )?;
                if stop.load(Ordering::Acquire) {
                    continue;
                }
                writeln!(stream, "{}", json!({"talk":talk}))?;
            }
            Some("connected") => {
                let talk = event["talk"] == true;
                if join {
                    writeln!(
                        output,
                        "Connected to {name}. {}",
                        if talk {
                            "You can send messages."
                        } else {
                            "Sending is off until the owner allows it."
                        }
                    )?;
                } else {
                    writeln!(
                        output,
                        "Connected {name}. Sending is {}.",
                        if talk { "on" } else { "off" }
                    )?;
                    if !talk {
                        let person = event["hintPerson"].as_str().unwrap_or(name);
                        writeln!(
                            output,
                            "To allow it later: tmt remote devices talk {} on",
                            tmt_remote::state::shell_quote(person)
                        )?;
                    }
                }
                if let Some(reference) = event["remote"].as_str() {
                    writeln!(output, "Remote: {reference}")?;
                }
                return Ok(());
            }
            _ => {}
        }
        output.flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_flags_can_turn_terminal_confirmation_into_consent() {
        for join in [false, true] {
            for flag in ["--yes", "--talk", "--no-open"] {
                let mut args = vec![if join { "join" } else { "invite" }];
                if join {
                    args.push("code");
                }
                args.push(flag);
                assert!(command(join).try_get_matches_from(args).is_err());
            }
        }
    }
}
