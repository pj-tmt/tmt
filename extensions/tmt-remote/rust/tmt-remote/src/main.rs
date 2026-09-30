use clap::{Arg, ArgAction, Command};
use serde_json::json;
use std::{
    io::Write,
    process::ExitCode,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use tmt_cli_style::{CommandSpec, Example, OutputModes, Route};
use tmt_remote::{
    core::CoreClient,
    error::RemoteError,
    http::{Door, ServeOptions},
};
const ROOT: CommandSpec = CommandSpec {
    name: "remote",
    summary: "Optional local remote door (deny-all pilot)",
    examples: &[Example {
        command: "tmt remote serve",
        note: "Open a finite loopback door; all requests are refused",
    }],
    outputs: OutputModes::Human,
    details: "Pairing and sends are not implemented. Core never listens.",
};
const SERVE: CommandSpec = CommandSpec {
    name: "serve",
    summary: "Run a foreground IPv4-loopback deny-all door",
    examples: &[Example {
        command: "tmt remote serve --json",
        note: "Print the bound descriptor for local testing",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Stops on interrupt, window expiry or 15 minutes without authorized activity.\nAll remote requests are refused; no core operation is forwarded.",
};
fn grammar() -> Command {
    tmt_cli_style::command(&ROOT)
        .bin_name("tmt remote")
        .version(env!("CARGO_PKG_VERSION"))
        .arg(tmt_cli_style::version_arg(ArgAction::Version))
        .subcommand_required(true)
        .subcommand(
            tmt_cli_style::command(&SERVE)
                .arg(
                    Arg::new("port")
                        .long("port")
                        .default_value("0")
                        .value_parser(clap::value_parser!(u16))
                        .help("Loopback port; 0 selects an unused port"),
                )
                .arg(
                    Arg::new("window-seconds")
                        .long("window-seconds")
                        .default_value("3600")
                        .value_parser(clap::value_parser!(u64).range(1..=86400))
                        .help("Hard window, at most 86400 seconds"),
                ),
        )
}
fn run(matches: &clap::ArgMatches) -> Result<(), RemoteError> {
    let (_, serve) = matches.subcommand().expect("required serve");
    let stop = Arc::new(AtomicBool::new(false));
    let signals = [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM]
        .map(|signal| signal_hook::flag::register(signal, Arc::clone(&stop)));
    let result = (|| {
        if signals.iter().any(Result::is_err) {
            return Err(RemoteError::new(
                "REMOTE_SIGNAL",
                "Could not register foreground shutdown.",
            ));
        }
        let capabilities = CoreClient::discover()?.capabilities(&stop)?;
        if capabilities["version"] != 1
            || capabilities["limits"]["outputBytes"]
                .as_u64()
                .is_none_or(|n| n == 0 || n > tmt_remote::core::OUTPUT_LIMIT as u64)
        {
            return Err(RemoteError::new(
                "REMOTE_CORE_UNAVAILABLE",
                "Core advertised an unsupported protocol or output bound.",
            ));
        }
        let input_limit = capabilities["limits"]["inputBytes"]
            .as_u64()
            .filter(|n| *n > 0 && *n <= 16 * 1024 * 1024)
            .ok_or_else(|| {
                RemoteError::new(
                    "REMOTE_CORE_UNAVAILABLE",
                    "Core did not advertise a valid input bound.",
                )
            })? as usize;
        let door = Door::bind(ServeOptions {
            port: *serve.get_one::<u16>("port").unwrap(),
            window: Duration::from_secs(*serve.get_one::<u64>("window-seconds").unwrap()),
            input_limit,
        })?;
        let json_output = serve.get_flag("json");
        let mut output = tmt_cli_style::stream::stdout(json_output);
        if json_output {
            writeln!(
                output,
                "{}",
                json!({"profile":"local-v1","binding":"loopback-http","state":"closed","address":door.address,"startupCoreCalls":1})
            )?;
        } else {
            let terminal = output.terminal();
            tmt_cli_style::message::warning(
                &mut output,
                terminal,
                &format!(
                    "Deny-all door bound at {}; pairing and sends unavailable",
                    door.address
                ),
                None,
            )?;
        }
        output.flush()?;
        drop(output);
        door.run(&stop)
    })();
    for id in signals.into_iter().flatten() {
        signal_hook::low_level::unregister(id);
    }
    result
}
fn main() -> ExitCode {
    let words: Vec<String> = std::env::args().skip(1).collect();
    let grammar = grammar();
    if let Route::Help(mut command) = tmt_cli_style::route(&grammar, &words) {
        let _ = command.print_help();
        return ExitCode::SUCCESS;
    }
    let matches = match grammar.try_get_matches() {
        Ok(m) => m,
        Err(e) => {
            let success = matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            );
            let _ = e.print();
            return if success {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
    };
    match run(&matches) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if matches
                .subcommand()
                .is_some_and(|(_, m)| m.get_flag("json"))
            {
                let _ = writeln!(
                    tmt_cli_style::stream::stdout(true),
                    "{}",
                    json!({"error":{"code":error.code,"message":error.message}})
                );
            } else {
                let mut output = tmt_cli_style::stream::stderr();
                let terminal = output.terminal();
                let _ = tmt_cli_style::message::error(&mut output, terminal, &error.message, None);
            }
            ExitCode::FAILURE
        }
    }
}
