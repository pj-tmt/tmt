mod activation_cli;
mod message_cli;
mod open_cli;
mod pairing_cli;
mod serve;
mod sync_cli;

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
    control::{self, StatusProjection},
    core::CoreClient,
    devices::Devices,
    error::RemoteError,
    firestore_limits, open, readiness, settings,
    state::Layout,
    store::Store,
};
const ROOT: CommandSpec = CommandSpec {
    name: "remote",
    summary: "Optional local remote door (pilot)",
    examples: &[Example {
        command: "tmt remote serve",
        note: "Open the owner-device loopback door",
    }],
    outputs: OutputModes::Human,
    details: "Paired devices dispatch through the public core API. Held sends require local approval. Core never listens.",
};
const STOP: CommandSpec = CommandSpec {
    name: "stop",
    summary: "Gracefully stop the running door and keep paired devices",
    examples: &[Example {
        command: "tmt remote stop --json",
        note: "Stop through the owner-only control socket and wait for cleanup",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Uses the same shutdown path as Ctrl-C/SIGTERM. Keeps pairings and grants.\nWaits at most 40 seconds for the lifecycle lease after acknowledgment; never signals a PID.",
};
const RESTART: CommandSpec = CommandSpec {
    name: "restart",
    summary: "Restart an idle background Remote at the same endpoint",
    examples: &[Example {
        command: "tmt remote restart --json --no-open",
        note: "Let Remote admit and perform one restart",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Foreground, legacy and busy services are left running. Uses the existing stop and startup bounds; never signals a discovered PID or retries an uncertain restart.",
};
const LINK: CommandSpec = CommandSpec {
    name: "link",
    summary: "Print a link to this remote's join guide",
    examples: &[Example {
        command: "tmt remote link --json",
        note: "Read the public link without opening a browser",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Share this link so someone can connect to this remote. It holds public connection details only and gives no access to shared pages or the project. Printing it opens no browser and changes nothing. The same link can be shared again.",
};
const STATUS: CommandSpec = CommandSpec {
    name: "status",
    summary: "Inspect the running door address without changing Remote state",
    examples: &[
        Example {
            command: "tmt remote status --json",
            note: "Discover the live origin and route path, or the last port when stopped",
        },
        Example {
            command: "tmt remote status --objects --json",
            note: "Observe Local object-channel readiness without starting a channel",
        },
        Example {
            command: "tmt remote status --layers",
            note: "Show which Firestore layers are enabled and what is missing; empty until a Firestore deployment exists",
        },
    ],
    outputs: OutputModes::HumanAndJson,
    details: "Read-only; does not start the door or pair a device. Live addresses come from serve.
Stopped status reports only the remembered port, which is not a live address.
--machine adds the running owner's machine UUID. --machine and --objects each require --json. --budget shows the dated Firestore free-plan limits, not the project's real usage. --machine, --objects, --layers and --budget cannot be combined; an older running door may not support these optional projections.",
};
const PAIR: CommandSpec = CommandSpec {
    name: "pair",
    summary: "Authorize one device on the running remote",
    examples: &[Example {
        command: "tmt remote pair",
        note: "Open the pairing link in your browser, then confirm the device here",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "The device opens the link or enters the code. Compare the four words on both sides, then confirm once.\nNew pairings are read only and do not expire; revoke them to end access.\n--talk allows this device to send to agents. --agents <uuid,...> and --hold require --talk and narrow its sending policy.\n--json streams one event per line and reads confirm or refuse from stdin.",
};
const SETTINGS: CommandSpec = CommandSpec {
    name: "settings",
    summary: "Show or change Remote browser settings",
    examples: &[Example {
        command: "tmt remote settings open off",
        note: "Print pairing links without opening the browser",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Browser opening defaults to on. sessions-per-device accepts a positive integer or off (default: off); changes apply at the next session open. Settings are stored only in Remote's data directory.",
};
const TALK: CommandSpec = CommandSpec {
    name: "talk",
    summary: "Enable or disable sending for one paired device",
    examples: &[Example {
        command: "tmt remote devices talk <person> on|off",
        note: "Keep its other grant policy unchanged",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Use a unique device name or a client ID. Only the local owner can run this command. A changed scope ends Sessions on the previous revision; revoked devices cannot be enabled.",
};
const DEVICES: CommandSpec = CommandSpec {
    name: "devices",
    summary: "List, remove or rename paired devices",
    examples: &[
        Example {
            command: "tmt remote devices",
            note: "Show each device with its fingerprint words",
        },
        Example {
            command: "tmt remote devices revoke <person>",
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
        command: "tmt remote devices revoke <person>",
        note: "Use a unique device name or the client ID shown by tmt remote devices",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Revoking a revoked device reports it again. Pair again to trust the device anew.\nTo revoke this machine's Firestore sign-in instead, use tmt remote sync revoke.",
};
const REMOVE_OTHERS: CommandSpec = CommandSpec {
    name: "remove-others",
    summary: "Remove all other paired devices and keep the selected device",
    details: "The kept device must be live. Removes at most 1000 other devices atomically; their saved pairing stops working.",
    examples: &[Example {
        command: "tmt remote devices remove-others --keep <client-id>",
        note: "Keep this paired device and end the others' access",
    }],
    outputs: OutputModes::HumanAndJson,
};
const RENAME: CommandSpec = CommandSpec {
    name: "rename",
    summary: "Rename one paired device",
    examples: &[Example {
        command: "tmt remote devices rename <client-id> <name>",
        note: "The client ID is shown by tmt remote devices",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Names are 1–64 nonblank UTF-8 bytes without controls. Authority stays the same.\nRenaming keeps this device's access, its open sessions and any held work.\nRenaming to the current name changes nothing. Revoked devices cannot be renamed.",
};
const DESIGNATE: CommandSpec = CommandSpec {
    name: "designate",
    summary: "Authorize one paired browser to manage Remote settings and devices",
    examples: &[Example {
        command: "tmt remote devices designate <client-id>",
        note: "Select a live browser UUID shown by devices",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Local-only. No browser is designated automatically. Replaces the previous designation; agent grants stay unchanged.",
};
const UNDESIGNATE: CommandSpec = CommandSpec {
    name: "undesignate",
    summary: "Remove browser authority to manage Remote settings and devices",
    examples: &[Example {
        command: "tmt remote devices undesignate",
        note: "Keep every browser read-only",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Local-only. Keeps pairings and existing agent grants; fences later management effects.",
};
const SERVE: CommandSpec = CommandSpec {
    name: "serve",
    summary: "Start the IPv4-loopback owner-device door",
    examples: &[Example {
        command: "tmt remote serve --json",
        note: "Print the bound descriptor for local testing",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Human output starts in the background; use status and stop to inspect and end it.\n--foreground owns the door until Ctrl-C or SIGTERM. Bare --json remains foreground; use --background --json to detach.\nSigned direct sends reach core; held sends wait for local approval.\nMounts colab under <prefix>/x/colab/ while its owner-only socket exists.\nWithout --port, reuse the last bound port; if busy, refuse until freed or explicitly overridden. --port 0 selects a random unused port.",
};
const APPROVE: CommandSpec = CommandSpec {
    name: "approve",
    summary: "Confirm one frozen held operation locally",
    examples: &[Example {
        command: "tmt remote approve <operation-id>",
        note: "Inspect the exact frozen message, then confirm once",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Only the local owner can approve. --json emits held/ended events and reads one confirm/refuse JSON line from stdin.",
};
const CANCEL: CommandSpec = CommandSpec {
    name: "cancel",
    summary: "Cancel one held operation locally",
    examples: &[Example {
        command: "tmt remote cancel <operation-id>",
        note: "Cancel without creating a core request",
    }],
    outputs: OutputModes::HumanAndJson,
    details: "Cancellation cannot undo a dispatch that already started.",
};
fn grammar() -> Command {
    tmt_cli_style::command(&ROOT)
        .bin_name("tmt remote")
        .version(env!("CARGO_PKG_VERSION"))
        .arg(tmt_cli_style::version_arg(ArgAction::Version))
        .subcommand_required(true)
        .subcommand(activation_cli::command())
        .subcommand(message_cli::command())
        .subcommand(pairing_cli::command(false))
        .subcommand(pairing_cli::command(true))
        .subcommand(open_cli::command(false))
        .subcommand(open_cli::command(true))
        .arg(Arg::new("remote").long("remote").global(true).help("Remote name, owned project id, or local"))
        .subcommand(sync_cli::command())
        .subcommand(
            tmt_cli_style::command(&SERVE)
                .arg(
                    Arg::new("foreground")
                        .long("foreground")
                        .action(ArgAction::SetTrue)
                        .conflicts_with("background")
                        .help("Keep the door owned by this command"),
                )
                .arg(
                    Arg::new("background")
                        .long("background")
                        .action(ArgAction::SetTrue)
                        .help("Detach after the private readiness handoff"),
                )
                .arg(
                    Arg::new("worker")
                        .long("worker")
                        .hide(true)
                        .action(ArgAction::SetTrue)
                        .requires("foreground"),
                )
                .arg(
                    Arg::new("port")
                        .long("port")
                        .value_parser(clap::value_parser!(u16))
                        .help(
                            "Loopback port; omitted reuses the last port, 0 selects an unused port",
                        ),
                ),
        )
        .subcommand(tmt_cli_style::command(&tmt_cli_style::CommandSpec {
            name: "deploy", summary: "Plan and authorize cloud sharing deployment", examples: &[Example { command: "tmt remote deploy firestore --help", note: "Inspect the Firestore deployment inputs and authorization" }],
            outputs: OutputModes::Human, details: "Only Firestore sharing is supported. Deployment requires explicit exact-plan authorization.",
        }).subcommand_required(true).subcommand(tmt_remote::deploy_cli::command()))
        .subcommand(tmt_cli_style::command(&CommandSpec { name: "ls", summary: "List remotes", examples: &[Example { command: "tmt remote ls --json", note: "List names, projects and regions" }], outputs: OutputModes::HumanAndJson, details: "The default column marks only the configured default. This is not live provider status." }))
        .subcommand(tmt_cli_style::command(&CommandSpec { name: "add", summary: "Remember a remote on this machine", examples: &[Example { command: "tmt remote add team '<remote-link>' --json", note: "Remember the remote as team" }], outputs: OutputModes::HumanAndJson, details: "Save the remote's public connection details under a name you choose. This does not sign in, grant page access or change the project." }).arg(Arg::new("name").required(true)).arg(Arg::new("url").required(true)))
        .subcommand(tmt_cli_style::command(&CommandSpec { name: "rm", summary: "Remove only a remote reference", examples: &[Example { command: "tmt remote rm test --json", note: "Keep deploy history and cloud resources" }], outputs: OutputModes::HumanAndJson, details: "Removing the default leaves it pointing to a removed remote until you change default-remote." }).arg(Arg::new("name").required(true)))
        .subcommand(tmt_cli_style::command(&LINK).arg(Arg::new("project").long("project").help("Project deployed from this machine")))
        .subcommand(tmt_cli_style::command(&STOP))
        .subcommand(tmt_cli_style::command(&RESTART).arg(Arg::new("no-open").long("no-open").action(ArgAction::SetTrue).help("Keep browser opening suppressed")))
        .subcommand(
            tmt_cli_style::command(&STATUS)
                .arg(
                    Arg::new("machine")
                        .long("machine")
                        .action(ArgAction::SetTrue)
                        .requires("json")
                        .help("Include the live machine UUID (requires --json)"),
                )
                .arg(
                    Arg::new("objects")
                        .long("objects")
                        .action(ArgAction::SetTrue)
                        .requires("json")
                        .conflicts_with("machine")
                        .help("Include live Local object-channel readiness (requires --json)"),
                )
                .arg(
                    Arg::new("layers")
                        .long("layers")
                        .action(ArgAction::SetTrue)
                        .conflicts_with_all(["machine", "objects"])
                        .help("Show which Firestore layers are enabled and what is missing"),
                )
                .arg(
                    Arg::new("project").long("project").conflicts_with_all(["objects", "machine", "budget"]).help("Assert the selected Firebase project id"),
                )
                .arg(
                    Arg::new("budget")
                        .long("budget")
                        .action(ArgAction::SetTrue)
                        .conflicts_with_all(["machine", "objects", "layers"])
                        .help("Show the dated Firestore free-plan limits (not real usage)"),
                ),
        )
        .subcommand(
            tmt_cli_style::command(&PAIR)
                .arg(
                    Arg::new("talk")
                        .long("talk")
                        .action(ArgAction::SetTrue)
                        .help("Allow this device to send to agents"),
                )
                .arg(
                    Arg::new("agents")
                        .long("agents")
                        .value_delimiter(',')
                        .num_args(1..)
                        .requires("talk")
                        .help("Limit sending to these agent UUIDs"),
                )
                .arg(
                    Arg::new("hold")
                        .long("hold")
                        .action(ArgAction::SetTrue)
                        .requires("talk")
                        .help("Hold sending for local approval"),
                )
                .arg(
                    Arg::new("open")
                        .long("open")
                        .action(ArgAction::SetTrue)
                        .conflicts_with("no-open")
                        .help(
                            "Open the browser when the environment permits, overriding the setting",
                        ),
                )
                .arg(
                    Arg::new("no-open")
                        .long("no-open")
                        .action(ArgAction::SetTrue)
                        .help("Print the pairing link without opening the browser"),
                ),
        )
        .subcommand(
            tmt_cli_style::command(&SETTINGS)
                .arg(
                    Arg::new("key")
                        .value_parser(["open", "sessions-per-device", "default-remote"])
                        .requires("value"),
                )
                .arg(Arg::new("value").requires("key")),
        )
        .subcommand(tmt_cli_style::command(&APPROVE).arg(Arg::new("operation-id").required(true)))
        .subcommand(tmt_cli_style::command(&CANCEL).arg(Arg::new("operation-id").required(true)))
        .subcommand(
            tmt_cli_style::command(&DEVICES)
                .subcommand(
                    tmt_cli_style::command(&TALK)
                        .arg(Arg::new("client-id").value_name("PERSON").required(true).help("Unique device name or client ID"))
                        .arg(
                            Arg::new("enabled")
                                .required(true)
                                .value_parser(["on", "off"]),
                        ),
                )
                .subcommand(
                    tmt_cli_style::command(&DESIGNATE).arg(Arg::new("client-id").required(true)),
                )
                .subcommand(tmt_cli_style::command(&UNDESIGNATE))
                .subcommand(tmt_cli_style::command(&REMOVE_OTHERS).arg(Arg::new("keep").long("keep").required(true).help("Client ID of the paired device to keep")))
                .subcommand(
                    tmt_cli_style::command(&REVOKE).arg(
                        Arg::new("client-id")
                            .value_name("PERSON")
                            .required(true)
                            .help("Unique device name or client ID from tmt remote devices"),
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
    let explicit = matches.get_one::<String>("remote").map(String::as_str);
    if !(matches!(
        name,
        "link" | "deploy" | "sync" | "message" | "invite" | "join"
    ) || name == "status"
        && !arguments.get_flag("objects")
        && !arguments.get_flag("machine")
        && !arguments.get_flag("budget"))
        && explicit.is_some()
    {
        let root = discovery_root()?;
        let layout = Layout::open(&root)?;
        tmt_remote::remote_selector::local(&layout, explicit, name)?;
    }
    if name == "invite" || name == "join" {
        return pairing_cli::run(arguments, name == "join", explicit);
    }
    if name == "message" {
        return message_cli::run(explicit);
    }
    if matches!(name, "add" | "ls" | "rm") {
        return remotes_command(name, arguments);
    }
    if name == "sync" {
        let layout = Layout::open(&discovery_root()?)?;
        return sync_cli::run(&layout, explicit, arguments);
    }
    if matches!(name, "open" | "launch") {
        return open_cli::run(arguments, name == "launch");
    }
    if name == "link" {
        return link_command(
            arguments.get_flag("json"),
            explicit,
            arguments.get_one::<String>("project").map(String::as_str),
        );
    }
    if name == "deploy" {
        return deploy_firestore_command(arguments);
    }
    if matches!(name, "approve" | "cancel") {
        return approval_command(name, arguments);
    }
    if name == "pair" {
        let mut agents = arguments
            .get_many::<String>("agents")
            .map(|values| values.cloned().collect::<Vec<_>>());
        if let Some(ids) = &mut agents {
            ids.sort();
        }
        let policy = tmt_remote::pairing::PairingPolicy {
            talk: arguments.get_flag("talk"),
            agents,
            hold: arguments.get_flag("hold"),
        };
        policy.validate()?;
        return pair(arguments.get_flag("json"), open::flag(arguments), policy);
    }
    if name == "status" {
        return status(
            arguments.get_flag("json"),
            if arguments.get_flag("budget") {
                StatusProjection::Budget
            } else if arguments.get_flag("layers") {
                StatusProjection::Layers
            } else if arguments.get_flag("objects") {
                StatusProjection::Objects
            } else if arguments.get_flag("machine") {
                StatusProjection::Machine
            } else {
                StatusProjection::Ordinary
            },
            explicit,
            arguments.get_one::<String>("project").map(String::as_str),
        );
    }
    if name == "stop" {
        return stop_command(arguments.get_flag("json"));
    }
    if name == "restart" {
        return serve::restart(arguments.get_flag("json"));
    }
    if name == "settings" {
        return settings_command(arguments);
    }
    if name == "devices" {
        return devices(arguments);
    }
    serve::run(arguments)
}
fn remotes_command(action: &str, words: &clap::ArgMatches) -> Result<(), RemoteError> {
    let root = discovery_root()?;
    let layout = Layout::open(&root)?;
    let json_output = words.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if action == "add" {
        let entry = tmt_remote::remote_catalog::add(
            &layout,
            words.get_one::<String>("name").unwrap(),
            words.get_one::<String>("url").unwrap(),
        )?;
        if json_output {
            writeln!(output, "{}", json!({"added":true}))?;
        } else {
            let name = words.get_one::<String>("name").unwrap();
            if let Some(project) = entry.project {
                let region = tmt_remote::deploy_record::metadata(&layout, &project)?
                    .and_then(|identity| identity["region"].as_str().map(str::to_owned));
                writeln!(
                    output,
                    "Added remote '{name}' ({project}, {})",
                    region.as_deref().unwrap_or("-")
                )?;
            } else {
                writeln!(output, "Added remote '{name}' (local)")?;
            }
        }
        return Ok(());
    } else if action == "rm" {
        tmt_remote::remote_catalog::remove(&layout, words.get_one::<String>("name").unwrap())?;
        if json_output {
            writeln!(
                output,
                "{}",
                json!({"removed":true,"cloudResourcesChanged":false,"deployHistoryChanged":false})
            )?;
        } else {
            writeln!(
                output,
                "Removed remote '{}'; its project and this machine's deploy records are untouched",
                words.get_one::<String>("name").unwrap()
            )?;
        }
        return Ok(());
    }
    let mut entries = tmt_remote::remote_catalog::read(&layout)?;
    for project in tmt_remote::deploy_record::projects(&layout)? {
        if !entries
            .iter()
            .any(|entry| entry.project.as_deref() == Some(&project))
        {
            entries.push(tmt_remote::remote_catalog::RemoteReference {
                name: project.clone(),
                kind: "firestore".into(),
                url: String::new(),
                project: Some(project),
            });
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let settings = settings::read(&root)?;
    let mut rows = vec![];
    for entry in entries {
        let region = entry
            .project
            .as_deref()
            .map(|p| tmt_remote::deploy_record::metadata(&layout, p))
            .transpose()?
            .flatten()
            .map(|m| m["region"].clone())
            .unwrap_or(json!(null));
        rows.push(json!({"name":entry.name,"kind":entry.kind,"project":entry.project,"region":region,"default":settings.default_remote() == Some(entry.name.as_str())}));
    }
    if json_output {
        writeln!(output, "{}", json!({"version":1,"remotes":rows}))?;
    } else {
        if rows.is_empty() {
            writeln!(
                output,
                "No remotes yet; add one with tmt remote add <name> <link>"
            )?;
        } else {
            use tmt_cli_style::table::{Column, Table};
            let mut table =
                Table::new(&[Column::Name, Column::Detail, Column::Fixed, Column::Fixed]);
            table.row(["REMOTE", "TARGET", "REGION", "DEFAULT"]);
            for row in rows {
                table.row([
                    row["name"].as_str().unwrap(),
                    row["project"].as_str().unwrap_or("local"),
                    row["region"].as_str().unwrap_or("-"),
                    if row["default"] == true {
                        "default"
                    } else {
                        ""
                    },
                ]);
            }
            let terminal = output.terminal();
            table.write(&mut output, terminal)?;
        }
    }
    Ok(())
}
fn link_command(
    json_output: bool,
    explicit: Option<&str>,
    selected_project: Option<&str>,
) -> Result<(), RemoteError> {
    let root = discovery_root()?;
    let layout = Layout::existing(&root)?.ok_or_else(tmt_remote::remote_link::unavailable)?;
    let project = tmt_remote::remote_selector::existing(&layout, explicit, selected_project)?
        .ok_or_else(tmt_remote::remote_link::unavailable)?;
    let record = tmt_remote::deploy_record::read_project(&layout, &project)?
        .ok_or_else(tmt_remote::remote_link::unavailable)?;
    let link = tmt_remote::remote_link::from_record(&record)?;
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(output, "{}", json!({"version":1,"remoteLink":link}))?;
    } else {
        writeln!(output, "Remote link: {link}")?;
    }
    Ok(())
}
fn deploy_firestore_command(matches: &clap::ArgMatches) -> Result<(), RemoteError> {
    use tmt_remote::{deploy_cli, deploy_discovery, deploy_firestore, deploy_tools};
    let (_, matches) = matches.subcommand().expect("required backend");
    // Grammar/authorization shape is checked before tool discovery or provider setup.
    let args = deploy_cli::arguments(matches).map_err(|_| RemoteError::new("USAGE_ERROR", "Use --project, --region and --sign-in; --authorize needs the first 12 or more lowercase hex characters of the plan digest."))?;
    let stop = Arc::new(AtomicBool::new(false));
    let _signals = [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM]
        .into_iter()
        .map(|signal| signal_hook::flag::register(signal, stop.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut declarations = deploy_discovery::InstalledDeclarations::new(&stop).map_err(|_| {
        RemoteError::new(
            "REMOTE_DEPLOY_DECLARATION_UNAVAILABLE",
            "Could not read what the installed extensions need from Firestore. Nothing changed in your Firebase project.",
        )
    })?;
    let enabled: Vec<_> = tmt_remote::mount::EXTENSIONS
        .iter()
        .map(|e| e.name)
        .collect();
    let mut confirm = |preview: &tmt_remote::deploy_command::DeployCommandOutput| {
        let mut output = tmt_cli_style::stream::stderr();
        write!(output, "{}", preview.human)
            .map_err(|e| deploy_cli::DeployCliError::Local(e.into()))?;
        output
            .flush()
            .map_err(|e| deploy_cli::DeployCliError::Local(e.into()))?;
        confirm_deploy(&mut std::io::stdin(), &mut output, &stop)
            .map_err(|e| deploy_cli::DeployCliError::Local(e.into()))
    };
    let interaction = Interaction::detect(args.json);
    let agent = std::env::var_os("TMT_AGENT").is_some_and(|value| !value.is_empty());
    let ci = std::env::var_os("CI").is_some();
    let live = deploy_cli::progress::enabled(interaction, agent, ci);
    let progress = deploy_cli::progress::Progress::stderr(live, stop.clone());
    let reporter = progress.reporter();
    let confirmation = deploy_cli::may_prompt(interaction, agent, ci)
        .then_some(&mut confirm as &mut deploy_cli::PlanConfirmation<'_>);
    let result = deploy_cli::execute_with_confirmation(
        &args,
        &mut declarations,
        &enabled,
        || {
            let search = std::env::var_os("PATH").unwrap_or_default();
            let tools =
                deploy_tools::discover(&search).map_err(deploy_cli::DeployCliError::Tool)?;
            deploy_firestore::DeployFirestore::at(tools.node, tools.package, &stop)
                .map(|provider| {
                    if live {
                        provider.with_progress(move |plan, event| reporter.notify(plan, event))
                    } else {
                        // Preserve the pre-existing plain/JSON index-round output.
                        provider.with_composite_progress(|elapsed, ceiling| {
                            let mut output = tmt_cli_style::stream::stderr();
                            let _ = writeln!(
                                output,
                                "{}",
                                deploy_cli::composite_progress(elapsed, ceiling)
                            );
                        })
                    }
                })
                .map_err(deploy_cli::DeployCliError::Setup)
        },
        || {
            let root = CoreClient::discover()
                .and_then(|core| core.storage_root(&stop))
                .map_err(deploy_cli::DeployCliError::Local)?;
            Layout::open(&root).map_err(deploy_cli::DeployCliError::Local)
        },
        || tmt_remote::pairing::now_ms().map_err(deploy_cli::DeployCliError::Local),
        confirmation,
    );
    // Stop, join and clear before either the final result or any propagated error.
    drop(progress);
    let result = result.map_err(|error| match error {
        deploy_cli::DeployCliError::Tool(deploy_tools::ToolDiscoveryError::FirebaseMissing) => RemoteError::new("REMOTE_DEPLOY_TOOL_MISSING", "Firebase CLI is not installed. Install it with: npm install -g firebase-tools@15.29.0"),
        deploy_cli::DeployCliError::Tool(deploy_tools::ToolDiscoveryError::NodeMissing) => RemoteError::new("REMOTE_DEPLOY_NODE_MISSING", "Node.js could not be found for the installed Firebase CLI. Install Node.js, then try again."),
        deploy_cli::DeployCliError::Tool(deploy_tools::ToolDiscoveryError::Unsupported) => RemoteError::new("REMOTE_DEPLOY_TOOL_UNSUPPORTED", &deploy_firestore::DeploySetupError::UnsupportedTool.to_string()),
        deploy_cli::DeployCliError::Setup(error) => RemoteError::new("REMOTE_DEPLOY_SETUP_UNAVAILABLE", &error.to_string()),
        deploy_cli::DeployCliError::Local(error) => error,
        deploy_cli::DeployCliError::Discovery(_) => RemoteError::new("REMOTE_DEPLOY_DECLARATION_UNAVAILABLE", "Could not read what the installed extensions need from Firestore. Nothing changed in your Firebase project."),
        deploy_cli::DeployCliError::Provider(error) => error.remote_error(),
        deploy_cli::DeployCliError::Command(error, path) => deploy_cli::command_error(error, &path),
        deploy_cli::DeployCliError::Usage => RemoteError::new("USAGE_ERROR", "Use --project, --region and --sign-in; --authorize needs the first 12 or more lowercase hex characters of the plan digest."),
    })?;
    let mut output = tmt_cli_style::stream::stdout(args.json);
    if args.json {
        writeln!(output, "{}", result.json)?;
    } else {
        write!(output, "{}", result.human)?;
    }
    Ok(())
}
/// One bounded line from the terminal; signals cancel the wait without a reader thread.
fn confirm_deploy(
    input: &mut (impl std::io::Read + std::os::fd::AsFd),
    output: &mut impl Write,
    stop: &AtomicBool,
) -> std::io::Result<bool> {
    confirm_terminal(input, output, stop, tmt_remote::deploy_cli::DEPLOY_PROMPT)
}
fn confirm_terminal(
    input: &mut (impl std::io::Read + std::os::fd::AsFd),
    output: &mut impl Write,
    stop: &AtomicBool,
    prompt: &str,
) -> std::io::Result<bool> {
    use nix::{
        errno::Errno,
        poll::{PollFd, PollFlags, poll},
    };
    write!(output, "{prompt} ")?;
    output.flush()?;
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let mut descriptors = [PollFd::new(input.as_fd(), PollFlags::POLLIN)];
        match poll(&mut descriptors, tmt_remote::limits::DEPLOY_CONFIRM_POLL_MS) {
            Ok(0) | Err(Errno::EINTR) => continue,
            Err(error) => return Err(std::io::Error::from_raw_os_error(error as i32)),
            Ok(_) => {}
        }
        let mut bytes = [0; tmt_remote::limits::DEPLOY_CONFIRM_BYTES];
        let count = input.read(&mut bytes)?;
        return Ok(!stop.load(Ordering::SeqCst)
            && count < bytes.len()
            && tmt_remote::deploy_cli::confirmation_answer(&bytes[..count]));
    }
}
fn discovery_root() -> Result<std::path::PathBuf, RemoteError> {
    CoreClient::discover()?
        .storage_root(&AtomicBool::new(false))
        .map_err(|error| {
            if error.code.starts_with("REMOTE_") {
                error
            } else {
                RemoteError::new(
                    "REMOTE_CORE_UNAVAILABLE",
                    &format!("Core storage.root failed: {error}"),
                )
            }
        })
}
fn stop_command(json_output: bool) -> Result<(), RemoteError> {
    let root = discovery_root()?;
    let stopped = match Layout::existing(&root)? {
        None => false,
        Some(layout) => {
            if control::request_stop(&layout.directory)? {
                let _lease = layout
                    .wait_for_release(std::time::Instant::now() + tmt_remote::limits::STOP_WAIT)?;
                match std::fs::symlink_metadata(layout.directory.join(control::SOCKET)) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Ok(_) => {
                        return Err(RemoteError::new(
                            "REMOTE_STOP_UNCONFIRMED",
                            "Shutdown was requested, but the control socket remains; a new serve may have started.",
                        ));
                    }
                    Err(error) => return Err(error.into()),
                }
                true
            } else {
                // No socket is not enough when a foreground or its invocation
                // child still retains the lease. Refuse rather than guess a PID.
                if let Some(lease) = layout.existing_serve_lock()? {
                    // Use the same read-only state admission as stopped status.
                    Store::stopped_port(&lease)?;
                }
                false
            }
        }
    };
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(
            output,
            "{}",
            if stopped {
                json!({"stopped":true})
            } else {
                json!({"running":false})
            }
        )?;
    } else {
        let terminal = output.terminal();
        if stopped {
            tmt_cli_style::message::success(
                &mut output,
                terminal,
                "Stopped Remote; paired devices are kept",
            )?;
        } else {
            tmt_cli_style::message::warning(&mut output, terminal, "Remote is not running", None)?;
        }
    }
    Ok(())
}
/// Public discovery for local extensions. Only the control socket supplies
/// live values; stopped reads hold the same lease as every database opener.
fn status(
    json_output: bool,
    projection: StatusProjection,
    explicit: Option<&str>,
    selected_project: Option<&str>,
) -> Result<(), RemoteError> {
    let root = discovery_root()?;
    let door_projection = if projection == StatusProjection::Layers {
        StatusProjection::Ordinary
    } else {
        projection
    };
    let mut answer = match Layout::existing(&root)? {
        None => json!({"running":false,"lastPort":null}),
        Some(layout) => match control::status_projection(&layout.directory, door_projection) {
            Ok(Some(answer)) => answer,
            Ok(None) => {
                let last_port = match layout.existing_serve_lock()? {
                    Some(serving) => Store::stopped_port(&serving)?,
                    None => None,
                };
                json!({"running":false,"lastPort":last_port})
            }
            Err(error) => return Err(error),
        },
    };
    if projection == StatusProjection::Ordinary
        && answer["running"] == true
        && let Ok(Some(cloud)) = control::cloud_status(&root.join("remote"))
        && (cloud["cleanupPending"] == true || !cloud["error"].is_null())
    {
        answer["cloud"] = cloud;
    }
    let mut recorded_summary = None;
    let sign_in = if projection == StatusProjection::Ordinary {
        let layout = Layout::existing(&root)?;
        Some(sync_cli::status(
            layout.as_ref(),
            explicit,
            selected_project,
        )?)
    } else {
        None
    };
    if let Some(sign_in) = &sign_in {
        answer["signIn"] = sign_in.value.clone();
        if let Some(notice) = sign_in.notice {
            answer["notices"] = json!([notice]);
        }
    }
    if projection == StatusProjection::Layers {
        let layout = Layout::existing(&root)?;
        let evidence = match layout {
            None => None,
            Some(layout) => {
                let project =
                    tmt_remote::remote_selector::existing(&layout, explicit, selected_project)?;
                if let Some(project) = project {
                    match tmt_remote::deploy_record::snapshot(&layout, &project) {
                        Ok(Some(snapshot)) => {
                            recorded_summary = Some(match snapshot.record.run.as_ref() {
                                None => "Firestore setup has not started",
                                Some(run)
                                    if run.state != tmt_remote::deploy_run::RunState::Complete =>
                                {
                                    "Firestore setup is not finished"
                                }
                                Some(_) => "",
                            });
                            if let Some(identity) = snapshot.identity() {
                                answer["firestoreDeployment"] = identity;
                            }
                            readiness::from_record(&snapshot.record)
                        }
                        Ok(None) => None,
                        Err(error) if error.code == "REMOTE_SELECTOR_CONFLICT" => {
                            return Err(error);
                        }
                        Err(_) => Some(readiness::FirestoreEvidence::unknown()),
                    }
                } else {
                    None
                }
            }
        };
        answer["firestoreLayers"] = readiness::project(evidence.as_ref(), &readiness::LAYERS);
    }
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(output, "{answer}")?;
    } else {
        let terminal = output.terminal();
        if answer["running"] == true {
            tmt_cli_style::message::success(&mut output, terminal, "Remote is running")?;
            writeln!(
                output,
                "{}{}",
                answer["origin"].as_str().unwrap(),
                answer["path"].as_str().unwrap()
            )?;
            if projection == StatusProjection::Budget {
                for line in firestore_limits::human_lines(&answer["firestoreBudget"]) {
                    writeln!(output, "{line}")?;
                }
            }
        } else {
            let detail = answer["lastPort"]
                .as_u64()
                .map(|port| format!("Remote is not running; last bound port {port}"))
                .unwrap_or_else(|| "Remote is not running".into());
            tmt_cli_style::message::warning(&mut output, terminal, &detail, None)?;
        }
        if answer["cloud"]["cleanupPending"] == true {
            writeln!(output, "Cloud cleanup is pending.")?;
        }
        if let Some(code) = answer["cloud"]["error"].as_str() {
            writeln!(output, "Cloud transport: {code}")?;
        }
        if projection == StatusProjection::Layers {
            if let Some(selected) = answer.get("firestoreDeployment") {
                writeln!(
                    output,
                    "Recorded Firestore setup: {} ({})",
                    selected["project"].as_str().unwrap(),
                    selected["region"].as_str().unwrap()
                )?;
            }
            if let Some(summary) = recorded_summary.filter(|summary| !summary.is_empty()) {
                tmt_cli_style::message::warning(&mut output, terminal, summary, None)?;
            }
            let lines = readiness::human_lines(&answer["firestoreLayers"], &readiness::LAYERS);
            for line in lines.into_iter().filter(|_| {
                recorded_summary.is_none()
                    || answer["firestoreLayers"]
                        .as_array()
                        .is_some_and(|rows| !rows.is_empty())
            }) {
                if line.enabled {
                    tmt_cli_style::message::success(&mut output, terminal, &line.what)?;
                } else {
                    tmt_cli_style::message::warning(
                        &mut output,
                        terminal,
                        &line.what,
                        line.hint.as_deref(),
                    )?;
                }
            }
        }
        if let Some(sign_in) = sign_in {
            if let Some(line) = sign_in.line {
                writeln!(output, "{line}")?;
            }
            if let Some(notice) = sign_in.notice {
                writeln!(output, "{notice}")?;
            }
        }
    }
    Ok(())
}
fn approval_command(action: &str, arguments: &clap::ArgMatches) -> Result<(), RemoteError> {
    let json_output = arguments.get_flag("json");
    if action == "approve"
        && !json_output
        && Interaction::detect(false).prompt() != Mode::Interactive
    {
        return Err(RemoteError::new(
            "REMOTE_CONFIRMATION_REQUIRED",
            "Approval needs a terminal, or --json confirmation.",
        ));
    }
    let id = arguments.get_one::<String>("operation-id").unwrap();
    if !tmt_remote::canonical::is_core_id(id) {
        return Err(RemoteError::new(
            "REMOTE_INPUT_INVALID",
            "Invalid operation ID.",
        ));
    }
    let core = CoreClient::discover()?;
    let root = core.storage_root(&AtomicBool::new(false))?;
    let mut stream = control::connect(&root.join("remote"))?;
    writeln!(stream, "{}", json!({"op":action,"operationId":id}))?;
    let mut events = BufReader::new(stream.try_clone()?);
    let mut output = tmt_cli_style::stream::stdout(json_output);
    loop {
        let mut line = String::new();
        if events.read_line(&mut line)? == 0 {
            return Err(RemoteError::new("REMOTE_IO", "Approval connection ended."));
        }
        let event: serde_json::Value = serde_json::from_str(&line)
            .map_err(|_| RemoteError::new("REMOTE_IO", "Invalid approval response."))?;
        if let Some(error) = event.get("error") {
            return Err(RemoteError::new(
                error["code"].as_str().unwrap_or("REMOTE_IO"),
                error["message"].as_str().unwrap_or("Approval refused."),
            ));
        }
        if json_output {
            writeln!(output, "{event}")?;
            output.flush()?;
        }
        if event["event"] != "held" {
            if !json_output {
                let terminal = output.terminal();
                tmt_cli_style::detail::write(
                    &mut output,
                    terminal,
                    "OPERATION",
                    &[
                        (
                            "state",
                            event["state"].as_str().unwrap_or("uncertain").into(),
                        ),
                        ("operation", id.clone()),
                    ],
                )?;
            }
            return Ok(());
        }
        if !json_output {
            let terminal = output.terminal();
            tmt_cli_style::detail::write(
                &mut output,
                terminal,
                "HELD MESSAGE",
                &[
                    ("device", event["deviceName"].as_str().unwrap_or("").into()),
                    ("source", event["clientId"].as_str().unwrap_or("").into()),
                    (
                        "recipient",
                        event["recipientId"].as_str().unwrap_or("").into(),
                    ),
                    ("message", event["message"].as_str().unwrap_or("").into()),
                ],
            )?;
            output.flush()?;
            let mut prompt = tmt_cli_style::stream::stderr();
            write!(prompt, "Send this frozen message? [y/N] ")?;
            prompt.flush()?;
        }
        let mut answer = String::new();
        let read = std::io::stdin().read_line(&mut answer)?;
        let confirmed = read > 0
            && if json_output {
                tmt_remote::wire::strict_json(answer.as_bytes())
                    .is_some_and(|v| v == json!({"op":"confirm"}))
            } else {
                matches!(answer.trim(), "y" | "yes")
            };
        writeln!(
            stream,
            "{}",
            json!({"op":if confirmed {"confirm"}else{"refuse"}})
        )?;
    }
}
/// `tmt remote pair`: open the single offer on the running serve and relay
/// the owner's one confirmation. State is reached only through serve.
fn pair(
    json_output: bool,
    flag: open::Flag,
    policy: tmt_remote::pairing::PairingPolicy,
) -> Result<(), RemoteError> {
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
    let loaded = settings::read_or_default(&root);
    let mut stream = control::connect(&root.join("remote"))?;
    stream.write_all(b"{\"op\":\"pair\"}\n")?;
    let mut events = BufReader::new(stream.try_clone()?);
    let answering = AtomicBool::new(false);
    let mut output = tmt_cli_style::stream::stdout(json_output);
    let mut paired = Err(RemoteError::new("REMOTE_PAIRING_ENDED", "Pairing ended."));
    let (selected, choice) = std::sync::mpsc::channel();
    let mut paired_device = String::new();
    let mut paired_name = String::new();
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
        // An old owner ignores policy fields. Refuse before exposing its offer
        // or answering it, so a read-only default cannot become a broad grant.
        if event["event"] == "offer" && event["ownerPolicyVersion"] != 1 {
            return Err(control::outdated_serve());
        }
        if json_output {
            writeln!(output, "{event}")?;
            output.flush()?;
        }
        match event["event"].as_str() {
            Some("offer") if !json_output => {
                let terminal = output.terminal();
                let link = event["link"].as_str().unwrap_or("");
                // Opened first, so the result is a row of the same block as the link.
                let opened = open::open_link(link, flag, loaded.open(), json_output);
                let mut rows = vec![
                    ("link", link.to_owned()),
                    ("code", event["code"].as_str().unwrap_or("").into()),
                    ("expires", "in 10 minutes".into()),
                ];
                rows.extend(open_row(&opened));
                tmt_cli_style::detail::write(&mut output, terminal, "PAIR A DEVICE", &rows)?;
                output.flush()?;
                warn_settings(&loaded)?;
                if let open::Outcome::Failed(why) = opened {
                    let mut diagnostic = tmt_cli_style::stream::stderr();
                    let terminal = diagnostic.terminal();
                    tmt_cli_style::message::warning(
                        &mut diagnostic,
                        terminal,
                        &format!("Could not open the browser ({why}); use the link above"),
                        None,
                    )?;
                }
            }
            Some("candidate") => {
                paired_name = event["name"].as_str().unwrap_or("").to_owned();
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
                            ("grant", pairing_grant_row(&policy)),
                        ],
                    )?;
                    output.flush()?;
                }
                // One answer per offer, written back by its own thread so an
                // offer that ends first (expiry, refusal) is still reported.
                if !answering.swap(true, Ordering::AcqRel) {
                    let mut control = stream.try_clone()?;
                    let policy = policy.clone();
                    let selected = selected.clone();
                    std::thread::spawn(move || {
                        let selected_policy = pairing_answer(
                            &mut std::io::stdin().lock(),
                            &mut tmt_cli_style::stream::stderr(),
                            json_output,
                            policy,
                        );
                        let line = match selected_policy {
                            Ok(Some(policy)) => {
                                // Publish the choice before confirm: the owner's paired event
                                // cannot arrive before this write, so reporting never waits on stdin.
                                let _ = selected.send(policy.talk);
                                json!({"op":"confirm","policy":policy})
                            }
                            _ => json!({"op":"refuse"}),
                        };
                        let _ = writeln!(control, "{line}");
                    });
                }
            }
            Some("ended") => {
                let reason = event["reason"].as_str().unwrap_or("ended");
                paired = if reason == "paired" {
                    paired_device = event["clientId"].as_str().unwrap_or("").to_owned();
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
        let message = pairing_result(
            &paired_name,
            &paired_device,
            choice.try_recv().unwrap_or(policy.talk),
            policy.hold,
        );
        tmt_cli_style::message::success(&mut output, terminal, &message)?;
    }
    paired
}
const PAIR_TRUST_PROMPT: &str = "Trust this device if the words match? [y/N] ";
const PAIR_TALK_PROMPT: &str = "Also allow this device to send to agents? [y/N] ";
fn pairing_result(name: &str, client_id: &str, talk: bool, hold: bool) -> String {
    if talk {
        format!(
            "Paired {name}. It can send to agents{}.",
            if hold { " · held" } else { "" }
        )
    } else {
        format!(
            "Paired {name} as read only. To allow sending later: tmt remote devices talk {client_id} on"
        )
    }
}
fn pairing_grant_row(policy: &tmt_remote::pairing::PairingPolicy) -> String {
    if !policy.talk {
        return "Read only · no expiry".into();
    }
    format!(
        "Read and send · {} · {} · no expiry",
        policy.agents.as_ref().map_or_else(
            || "all agents".into(),
            |ids| format!(
                "{} {}",
                ids.len(),
                if ids.len() == 1 { "agent" } else { "agents" }
            )
        ),
        if policy.hold { "held" } else { "direct" },
    )
}
/// Trust and sending are separate owner choices. JSON consumes only the trust
/// answer; its sending policy comes exclusively from the owner's explicit flag.
fn pairing_answer(
    input: &mut impl BufRead,
    prompt: &mut impl Write,
    json_output: bool,
    mut policy: tmt_remote::pairing::PairingPolicy,
) -> std::io::Result<Option<tmt_remote::pairing::PairingPolicy>> {
    if !json_output {
        write!(prompt, "{PAIR_TRUST_PROMPT}")?;
        prompt.flush()?;
    }
    let mut answer = String::new();
    if input.read_line(&mut answer)? == 0 || !matches!(answer.trim(), "y" | "yes" | "confirm") {
        return Ok(None);
    }
    if !json_output && !policy.talk {
        write!(prompt, "{PAIR_TALK_PROMPT}")?;
        prompt.flush()?;
        answer.clear();
        input.read_line(&mut answer)?;
        policy.talk = matches!(answer.trim(), "y" | "yes");
    }
    Ok(Some(policy))
}
/// The `open` row of the pairing detail block, present only when a browser was handed the link.
fn open_row(outcome: &open::Outcome) -> Option<(&'static str, String)> {
    matches!(outcome, open::Outcome::Opened).then(|| ("open", "opened in your browser".to_owned()))
}
fn warn_settings(loaded: &settings::RemoteSettings) -> Result<(), RemoteError> {
    if loaded.malformed {
        let mut output = tmt_cli_style::stream::stderr();
        let terminal = output.terminal();
        tmt_cli_style::message::warning(&mut output, terminal, settings::UNREADABLE, None)?;
    }
    Ok(())
}
fn settings_command(arguments: &clap::ArgMatches) -> Result<(), RemoteError> {
    let root = discovery_root()?;
    let loaded = match arguments.get_one::<String>("value") {
        Some(value) => match arguments.get_one::<String>("key").map(String::as_str) {
            Some("open") if matches!(value.as_str(), "on" | "off") => {
                settings::set_open(&root, value == "on")?
            }
            Some("default-remote") => {
                let layout = Layout::open(&root)?;
                if value != "local" {
                    tmt_remote::remote_selector::validate_reference(&layout, value)?;
                }
                settings::set_default_remote(&root, value)?
            }
            Some("sessions-per-device") => {
                let limit = if value == "off" {
                    None
                } else {
                    Some(
                        value
                            .parse::<usize>()
                            .ok()
                            .filter(|n| *n > 0 && n.to_string() == *value)
                            .ok_or_else(|| {
                                RemoteError::new(
                                    "REMOTE_INPUT_INVALID",
                                    "Session limit must be a positive integer or off.",
                                )
                            })?,
                    )
                };
                settings::set_sessions_per_device(&root, limit)?
            }
            _ => {
                return Err(RemoteError::new(
                    "REMOTE_INPUT_INVALID",
                    "Browser opening must be on or off.",
                ));
            }
        },
        None => settings::read_or_default(&root),
    };
    let json_output = arguments.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        let mut value = loaded.json();
        value["defaultRemote"] = json!(loaded.default_remote().unwrap_or("local"));
        value["defaultRemoteSource"] = json!(loaded.default_source());
        writeln!(output, "{}", value)?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            "REMOTE SETTINGS",
            &[
                (
                    "default-remote",
                    format!(
                        "{} ({})",
                        if loaded.default_remote().unwrap_or("local") == "local" {
                            "local"
                        } else {
                            loaded.default_remote().unwrap()
                        },
                        loaded.default_source()
                    ),
                ),
                (
                    "open",
                    format!(
                        "{} ({})",
                        if loaded.open() { "on" } else { "off" },
                        loaded.source()
                    ),
                ),
                (
                    "sessions-per-device",
                    format!(
                        "{} ({})",
                        loaded
                            .sessions_per_device()
                            .map_or_else(|| "off (unlimited)".into(), |n| n.to_string()),
                        loaded.sessions_source()
                    ),
                ),
            ],
        )?;
        warn_settings(&loaded)?;
    }
    Ok(())
}
/// `tmt remote devices [revoke|rename ...]`, through the running serve or,
/// when none runs, directly under the serve lock.
fn devices(arguments: &clap::ArgMatches) -> Result<(), RemoteError> {
    let mutation = arguments.subcommand();
    let json_output =
        arguments.get_flag("json") || mutation.is_some_and(|(_, m)| m.get_flag("json"));
    let request = match mutation {
        Some(("talk", m)) => {
            json!({"op":"talk","clientId":m.get_one::<String>("client-id").unwrap(),"enabled":m.get_one::<String>("enabled").unwrap()=="on"})
        }
        Some(("rename", m)) => {
            json!({"op":"rename","clientId":m.get_one::<String>("client-id").unwrap(), "name":m.get_one::<String>("name").unwrap()})
        }
        Some(("revoke", m)) => {
            json!({"op":"revoke","clientId":m.get_one::<String>("client-id").unwrap()})
        }
        Some(("designate", m)) => {
            json!({"op":"designate","clientId":m.get_one::<String>("client-id").unwrap()})
        }
        Some(("undesignate", _)) => json!({"op":"undesignate"}),
        Some(("remove-others", m)) => {
            json!({"op":"remove-others","keep":m.get_one::<String>("keep").unwrap()})
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
            let mut store = Store::open(&serving)?;
            let origin = store
                .remembered_port()?
                .map(|port| format!("http://127.0.0.1:{port}"));
            store.machine()?;
            let devices = Devices::new(Arc::new(Mutex::new(store)), None);
            match mutation {
                Some(("talk", m)) => {
                    json!({"device":devices.describe(&devices.talk(m.get_one::<String>("client-id").unwrap(), m.get_one::<String>("enabled").unwrap()=="on")?)?})
                }
                Some(("rename", m)) => {
                    json!({"device": devices.describe(&devices.rename(m.get_one::<String>("client-id").unwrap(), m.get_one::<String>("name").unwrap())?)?})
                }
                Some(("revoke", m)) => {
                    json!({"device": devices.describe(&devices.revoke(m.get_one::<String>("client-id").unwrap())?)?})
                }
                Some(("designate", m)) => {
                    let origin = origin.ok_or_else(|| {
                        RemoteError::new(
                            "REMOTE_NOT_RUNNING",
                            "No remembered door origin; run serve first.",
                        )
                    })?;
                    let grant =
                        devices.designate(m.get_one::<String>("client-id").unwrap(), &origin)?;
                    json!({"designatedClientId":grant.client_id})
                }
                Some(("remove-others", m)) => {
                    json!({"removed":devices.remove_others(m.get_one::<String>("keep").unwrap())?})
                }
                Some(("undesignate", _)) => {
                    devices.undesignate()?;
                    json!({"designatedClientId":null})
                }
                Some(_) => unreachable!("typed device grammar"),
                None => {
                    json!({"devices": devices.visible()?})
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
    if let Some(client) = answer.get("designatedClientId") {
        return Ok(tmt_cli_style::message::success(
            &mut output,
            terminal,
            &if let Some(client) = client.as_str() {
                format!("Designated browser {client}")
            } else {
                "Removed browser management designation".to_owned()
            },
        )?);
    }
    if let Some(count) = answer["removed"].as_u64() {
        return Ok(tmt_cli_style::message::success(
            &mut output,
            terminal,
            &format!(
                "Removed {count} other {}",
                if count == 1 { "device" } else { "devices" }
            ),
        )?);
    }
    if let Some(device) = answer.get("device") {
        let name = device["name"].as_str().unwrap_or("");
        return Ok(tmt_cli_style::message::success(
            &mut output,
            terminal,
            &match request["op"].as_str() {
                Some("rename") => format!("Renamed {name}"),
                Some("talk") if request["enabled"] == true => format!("Sending is on for {name}."),
                Some("talk") => format!("Sending is off for {name}."),
                _ => format!("Revoked {name}. This machine no longer accepts their requests."),
            },
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
        Column::Detail,
    ]);
    let now = tmt_remote::pairing::now_ms()?;
    for device in &listed {
        let mark = Mark::Running;
        let last_used = device["lastUsedAtMs"]
            .as_u64()
            .map(|time| {
                format!(
                    "Last used {}",
                    tmt_cli_style::value::relative_time(now.saturating_sub(time))
                )
            })
            .unwrap_or_else(|| "Last used Not recorded".into());
        let text = |key: &str| Cell::from(device[key].as_str().unwrap_or(""));
        rows.row([
            Cell::styled(mark.symbol(), mark.token()),
            text("name"),
            text("kind"),
            text("words"),
            text("clientId"),
            Cell::from(last_used.as_str()),
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
    if let Some(("__activate", arguments)) = matches.subcommand() {
        return activation_cli::run(arguments);
    }
    match run(&matches) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if error.code == "REMOTE_SYNC_REPORTED" {
                return ExitCode::FAILURE;
            }
            if (matches.subcommand_name() == Some("launch") || wants_json(&matches))
                && error.code != "REMOTE_READY_OUTPUT"
            {
                let mut value = error.json_value();
                if matches.subcommand_name() == Some("launch") {
                    value["version"] = json!(1);
                }
                let _ = writeln!(tmt_cli_style::stream::stdout(true), "{}", value);
            } else {
                let mut output = tmt_cli_style::stream::stderr();
                let terminal = output.terminal();
                let mut hints = error.hint.as_deref().unwrap_or("").lines();
                let _ = tmt_cli_style::message::error(
                    &mut output,
                    terminal,
                    &error.message,
                    hints.next(),
                );
                for hint in hints {
                    let _ = tmt_cli_style::message::hint(&mut output, terminal, hint);
                }
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod cli_style_allowlist;

#[cfg(test)]
mod tests {
    use super::{cli_style_allowlist, grammar, open::Outcome, open_row};

    #[test]
    fn anonymous_connect_help_separates_reference_sign_in_and_browser_page_access() {
        for (words, expected) in [
            (
                vec!["link"],
                vec![
                    "Print a link to this remote's join guide",
                    "Read the public link without opening a browser",
                    "Share this link so someone can connect to this remote. It holds public connection details only and gives no access to shared pages or the project. Printing it opens no browser and changes nothing. The same link can be shared again.",
                ],
            ),
            (
                vec!["add"],
                vec![
                    "Remember a remote on this machine",
                    "tmt remote add team '<remote-link>' --json",
                    "Remember the remote as team",
                    "Save the remote's public connection details under a name you choose. This does not sign in, grant page access or change the project.",
                ],
            ),
            (
                vec!["sync"],
                vec![
                    "For a remote added with tmt remote add, page access for this machine is not available yet.",
                    "To open a shared page in your browser, use the page link from its owner.",
                ],
            ),
            (
                vec!["sync", "sign-in"],
                vec![
                    "Sign this machine in to a remote",
                    "Sign this machine in to the saved remote",
                    "Use the selected remote's anonymous sign-in. No personal account is needed. An active sign-in is reused. After it ends, signing in creates a new identity. For a remote added with tmt remote add, page access for this machine is not available yet. To open a shared page in your browser, use the page link from its owner. If a sign-in is not confirmed, it is not retried automatically; run the command again.",
                ],
            ),
        ] {
            let mut command = grammar();
            for word in &words {
                command = command.find_subcommand(word).unwrap().clone();
            }
            let help = tmt_cli_style::help_text(&command, tmt_cli_style::Terminal::PLAIN)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            for text in expected {
                assert!(help.contains(text), "{words:?}: missing {text:?} in {help}");
            }
            assert!(!help.contains("pairing still grants access"));
            assert!(!help.contains("owner must admit"));
        }
    }

    #[test]
    fn sync_commands_use_the_shipped_selector_and_have_no_browser_flags() {
        for action in ["sign-in", "revoke"] {
            let words = [
                "tmt-remote",
                "--remote",
                "tenant",
                "sync",
                action,
                "--project",
                "demo-remote-1",
                "--json",
            ];
            let matches = grammar().try_get_matches_from(words).unwrap();
            assert_eq!(matches.get_one::<String>("remote").unwrap(), "tenant");
            let action_matches = matches.subcommand().unwrap().1.subcommand().unwrap().1;
            assert_eq!(
                action_matches.get_one::<String>("project").unwrap(),
                "demo-remote-1"
            );
            assert!(action_matches.get_flag("json"));
            assert!(
                grammar()
                    .try_get_matches_from(["tmt-remote", "sync", action, "--no-open"])
                    .is_err()
            );
        }
        let mut command = grammar();
        let mut help = Vec::new();
        command
            .find_subcommand_mut("sync")
            .unwrap()
            .find_subcommand_mut("sign-in")
            .unwrap()
            .write_long_help(&mut help)
            .unwrap();
        let help = String::from_utf8(help).unwrap();
        assert!(help.contains("active sign-in is reused"));
        assert!(help.contains("tmt remote devices revoke"));
    }

    #[test]
    fn hidden_commands_are_listed_with_a_reason_and_double_underscore_commands_are_hidden() {
        let report = tmt_cli_style::audit::hidden_report(
            &grammar(),
            &["tmt", "remote"],
            cli_style_allowlist::HIDDEN,
        );
        assert!(report.is_empty(), "{}", report.join("\n"));
    }

    #[test]
    fn trust_confirmation_and_sending_choice_are_separate_and_default_to_read_only() {
        use tmt_remote::pairing::PairingPolicy;
        for (input, json, opt_in, expected, second) in [
            ("y\nn\n", false, false, Some(false), true),
            ("y\n\n", false, false, Some(false), true),
            ("y\ny\n", false, false, Some(true), true),
            ("y\n", false, true, Some(true), false),
            ("confirm\n", true, false, Some(false), false),
            ("confirm\n", true, true, Some(true), false),
            ("n\n", false, false, None, false),
            ("y\n", false, false, Some(false), true),
            ("", false, false, None, false),
        ] {
            let mut prompts = Vec::new();
            let answer = super::pairing_answer(
                &mut input.as_bytes(),
                &mut prompts,
                json,
                PairingPolicy {
                    talk: opt_in,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(answer.map(|policy| policy.talk), expected);
            let expected_prompt = if json {
                "".into()
            } else if second {
                format!("{}{}", super::PAIR_TRUST_PROMPT, super::PAIR_TALK_PROMPT)
            } else {
                super::PAIR_TRUST_PROMPT.to_owned()
            };
            assert_eq!(String::from_utf8(prompts).unwrap(), expected_prompt);
        }
        assert_eq!(
            super::pairing_grant_row(&PairingPolicy::default()),
            "Read only · no expiry"
        );
        assert_eq!(
            super::pairing_grant_row(&PairingPolicy {
                talk: true,
                ..Default::default()
            }),
            "Read and send · all agents · direct · no expiry"
        );
        assert_eq!(
            super::pairing_grant_row(&PairingPolicy {
                talk: true,
                hold: true,
                agents: Some(vec!["11111111-1111-4111-8111-111111111111".into()])
            }),
            "Read and send · 1 agent · held · no expiry"
        );
        for (talk, hold, expected) in [
            (
                false,
                false,
                "Paired Fixture device as read only. To allow sending later: tmt remote devices talk 11111111-1111-4111-8111-111111111111 on",
            ),
            (true, false, "Paired Fixture device. It can send to agents."),
            (
                true,
                true,
                "Paired Fixture device. It can send to agents · held.",
            ),
        ] {
            assert_eq!(
                super::pairing_result(
                    "Fixture device",
                    "11111111-1111-4111-8111-111111111111",
                    talk,
                    hold
                ),
                expected
            );
        }
    }

    #[test]
    fn only_a_handed_over_link_gets_an_open_row() {
        assert_eq!(
            open_row(&Outcome::Opened),
            Some(("open", "opened in your browser".to_owned()))
        );
        for outcome in [
            Outcome::Skipped,
            Outcome::NoOpener,
            Outcome::Failed("exit 9".into()),
        ] {
            assert_eq!(open_row(&outcome), None);
        }
    }
}

#[cfg(test)]
mod deploy_prompt_tests {
    use super::*;
    #[test]
    fn scripted_terminal_yes_no_eof_and_cancellation_are_bounded() {
        use std::os::unix::net::UnixStream;
        for (answer, stop, expected) in [
            (b"y\n".as_slice(), false, true),
            (b"N\n", false, false),
            (b"", false, false),
            (b"yes\n", true, false),
        ] {
            let (mut input, mut writer) = UnixStream::pair().unwrap();
            writer.write_all(answer).unwrap();
            writer.shutdown(std::net::Shutdown::Write).unwrap();
            let mut output = Vec::new();
            assert_eq!(
                confirm_deploy(&mut input, &mut output, &AtomicBool::new(stop)).unwrap(),
                expected
            );
            assert_eq!(
                String::from_utf8(output).unwrap(),
                "Deploy these changes to your Firebase project? [y/N] "
            );
        }
    }
}
