use clap::{Arg, ArgAction, Command};
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Write},
    process::ExitCode,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tmt_cli_style::{CommandSpec, Example, Interaction, Mode, OutputModes, Route};
use tmt_remote::{
    control::{self, Control},
    core::CoreClient,
    devices::{Devices, device_json},
    error::RemoteError,
    http::{Door, Handler},
    mount::Mounts,
    pages::Pages,
    pairing::{Pairing, Timing},
    routes::Routes,
    session::{self, DoorSessions},
    site::Site,
    state::{Layout, MachineKey},
    store::{Store, uuid_v4},
};
const ROOT: CommandSpec = CommandSpec {
    name: "remote",
    summary: "Optional local remote door (pilot)",
    examples: &[Example {
        command: "tmt remote serve",
        note: "Open a loopback door; all requests are refused",
    }],
    outputs: OutputModes::Human,
    details: "Sends are not implemented. Core never listens.",
};
const PAIR: CommandSpec = CommandSpec {
    name: "pair",
    summary: "Authorize one device on the running remote",
    examples: &[Example {
        command: "tmt remote pair",
        note: "Print a pairing link and code, then confirm the device here",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "The device opens the link or enters the code. Compare the four words on both sides, then confirm once.\nThe grant reaches all agents, sends directly and does not expire; revoke it to end it.\n--json streams one event per line and reads confirm or refuse from stdin.",
};
const DEVICES: CommandSpec = CommandSpec {
    name: "devices",
    summary: "List, revoke or rename paired devices",
    examples: &[
        Example {
            command: "tmt remote devices",
            note: "Show each device with its fingerprint words",
        },
        Example {
            command: "tmt remote devices revoke <client-id>",
            note: "End a device's access now",
        },
        Example {
            command: "tmt remote devices rename <client-id> <name>",
            note: "Change a device's display name",
        },
    ],
    outputs: OutputModes::HumanAndJson,
    details: "Revoking disables the grant before it reports success and ends the device's door session and open pages.\nWorks whether or not tmt remote serve is running.",
};
const REVOKE: CommandSpec = CommandSpec {
    name: "revoke",
    summary: "Revoke one paired device",
    examples: &[Example {
        command: "tmt remote devices revoke <client-id>",
        note: "The client ID is shown by tmt remote devices",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Revoking a revoked device reports it again. Pair again to trust the device anew.",
};
const RENAME: CommandSpec = CommandSpec {
    name: "rename",
    summary: "Rename one paired device",
    examples: &[Example {
        command: "tmt remote devices rename <client-id> <name>",
        note: "The client ID is shown by tmt remote devices",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Names are 1–64 nonblank UTF-8 bytes without controls. Authority stays the same.\nA changed name ends the old door session; the device silently reopens it.\nRepeating the same name preserves the revision. Revoked devices cannot be renamed.",
};
const SERVE: CommandSpec = CommandSpec {
    name: "serve",
    summary: "Run a foreground IPv4-loopback deny-all door",
    examples: &[Example {
        command: "tmt remote serve --json",
        note: "Print the bound descriptor for local testing",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Runs in the foreground until Ctrl-C or SIGTERM; there is no default deadline.\nOnly pairing is admitted; no core operation is forwarded.\nMounts colab under <prefix>/x/colab/ while its owner-only socket exists.",
};
fn grammar() -> Command {
    tmt_cli_style::command(&ROOT)
        .bin_name("tmt remote")
        .version(env!("CARGO_PKG_VERSION"))
        .arg(tmt_cli_style::version_arg(ArgAction::Version))
        .subcommand_required(true)
        .subcommand(
            tmt_cli_style::command(&SERVE).arg(
                Arg::new("port")
                    .long("port")
                    .default_value("0")
                    .value_parser(clap::value_parser!(u16))
                    .help("Loopback port; 0 selects an unused port"),
            ),
        )
        .subcommand(tmt_cli_style::command(&PAIR))
        .subcommand(
            tmt_cli_style::command(&DEVICES)
                .subcommand(
                    tmt_cli_style::command(&REVOKE).arg(
                        Arg::new("client-id")
                            .required(true)
                            .help("Device client ID from tmt remote devices"),
                    ),
                )
                .subcommand(
                    tmt_cli_style::command(&RENAME)
                        .arg(
                            Arg::new("client-id")
                                .required(true)
                                .help("Device client ID from tmt remote devices"),
                        )
                        .arg(
                            Arg::new("name")
                                .required(true)
                                .help("New device display name"),
                        ),
                ),
        )
}
fn run(matches: &clap::ArgMatches) -> Result<(), RemoteError> {
    let (name, arguments) = matches.subcommand().expect("required subcommand");
    if name == "pair" {
        return pair(arguments.get_flag("json"));
    }
    if name == "devices" {
        return devices(arguments);
    }
    let serve = arguments;
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
        let core = CoreClient::discover()?;
        let capabilities = core.capabilities(&stop)?;
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
            .filter(|n| *n > 0 && *n <= tmt_remote::limits::CORE_INPUT_BYTES as u64)
            .ok_or_else(|| {
                RemoteError::new(
                    "REMOTE_CORE_UNAVAILABLE",
                    "Core did not advertise a valid input bound.",
                )
            })? as usize;
        let root = core.storage_root(&stop)?;
        let layout = Layout::open(&root)?;
        let serving = layout.serve_lock()?;
        let machine_key = MachineKey::open(&layout)?;
        let mut store = Store::open(&serving)?;
        let machine = store.machine()?;
        let store = Arc::new(Mutex::new(store));
        let door = Door::bind(*serve.get_one::<u16>("port").unwrap())?;
        // Each run is a new window; grants survive it, sessions do not.
        let window_id = uuid_v4()?;
        let pairing = Arc::new(Pairing::new(
            machine.id.clone(),
            window_id.clone(),
            machine_key.public(),
            door.origin.clone(),
            Arc::clone(&store),
            Timing::CONTRACT,
        ));
        let sessions = Arc::new(DoorSessions::new(
            machine.id.clone(),
            window_id.clone(),
            door.origin.clone(),
            format!("{}/x/", machine.route_prefix),
            machine_key,
            Arc::clone(&store),
            session::IDLE,
        ));
        let routes = Routes::new(input_limit, machine.route_prefix.clone())?
            .with_pairing(Arc::clone(&pairing))
            .with_sessions(Arc::clone(&sessions));
        let address = format!("{}{}", door.origin, routes.prefix());
        let devices = Arc::new(Devices::new(store, Some(Arc::clone(&sessions))));
        let control = Control::start(
            &serving,
            pairing,
            Arc::clone(&devices),
            control::Door {
                origin: door.origin.clone(),
                prefix: machine.route_prefix.clone(),
            },
        )?;
        let site = Arc::new(Site {
            routes,
            mounts: Arc::new(Mounts::new(
                root,
                &door.origin,
                &machine.route_prefix,
                sessions,
            )),
            pages: Some(Pages::new(
                &door.origin,
                machine.id.clone(),
                window_id.clone(),
                &machine.route_prefix,
            )),
        });
        let events = devices.start_events(Arc::clone(&site.mounts))?;
        let json_output = serve.get_flag("json");
        let mut output = tmt_cli_style::stream::stdout(json_output);
        if json_output {
            writeln!(
                output,
                "{}",
                json!({"profile":"local-v1","binding":"loopback-http","state":"closed","address":address,"machineId":machine.id,"startupCoreCalls":2})
            )?;
        } else {
            let terminal = output.terminal();
            tmt_cli_style::message::warning(
                &mut output,
                terminal,
                &format!("Door bound at {address}; pair a device with tmt remote pair"),
                None,
            )?;
        }
        output.flush()?;
        drop(output);
        let result = door.run(&stop, site as Arc<dyn Handler>);
        // Stopping cancels any pending pairing before state is released.
        control.stop();
        drop(events);
        result
    })();
    for id in signals.into_iter().flatten() {
        signal_hook::low_level::unregister(id);
    }
    result
}
/// `tmt remote pair`: open the single offer on the running serve and relay
/// the owner's one confirmation. State is reached only through serve.
fn pair(json_output: bool) -> Result<(), RemoteError> {
    let interaction = Interaction::detect(json_output);
    if !json_output && interaction.prompt() != Mode::Interactive {
        return Err(RemoteError::new(
            "REMOTE_CONFIRMATION_REQUIRED",
            "Pairing needs a terminal for the owner's confirmation, or --json.",
        ));
    }
    let stop = AtomicBool::new(false);
    let root = CoreClient::discover()?.storage_root(&stop)?;
    let root = std::fs::canonicalize(&root).map_err(|_| not_running())?;
    let mut stream = control::connect(&root.join("remote"))?;
    stream.write_all(b"{\"op\":\"pair\"}\n")?;
    let mut events = BufReader::new(stream.try_clone()?);
    let answering = AtomicBool::new(false);
    let mut output = tmt_cli_style::stream::stdout(json_output);
    let mut paired = Err(RemoteError::new("REMOTE_PAIRING_ENDED", "Pairing ended."));
    loop {
        let mut line = String::new();
        if events.read_line(&mut line)? == 0 {
            break;
        }
        let event: serde_json::Value = serde_json::from_str(&line)
            .map_err(|_| RemoteError::new("REMOTE_IO", "Remote sent an invalid control line."))?;
        if let Some(error) = event.get("error") {
            return Err(RemoteError::new(
                error["code"].as_str().unwrap_or("REMOTE_IO"),
                error["message"].as_str().unwrap_or("Pairing failed."),
            ));
        }
        if json_output {
            writeln!(output, "{event}")?;
            output.flush()?;
        }
        match event["event"].as_str() {
            Some("offer") if !json_output => {
                let terminal = output.terminal();
                tmt_cli_style::detail::write(
                    &mut output,
                    terminal,
                    "PAIR A DEVICE",
                    &[
                        ("link", event["link"].as_str().unwrap_or("").into()),
                        ("code", event["code"].as_str().unwrap_or("").into()),
                        ("expires", "in 10 minutes".into()),
                    ],
                )?;
                output.flush()?;
            }
            Some("candidate") => {
                if !json_output {
                    let words: Vec<&str> = event["words"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(serde_json::Value::as_str)
                        .collect();
                    let terminal = output.terminal();
                    tmt_cli_style::detail::write(
                        &mut output,
                        terminal,
                        "DEVICE",
                        &[
                            ("kind", event["kind"].as_str().unwrap_or("").into()),
                            ("origin", event["origin"].as_str().unwrap_or("").into()),
                            ("name", event["name"].as_str().unwrap_or("").into()),
                            ("words", words.join(" ")),
                            ("grant", "all agents, direct sends, no expiry".into()),
                        ],
                    )?;
                    output.flush()?;
                }
                // One answer per offer, written back by its own thread so an
                // offer that ends first (expiry, refusal) is still reported.
                if !answering.swap(true, Ordering::AcqRel) {
                    let mut control = stream.try_clone()?;
                    std::thread::spawn(move || {
                        if !json_output {
                            let mut prompt = tmt_cli_style::stream::stderr();
                            let _ = write!(prompt, "Trust this device if the words match? [y/N] ");
                            let _ = prompt.flush();
                        }
                        let mut answer = String::new();
                        let confirmed = std::io::stdin().read_line(&mut answer).is_ok()
                            && matches!(answer.trim(), "y" | "yes" | "confirm");
                        let line: &[u8] = if confirmed {
                            b"{\"op\":\"confirm\"}\n"
                        } else {
                            b"{\"op\":\"refuse\"}\n"
                        };
                        let _ = control.write_all(line);
                    });
                }
            }
            Some("ended") => {
                let reason = event["reason"].as_str().unwrap_or("ended");
                paired = if reason == "paired" {
                    Ok(())
                } else {
                    Err(RemoteError::new(
                        "REMOTE_PAIRING_ENDED",
                        &format!("Pairing ended: {reason}."),
                    ))
                };
                break;
            }
            _ => {}
        }
    }
    if paired.is_ok() && !json_output {
        let terminal = output.terminal();
        tmt_cli_style::message::success(&mut output, terminal, "Paired the device")?;
    }
    paired
}
/// `tmt remote devices [revoke|rename ...]`, through the running serve or,
/// when none runs, directly under the serve lock.
fn devices(arguments: &clap::ArgMatches) -> Result<(), RemoteError> {
    let mutation = arguments.subcommand();
    let json_output =
        arguments.get_flag("json") || mutation.is_some_and(|(_, m)| m.get_flag("json"));
    let request = match mutation {
        Some(("rename", m)) => {
            json!({"op":"rename","clientId":m.get_one::<String>("client-id").unwrap(), "name":m.get_one::<String>("name").unwrap()})
        }
        Some(("revoke", m)) => {
            json!({"op":"revoke","clientId":m.get_one::<String>("client-id").unwrap()})
        }
        Some(_) => unreachable!("typed device grammar"),
        None => json!({"op":"devices"}),
    };
    let stop = AtomicBool::new(false);
    let root = CoreClient::discover()?.storage_root(&stop)?;
    let answer = match std::fs::canonicalize(&root)
        .map_err(|_| not_running())
        .and_then(|root| control::connect(&root.join("remote")))
    {
        Ok(mut stream) => {
            stream.write_all(format!("{request}\n").as_bytes())?;
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line)?;
            let answer: serde_json::Value = serde_json::from_str(&line).map_err(|_| {
                RemoteError::new("REMOTE_IO", "Remote sent an invalid control line.")
            })?;
            if let Some(error) = answer.get("error") {
                return Err(RemoteError::new(
                    error["code"].as_str().unwrap_or("REMOTE_IO"),
                    error["message"]
                        .as_str()
                        .unwrap_or("Device management failed."),
                ));
            }
            answer
        }
        Err(error) if error.code == "REMOTE_NOT_RUNNING" => {
            let serving = Layout::open(&root)?.serve_lock()?;
            let devices = Devices::new(Arc::new(Mutex::new(Store::open(&serving)?)), None);
            match mutation {
                Some(("rename", m)) => {
                    json!({"device": device_json(&devices.rename(m.get_one::<String>("client-id").unwrap(), m.get_one::<String>("name").unwrap())?)})
                }
                Some(("revoke", m)) => {
                    json!({"device": device_json(&devices.revoke(m.get_one::<String>("client-id").unwrap())?)})
                }
                Some(_) => unreachable!("typed device grammar"),
                None => {
                    json!({"devices": devices.list()?.iter().map(device_json).collect::<Vec<_>>()})
                }
            }
        }
        Err(error) => return Err(error),
    };
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(output, "{answer}")?;
        return Ok(());
    }
    let terminal = output.terminal();
    if let Some(device) = answer.get("device") {
        let name = device["name"].as_str().unwrap_or("");
        return Ok(tmt_cli_style::message::success(
            &mut output,
            terminal,
            &format!(
                "{} {name}",
                if request["op"] == "rename" {
                    "Renamed"
                } else {
                    "Revoked"
                }
            ),
        )?);
    }
    use tmt_cli_style::{
        list::Section,
        mark::Mark,
        table::{Cell, Column, Table},
    };
    let listed = answer["devices"].as_array().cloned().unwrap_or_default();
    let mut rows = Table::new(&[
        Column::Fixed,
        Column::Name,
        Column::Fixed,
        Column::Detail,
        Column::Fixed,
    ]);
    for device in &listed {
        let mark = if device["revoked"] == true {
            Mark::Offline
        } else {
            Mark::Running
        };
        let text = |key: &str| Cell::from(device[key].as_str().unwrap_or(""));
        rows.row([
            Cell::styled(mark.symbol(), mark.token()),
            text("name"),
            text("kind"),
            text("words"),
            text("clientId"),
        ]);
    }
    Section {
        title: "DEVICES",
        count: Some(listed.len()),
        rows,
        note: None,
        hint: None,
    }
    .write(&mut output, terminal)?;
    Ok(())
}
/// Whether any subcommand on the chain asked for `--json`.
fn wants_json(matches: &clap::ArgMatches) -> bool {
    matches.subcommand().is_some_and(|(_, m)| {
        m.try_get_one::<bool>("json").ok().flatten() == Some(&true) || wants_json(m)
    })
}
fn not_running() -> RemoteError {
    RemoteError::new(
        "REMOTE_NOT_RUNNING",
        "Remote is not running; start it with tmt remote serve.",
    )
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
            if wants_json(&matches) {
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
