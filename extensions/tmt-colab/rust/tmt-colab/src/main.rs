use clap::{Arg, Command};
use serde_json::json;
use std::{
    io::Write,
    process::ExitCode,
    sync::{Arc, atomic::AtomicBool},
};
use tmt_cli_style::{CommandSpec, Example, OutputModes, Route};
use tmt_colab::{
    Result, core,
    http::Door,
    keyring::{Keyring, Layout},
    store::Store,
};

fn grammar() -> Command {
    const ROOT: CommandSpec = CommandSpec {
        name: "colab",
        summary: "Local collaborative-space pilot",
        examples: &[Example {
            command: "tmt colab serve",
            note: "Run the local foreground space",
        }],
        outputs: OutputModes::Human,
        details: "Sign-in and sync are not available in this slice.",
    };
    const SERVE: CommandSpec = CommandSpec {
        name: "serve",
        summary: "Serve a local space in the foreground",
        examples: &[Example {
            command: "tmt colab serve --port 0",
            note: "Choose a free loopback port",
        }],
        outputs: OutputModes::HumanAndJson,
        details: "Ctrl-C or SIGTERM closes the listener and all workers. APIs and upgrades are denied.",
    };
    const SPACES: CommandSpec = CommandSpec {
        name: "spaces",
        summary: "List the local space and its running state",
        examples: &[Example {
            command: "tmt colab spaces --json",
            note: "Read configured local-space metadata",
        }],
        outputs: OutputModes::HumanAndJson,
        details: "Uses only the extension subtree of core's reported data directory.",
    };
    tmt_cli_style::command(&ROOT)
        .bin_name("tmt colab")
        .subcommand_required(true)
        .subcommand(
            tmt_cli_style::command(&SERVE).arg(
                Arg::new("port")
                    .long("port")
                    .default_value("7341")
                    .value_parser(clap::value_parser!(u16))
                    .help("IPv4 loopback port (default 7341); 0 selects a free port"),
            ),
        )
        .subcommand(tmt_cli_style::command(&SPACES))
}
fn run(matches: &clap::ArgMatches) -> Result<()> {
    let (command, args) = matches.subcommand().expect("required subcommand");
    let stop = Arc::new(AtomicBool::new(false));
    let mut signals = Vec::new();
    let result = (|| -> Result<()> {
        for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
            signals.push(signal_hook::flag::register(signal, Arc::clone(&stop))?);
        }
        let root = core::data_root(&stop)?;
        let json_output = args.get_flag("json");
        if command == "spaces" {
            return spaces(&root, json_output);
        }
        let layout = Layout::open(&root)?;
        let _lock = layout.serve_lock()?;
        let keyring = Keyring::open(&layout)?;
        let mut output = tmt_cli_style::stream::stdout(json_output);
        let store = Store::open(&layout)?;
        let door = Door::bind(*args.get_one::<u16>("port").unwrap())?;
        if json_output {
            writeln!(
                output,
                "{}",
                json!({"spaceId":keyring.space_id,"url":door.address,"profile":"colab-sync-v1","state":"unauthenticated"})
            )?;
        } else {
            let terminal = output.terminal();
            tmt_cli_style::detail::write(
                &mut output,
                terminal,
                "LOCAL SPACE",
                &[
                    ("space", keyring.space_id),
                    ("url", door.address.clone()),
                    ("state", "placeholder; sign-in and sync unavailable".into()),
                ],
            )?;
        }
        output.flush()?;
        drop(output);
        let result = door.run(&stop);
        let closed = store.close();
        result?;
        closed
    })();
    for signal in signals {
        signal_hook::low_level::unregister(signal);
    }
    result
}
fn spaces(root: &std::path::Path, json_output: bool) -> Result<()> {
    let space = Layout::existing(root)?
        .map(|layout| {
            let exists = match std::fs::symlink_metadata(layout.directory.join("owner.key")) {
                Ok(_) => true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(e) => return Err(e.into()),
            };
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(if exists {
                Some(layout)
            } else {
                None
            })
        })
        .transpose()?
        .flatten()
        .map(|layout| {
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>((
                Keyring::read(&layout)?.space_id,
                layout.running()?,
            ))
        })
        .transpose()?;
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        let values: Vec<_> = space
            .into_iter()
            .map(|(id, running)| json!({"spaceId":id,"backend":"local","running":running}))
            .collect();
        writeln!(output, "{}", json!({"spaces":values}))?;
    } else {
        use tmt_cli_style::{
            list::Section,
            mark::Mark,
            table::{Cell, Column, Table},
        };
        let mut rows = Table::new(&[Column::Fixed, Column::Fixed, Column::Fixed]);
        if let Some((id, running)) = &space {
            let mark = if *running {
                Mark::Running
            } else {
                Mark::Offline
            };
            rows.row([
                Cell::styled(mark.symbol(), mark.token()),
                id.clone().into(),
                "local".into(),
            ]);
        }
        let terminal = output.terminal();
        Section {
            title: "SPACES",
            count: Some(usize::from(space.is_some())),
            rows,
            note: None,
            hint: None,
        }
        .write(&mut output, terminal)?;
    }
    Ok(())
}
fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("__decoder") {
        return tmt_colab::decoder::child_main();
    }
    let words: Vec<String> = std::env::args().skip(1).collect();
    let json_output = words.iter().any(|s| s == "--json");
    let command = if json_output {
        grammar().color(clap::ColorChoice::Never)
    } else {
        grammar()
    };
    if let Route::Help(mut command) = tmt_cli_style::route(&command, &words) {
        let _ = command.print_help();
        return ExitCode::SUCCESS;
    }
    let matches = match command.try_get_matches() {
        Ok(m) => m,
        Err(e) => {
            let help = matches!(e.kind(), clap::error::ErrorKind::DisplayHelp);
            if json_output && !help {
                let _ = writeln!(
                    tmt_cli_style::stream::stdout(true),
                    "{}",
                    json!({"error":{"code":"COLAB_INPUT_INVALID","message":e.to_string()}})
                );
            } else {
                let _ = e.print();
            }
            return if help {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
    };
    match run(&matches) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let code = error
                .downcast_ref::<tmt_colab::keyring::StateFault>()
                .map(|e| e.code())
                .unwrap_or_else(|| {
                    if matches!(
                        error.downcast_ref::<tmt_colab::store::Fault>(),
                        Some(tmt_colab::store::Fault::UnsupportedSchema(_))
                    ) {
                        "COLAB_SCHEMA_UNSUPPORTED"
                    } else {
                        "COLAB_UNAVAILABLE"
                    }
                });
            if matches
                .subcommand()
                .is_some_and(|(_, m)| m.get_flag("json"))
            {
                let _ = writeln!(
                    tmt_cli_style::stream::stdout(true),
                    "{}",
                    json!({"error":{"code":code,"message":error.to_string()}})
                );
            } else {
                let mut output = tmt_cli_style::stream::stderr();
                let terminal = output.terminal();
                let _ =
                    tmt_cli_style::message::error(&mut output, terminal, &error.to_string(), None);
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serve_defaults_to_the_fixed_port() {
        let matches = grammar().try_get_matches_from(["colab", "serve"]).unwrap();
        assert_eq!(
            matches
                .subcommand_matches("serve")
                .unwrap()
                .get_one::<u16>("port"),
            Some(&7341)
        );
    }
    #[test]
    fn help_and_examples_obey_shared_style() {
        let help = |words: &[String]| match tmt_cli_style::route(&grammar(), words) {
            Route::Help(command) => Ok(tmt_cli_style::help_text(
                &command,
                tmt_cli_style::Terminal::PLAIN,
            )),
            _ => match grammar().try_get_matches_from(
                std::iter::once("colab").chain(words.iter().map(String::as_str)),
            ) {
                Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => Ok(e.to_string()),
                Err(e) => Err(e.to_string()),
                Ok(_) => Err("Not a help request.".into()),
            },
        };
        let parse = |words: &[String]| {
            grammar()
                .try_get_matches_from(
                    std::iter::once("colab").chain(words.iter().map(String::as_str)),
                )
                .map(|_| ())
                .map_err(|e| e.to_string())
        };
        let violations = tmt_cli_style::audit::walk(
            &grammar(),
            &tmt_cli_style::audit::Probe {
                program: &["tmt", "colab"],
                help: &help,
                parse: &parse,
            },
        );
        assert!(violations.is_empty(), "{violations:?}");
    }
}
