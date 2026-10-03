mod cli_grammar;
mod cli_management;
const IPC_RESPONSE_BYTES: usize = 8192;
use clap::{Arg, ArgAction, Command};
use serde_json::json;
use std::{
    io::Write,
    process::ExitCode,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
use tmt_cli_style::{CommandSpec, Example, OutputModes, Route};
use tmt_colab::{
    Result,
    assets::App,
    core,
    keyring::{Keyring, Layout},
    registration::Registration,
    socket::{MountSocket, Tunnels},
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
        details: "The space is reached through tmt remote, which mounts it for paired browsers. Serve the bundled browser app, or build it for local development. Local root-authorized management uses the same owner service as mounted browser requests.",
    };
    const SERVE: CommandSpec = CommandSpec {
        name: "serve",
        summary: "Serve a local space in the foreground",
        examples: &[Example {
            command: "tmt colab serve",
            note: "Serve the space on its owner-only socket for tmt remote",
        }],
        outputs: OutputModes::HumanAndJson,
        details: "Listens only on <data root>/colab/door.sock; open it from a browser paired with tmt remote pair while tmt remote serve runs.\nCtrl-C or SIGTERM closes the socket, its workers and tunnels.",
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
    const EXPORT: CommandSpec = CommandSpec {
        name: "export",
        summary: "Export a page as unencrypted HTML and a manifest",
        examples: &[Example {
            command: "tmt colab export 10000000-0000-4000-8000-000000000001 --json",
            note: "Create a private UUID-named export directory",
        }],
        outputs: OutputModes::HumanAndJson,
        details: "This creates an unencrypted copy of the page. Anyone with these files can read it.\nCreates page.html and manifest.json in a new UUID subdirectory of --dir (default: current directory). The parent must exist; aliases resolve to a canonical path. Created entries cannot be symlinks; parent traversal and overwrite are refused. Discussions are not included. Archived or deleted pages cannot be exported yet.",
    };
    cli_grammar::extend(
        tmt_cli_style::command(&ROOT)
            .bin_name("tmt colab")
            .version(env!("CARGO_PKG_VERSION"))
            .arg(tmt_cli_style::version_arg(ArgAction::Version))
            .subcommand_required(true)
            .subcommand(
                tmt_cli_style::command(&SERVE).arg(
                    Arg::new("app-dir")
                        .long("app-dir")
                        .value_name("DIRECTORY")
                        .value_parser(clap::value_parser!(std::path::PathBuf))
                        .help(
                            "Override embedded or checkout app bytes with an absolute build directory",
                        ),
                ),
            )
            .subcommand(tmt_cli_style::command(&SPACES))
            .subcommand(
                tmt_cli_style::command(&EXPORT)
                    .arg(Arg::new("page").required(true).value_parser(|value: &str| {
                        tmt_colab_model::values::generated_id(value)
                            .map(|_| value.to_owned())
                            .map_err(|error| error.to_string())
                    }))
                    .arg(
                        Arg::new("dir")
                            .long("dir")
                            .value_name("destination")
                            .value_parser(clap::value_parser!(std::path::PathBuf)),
                    ),
            ),
    )
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
        if command == "export" {
            return export(&root, args);
        }
        if command != "serve" {
            return cli_management::run(command, args, &root, json_output);
        }
        let app = App::selected(
            args.get_one::<std::path::PathBuf>("app-dir")
                .map(|path| path.as_path()),
        )?;
        let layout = Layout::open(&root)?;
        let _lock = layout.serve_lock()?;
        let keyring = Keyring::open(&layout)?;
        let mut output = tmt_cli_style::stream::stdout(json_output);
        let store = Store::open(&layout)?;
        let space_id = keyring.space_id.clone();
        let registration = Arc::new(Mutex::new(Registration::new(
            store,
            keyring,
            std::env::current_exe()?,
        )?));
        let socket = MountSocket::bind(&layout, &space_id, Tunnels::PRODUCT)?
            .with_registration(&layout, Arc::clone(&registration))?
            .with_app(app);
        if json_output {
            writeln!(
                output,
                "{}",
                json!({"spaceId":space_id,"socket":socket.path,"profile":"colab-sync-v1","state":"mounted"})
            )?;
        } else {
            let terminal = output.terminal();
            tmt_cli_style::detail::write(
                &mut output,
                terminal,
                "LOCAL SPACE",
                &[
                    ("space", space_id),
                    ("socket", socket.path.display().to_string()),
                    (
                        "open",
                        "run tmt remote serve, then open colab from a browser paired with tmt remote pair"
                            .into(),
                    ),
                ],
            )?;
        }
        output.flush()?;
        drop(output);
        let result = socket.run(&stop);
        let closed = Arc::try_unwrap(registration)
            .map_err(|_| "Registration worker retained.")?
            .into_inner()
            .map_err(|_| "Registration lock poisoned.")?
            .close();
        result?;
        closed
    })();
    for signal in signals {
        signal_hook::low_level::unregister(signal);
    }
    result
}
fn export(root: &std::path::Path, args: &clap::ArgMatches) -> Result<()> {
    use tmt_colab::export::{Bundle, DISCLOSURE, Fault};
    let layout = Layout::existing(root)?.ok_or(Fault::MissingState)?;
    let keyring = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    let mut decoder = tmt_colab::decoder::Decoder::new(std::env::current_exe()?)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis()
        .try_into()?;
    let bundle = Bundle::capture(
        &store,
        &keyring,
        args.get_one::<String>("page").expect("required page"),
        &mut decoder,
        now,
    );
    let closed = store.close();
    let bundle = bundle?;
    closed?;
    let json_output = args.get_flag("json");
    if !json_output {
        let mut warning = tmt_cli_style::stream::stderr();
        let terminal = warning.terminal();
        tmt_cli_style::detail::write(
            &mut warning,
            terminal,
            "PLAINTEXT EXPORT",
            &[("disclosure", DISCLOSURE.into())],
        )?;
    }
    let parent = args
        .get_one::<std::path::PathBuf>("dir")
        .cloned()
        .unwrap_or(std::env::current_dir()?);
    let published = bundle.publish(&parent)?;
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(output, "{}", serde_json::to_string(&published)?)?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            "PAGE EXPORTED",
            &[
                ("directory", published.directory.display().to_string()),
                ("files", "page.html, manifest.json".into()),
                ("discussions", "not included".into()),
            ],
        )?;
    }
    Ok(())
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
fn error_code(error: &(dyn std::error::Error + Send + Sync + 'static)) -> &'static str {
    error
        .downcast_ref::<tmt_colab::keyring::StateFault>()
        .map(|e| e.code())
        .or_else(|| {
            error
                .downcast_ref::<tmt_colab::socket::SocketFault>()
                .map(|e| e.code())
        })
        .or_else(|| {
            error
                .downcast_ref::<cli_management::ManagementFault>()
                .map(|e| e.code)
        })
        .or_else(|| {
            error
                .downcast_ref::<tmt_colab::export::Fault>()
                .map(|e| e.code())
        })
        .or_else(|| {
            error
                .downcast_ref::<tmt_colab::assets::AssetFault>()
                .map(|_| "COLAB_APP_UNAVAILABLE")
        })
        .unwrap_or_else(|| {
            if matches!(
                error.downcast_ref::<tmt_colab::store::Fault>(),
                Some(tmt_colab::store::Fault::UnsupportedSchema(_))
            ) {
                "COLAB_SCHEMA_UNSUPPORTED"
            } else {
                "COLAB_UNAVAILABLE"
            }
        })
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
            let help = matches!(e.kind(), clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion);
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
            let cli_failure = error.downcast_ref::<cli_management::ManagementFault>();
            let code = error_code(error.as_ref());
            if matches
                .subcommand()
                .is_some_and(|(_, m)| m.get_flag("json"))
            {
                let mut value = cli_failure
                    .map(|e| e.correlation.clone())
                    .unwrap_or_else(|| json!({}));
                value["error"] = json!({"code":code,"message":error.to_string()});
                if let Some(path) = error
                    .downcast_ref::<tmt_colab::export::Fault>()
                    .and_then(|fault| fault.partial_directory())
                {
                    value["error"]["partialDirectory"] = json!(path);
                }
                let _ = writeln!(tmt_cli_style::stream::stdout(true), "{}", value);
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
        assert!(
            tmt_cli_style::audit::list_spelling_report(&grammar(), &["tmt", "colab"]).is_empty()
        );
    }
}
