mod cli_grammar;
mod cli_management;
mod cli_threads;
mod door;
mod open;
mod reach;
mod settings_cli;
mod status;
mod supervisor;
const IPC_RESPONSE_BYTES: usize = 8192;
const SKILL: &str = include_str!("../../../skills/tmt-colab/SKILL.md");
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

/// `--open` / `--no-open`: per command, over the `open` setting.
fn open_args(command: Command) -> Command {
    command
        .arg(
            Arg::new("open")
                .long("open")
                .action(ArgAction::SetTrue)
                .overrides_with("no-open")
                .help("Open the link in your browser even when the setting or environment says not to"),
        )
        .arg(
            Arg::new("no-open")
                .long("no-open")
                .action(ArgAction::SetTrue)
                .overrides_with("open")
                .help("Only print the link"),
        )
}
fn grammar() -> Command {
    const ROOT: CommandSpec = CommandSpec {
        name: "colab",
        summary: "Local collaborative-space pilot",
        examples: &[
            Example {
                command: "tmt colab serve",
                note: "Run the space; stop it with tmt colab stop",
            },
            Example {
                command: "tmt colab page create --title Notes --file page.html",
                note: "Create a page and print its link",
            },
            Example {
                command: "tmt colab share mode 10000000-0000-4000-8000-000000000001 link --yes",
                note: "Allow read-only share links for a page",
            },
        ],
        outputs: OutputModes::Human,
        details: "Every command that names a page prints its full link: give that link to the person, not JSON. The space is reached through tmt remote, which mounts it for paired browsers. Serve the bundled browser app, or build it for local development. Local root-authorized management uses the same owner service as mounted browser requests.",
    };
    const SKILL_COMMAND: CommandSpec = CommandSpec {
        name: "skill",
        summary: "Print the bundled Colab agent skill",
        examples: &[Example {
            command: "tmt colab skill",
            note: "Read the exact instructions shipped with this executable",
        }],
        outputs: OutputModes::Human,
        details: "Prints the canonical skill bytes without core discovery, storage access or a running server. Install through tmt extension install colab --skills; this command only reads the embedded instructions.",
    };
    const SERVE: CommandSpec = CommandSpec {
        name: "serve",
        summary: "Serve a local space in the foreground",
        examples: &[Example {
            command: "tmt colab serve",
            note: "Serve the space on its owner-only socket for tmt remote",
        }],
        outputs: OutputModes::HumanAndJson,
        details: "Listens only on <data root>/colab/door.sock. Attaches to a running Remote door, or starts tmt remote serve itself, and prints the state and the next step (pairing stays explicit: tmt remote pair).\nCtrl-C or SIGTERM closes the socket, its workers and tunnels, then stops a door it started; an attached door keeps running.",
    };
    const STOP: CommandSpec = CommandSpec {
        name: "stop",
        summary: "Stop the running tmt colab serve",
        examples: &[Example {
            command: "tmt colab stop",
            note: "Ask the serving Colab to shut down, then wait until it has",
        }],
        outputs: OutputModes::HumanAndJson,
        details: "Asks the serving process over its owner-only socket, never by signalling a pid. A Remote door that serve started stops with it; a door it only attached to keeps running. Pairings and data are untouched. Not running is not an error.",
    };
    const OPEN: CommandSpec = CommandSpec {
        name: "open",
        summary: "Open the existing space or a page in your browser",
        examples: &[
            Example {
                command: "tmt colab open",
                note: "Open the running space home",
            },
            Example {
                command: "tmt colab open 10000000-0000-4000-8000-000000000001",
                note: "Open an existing page; a unique UUID prefix also works",
            },
        ],
        outputs: OutputModes::HumanAndJson,
        details: "Omit PAGE for the existing space home. This explicit request opens even without a terminal or with the automatic-open setting off. --no-open and --json only print link facts. Uses the running Colab and Remote services; never starts another service, pairs a browser or changes page access. If Colab is stopped, run tmt colab serve. Missing, deleted or ambiguous pages are refused before opening.",
    };
    const SETTINGS: CommandSpec = CommandSpec {
        name: "settings",
        summary: "Show or change Colab settings",
        examples: &[
            Example {
                command: "tmt colab settings",
                note: "Show each setting and where its value comes from",
            },
            Example {
                command: "tmt colab settings open off",
                note: "Stop opening the browser from serve and page create",
            },
        ],
        outputs: OutputModes::HumanAndJson,
        details: "open (default on) controls whether serve and page create open the link in your browser. It is skipped without a terminal, with --json, in CI, in an SSH session without a display and when no opener is installed; --open and --no-open override it per command. Stored in <data root>/colab/settings.json.",
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
        details: "This creates an unencrypted copy of the page. Anyone with these files can read it.\nCreates page.html, conversations.json, conversations.md and manifest.json in a new UUID subdirectory of --dir (default: current directory). The parent must exist; aliases resolve to a canonical path. Created entries cannot be symlinks; parent traversal and overwrite are refused. The conversations files hold the page's verified threads, comments and Ask conversations for the current epoch (names and times are labels). Archived or deleted pages cannot be exported yet.",
    };
    const PAGE: CommandSpec = CommandSpec {
        name: "page",
        summary: "Create, read and write local pages",
        examples: &[
            Example {
                command: "tmt colab page create --title Notes --file page.html",
                note: "Create a private page and print its link",
            },
            Example {
                command: "tmt colab page read 10000000-0000-4000-8000-000000000001 --json",
                note: "Read source and its verified editing base",
            },
        ],
        outputs: OutputModes::Human,
        details: "Root-local page access using existing encrypted state.",
    };
    const CREATE: CommandSpec = CommandSpec {
        name: "create",
        summary: "Create a private local page",
        examples: &[Example {
            command: "tmt colab page create --title Notes --file page.html --json",
            note: "Create a page editable by your registered owner browsers",
        }],
        outputs: OutputModes::HumanAndJson,
        details: "Initializes a fresh local space when needed. Without --file the source is empty; use --file - for bounded UTF-8 stdin. Commits a private page, epoch key, owner-device wraps and encrypted initial content through the same owner service whether serve is running or stopped. While a Remote door runs, the full link is printed (link in JSON, null otherwise) and opened in your browser unless --no-open, --json, the open setting or the environment says not to; without a door the path relative to the Remote door address and the reason are printed.",
    };
    const READ: CommandSpec = CommandSpec {
        name: "read",
        summary: "Read exact admitted UTF-8 source",
        examples: &[Example {
            command: "tmt colab page read 10000000-0000-4000-8000-000000000001 --json",
            note: "Read source, title and verified revision",
        }],
        outputs: OutputModes::HumanAndJson,
        details: "Writes source bytes unchanged to stdout, with revision/head/epoch on stderr. JSON contains both. Does not create or migrate state.",
    };
    const WRITE: CommandSpec = CommandSpec {
        name: "write",
        summary: "Write page source against a verified base",
        examples: &[Example {
            command: "tmt colab page write 10000000-0000-4000-8000-000000000001 --file page.html --json",
            note: "Replace the page source",
        }],
        outputs: OutputModes::HumanAndJson,
        details: "Retains the title and replaces the page source (at most 2 MiB) as one atomic batch of signed content updates, through serve when running, or under its lifecycle lock when stopped. A source equal to the current one publishes nothing and reports changed false. If a serving write's reply is lost, the command checks the original operation once and otherwise reports COLAB_OUTCOME_UNKNOWN with its operation ID; it never resends. Use the opaque revision from page read as --expected-revision. Without it, the base is captured when this command starts; intervening changes still reject.",
    };
    cli_grammar::extend(
        tmt_cli_style::command(&ROOT)
            .bin_name("tmt colab")
            .version(env!("CARGO_PKG_VERSION"))
            .arg(tmt_cli_style::version_arg(ArgAction::Version))
            .subcommand_required(true)
            .subcommand(open_args(
                tmt_cli_style::command(&SERVE).arg(
                    Arg::new("app-dir")
                        .long("app-dir")
                        .value_name("DIRECTORY")
                        .value_parser(clap::value_parser!(std::path::PathBuf))
                        .help(
                            "Override embedded or checkout app bytes with an absolute build directory",
                        ),
                ),
            ))
            .subcommand(tmt_cli_style::command(&SKILL_COMMAND))
            .subcommand(tmt_cli_style::command(&STOP))
            .subcommand(open_args(
                tmt_cli_style::command(&OPEN).arg(cli_grammar::page().required(false)),
            ))
            .subcommand(
                tmt_cli_style::command(&SETTINGS)
                    .arg(
                        Arg::new("key")
                            .value_name("setting")
                            .value_parser(["open"]),
                    )
                    .arg(
                        Arg::new("value")
                            .value_name("on|off")
                            .value_parser(["on", "off"])
                            .requires("key"),
                    ),
            )
            .subcommand(tmt_cli_style::command(&SPACES))
            .subcommand(cli_threads::command())
            .subcommand(
                tmt_cli_style::command(&PAGE)
                    .subcommand_required(true)
                    .subcommand(
                        open_args(
                            tmt_cli_style::command(&CREATE)
                                .arg(Arg::new("title").long("title").required(true).value_name("TITLE"))
                                .arg(Arg::new("file").long("file").value_name("path|-")
                                    .value_parser(clap::value_parser!(std::path::PathBuf))),
                        ),
                    )
                    .subcommand(tmt_cli_style::command(&READ).arg(cli_grammar::page()))
                    .subcommand(
                        tmt_cli_style::command(&WRITE)
                            .arg(cli_grammar::page())
                            .arg(
                                Arg::new("file")
                                    .long("file")
                                    .required(true)
                                    .value_name("path|-")
                                    .value_parser(clap::value_parser!(std::path::PathBuf)),
                            )
                            .arg(
                                Arg::new("expected-revision")
                                    .long("expected-revision")
                                    .value_name("revision")
                                    .value_parser(|value: &str| {
                                        if value.strip_prefix("v1:").is_some_and(|h| {
                                            h.len() == 64
                                                && h.bytes().all(|b| {
                                                    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
                                                })
                                        }) {
                                            Ok(value.to_owned())
                                        } else {
                                            Err("Use the opaque revision returned by page read")
                                        }
                                    }),
                            ),
                    ),
            )
            .subcommand(
                tmt_cli_style::command(&EXPORT)
                    .arg(cli_grammar::page())
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
    if command == "skill" {
        let mut output = tmt_cli_style::stream::stdout(false);
        output.write_all(SKILL.as_bytes())?;
        output.flush()?;
        return Ok(());
    }
    let stop = Arc::new(AtomicBool::new(false));
    let mut signals = Vec::new();
    let result = (|| -> Result<()> {
        for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
            signals.push(signal_hook::flag::register(signal, Arc::clone(&stop))?);
        }
        let root = core::data_root(&stop)?;
        if command == "page" {
            return page(&root, args);
        }
        if command == "threads" {
            return cli_threads::run(&root, args);
        }
        let json_output = args.get_flag("json");
        if command == "spaces" {
            return spaces(&root, json_output);
        }
        if command == "settings" {
            return settings_cli::run(&root, args, json_output);
        }
        if command == "stop" {
            return stop_serving(&root, json_output);
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
        let (pages, all_pages) = open_pages(&store, &keyring);
        let save_root = root.clone();
        let registration = Arc::new(Mutex::new(
            Registration::new(store, keyring, std::env::current_exe()?)?.with_save_source(
                Arc::new(move || {
                    tmt_colab::page::save::open_source(
                        &save_root,
                        tmt_colab::decoder::Config::new(std::env::current_exe()?),
                    )
                }),
            ),
        ));
        let release = Arc::new(tmt_colab::serve_release::ServeRelease::new(
            tmt_colab::serve_release::Running::detect(),
        ));
        let socket = MountSocket::bind(&layout, &space_id, Tunnels::PRODUCT)?
            .with_registration(&layout, Arc::clone(&registration))?
            .with_release(Arc::clone(&release))
            .with_app(app);
        // The socket is bound first, so a door started now mounts it as soon as it is ready.
        let access = supervisor::Access::open(&stop);
        let socket = socket.with_door(match &access {
            supervisor::Access::Attached(_) => "attached",
            supervisor::Access::Started { .. } => "started",
            supervisor::Access::Unavailable { .. } => "unavailable",
        });
        let pairing = match &access {
            supervisor::Access::Unavailable { .. } => None,
            _ => Some(door::Pairing::lookup()),
        };
        let mut status = status::Status {
            space: &space_id,
            socket: &socket.path,
            access: &access,
            pairing,
            pages,
            all_pages,
            opened: false,
        };
        // Once the door is ready, open the page (or the space home) unless told or unable not to.
        let mut open_warnings = Vec::new();
        if let Some(link) = status.open_link() {
            // The door is ready: unreadable settings are the defaults, never a failed serve.
            let settings = tmt_colab::settings::read_or_default(&root);
            let outcome = open::open_link(&link, open::flag(args), settings.open(), json_output);
            status.opened = matches!(outcome, open::Outcome::Opened);
            open_warnings.extend(open::describe(&outcome, &link).1);
            if settings.malformed {
                open_warnings.push(tmt_colab::settings::UNREADABLE.to_owned());
            }
        }
        if json_output {
            writeln!(output, "{}", status.json())?;
        } else {
            let terminal = output.terminal();
            let rows = status.rows();
            tmt_cli_style::detail::write(&mut output, terminal, "LOCAL SPACE", &rows)?;
        }
        if let (Some((what, hint)), false) = (access.warning(), json_output) {
            let mut warning = tmt_cli_style::stream::stderr();
            let terminal = warning.terminal();
            tmt_cli_style::message::warning(&mut warning, terminal, what, hint)?;
        }
        for what in open_warnings.iter().filter(|_| !json_output) {
            let mut warning = tmt_cli_style::stream::stderr();
            let terminal = warning.terminal();
            tmt_cli_style::message::warning(&mut warning, terminal, what, None)?;
        }
        output.flush()?;
        drop(output);
        // The watcher ends with the serve, however the socket loop was stopped.
        let serving = Arc::new(AtomicBool::new(true));
        let watcher = (!json_output).then(|| watch_release(&release, &serving));
        let result = socket.run(&stop);
        serving.store(false, std::sync::atomic::Ordering::Relaxed);
        if let Some(watcher) = watcher {
            let _ = watcher.join();
        }
        let closed = Arc::try_unwrap(registration)
            .map_err(|_| "Registration worker retained.")?
            .into_inner()
            .map_err(|_| "Registration lock poisoned.")?
            .close();
        // The door Colab started stops after its own socket is closed.
        drop(access);
        result?;
        closed
    })();
    for signal in signals {
        signal_hook::low_level::unregister(signal);
    }
    result
}
/// Tells the foreground user, once, when the installed release is no longer the one serving.
/// The check reads only the install layout; it never restarts, signals or changes the serve.
/// It runs until `serving` clears and a stop within a tenth of a second.
fn watch_release(
    release: &Arc<tmt_colab::serve_release::ServeRelease>,
    serving: &Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    let (release, serving) = (Arc::clone(release), Arc::clone(serving));
    std::thread::spawn(move || {
        // Check at most every `PERIOD`, but notice a stop within a tenth of a second.
        const PERIOD: u32 = 50;
        let mut ticks = PERIOD;
        while serving.load(std::sync::atomic::Ordering::Relaxed) {
            if ticks >= PERIOD {
                ticks = 0;
                if let Some(stale) = release.fresh() {
                    let mut warning = tmt_cli_style::stream::stderr();
                    let terminal = warning.terminal();
                    let _ = tmt_cli_style::message::warning(
                        &mut warning,
                        terminal,
                        &tmt_colab::serve_release::restart_text(&stale),
                        Some(tmt_colab::serve_release::RESTART_HINT),
                    );
                    return;
                }
            }
            ticks += 1;
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    })
}
/// Ids of the pages that are not archived, for the start-up status. A catalog that cannot be
/// read is unknown, never a reason to refuse to serve.
fn open_pages(store: &Store, keyring: &Keyring) -> (Option<Vec<String>>, Option<Vec<String>>) {
    let Some(catalog) = tmt_colab::inspection::catalog(store, keyring).ok() else {
        return (None, None);
    };
    let ids = |rows: &serde_json::Value, active: bool| -> Option<Vec<String>> {
        rows.as_array()?
            .iter()
            .filter(|page| !active || page["archived"] != true)
            .map(|page| page["pageId"].as_str().map(str::to_owned))
            .collect()
    };
    (
        ids(&catalog["pages"], true),
        ids(&catalog["pageIds"], false),
    )
}

/// How long a stop request waits for the serving process to release its lock: the door's grace
/// period plus socket and worker shutdown.
const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
fn stop_serving(root: &std::path::Path, json_output: bool) -> Result<()> {
    use std::time::Instant;
    use tmt_colab::control::{self, StopFault};
    let layout = Layout::existing(root)?.filter(|layout| layout.running().unwrap_or(false));
    let door = match &layout {
        None => None,
        Some(layout) => {
            let reply = control::request_stop(layout)?;
            let deadline = Instant::now() + STOP_WAIT;
            while layout.running()? {
                if Instant::now() >= deadline {
                    return Err(StopFault::Slow.into());
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            reply.door()
        }
    };
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        let state = if door.is_some() {
            "stopped"
        } else {
            "not-running"
        };
        writeln!(output, "{}", json!({"state":state,"door":door}))?;
        return Ok(());
    }
    let terminal = output.terminal();
    let done = match door {
        None => "Colab is not running",
        Some("started") => "Colab stopped, and the Remote door it started",
        Some(_) => "Colab stopped",
    };
    tmt_cli_style::message::success(&mut output, terminal, done)?;
    if door == Some("attached") {
        let note = tmt_cli_style::table::escape(door::Door::ATTACHED_STOP_NOTE);
        writeln!(
            output,
            "{}",
            terminal.paint(tmt_cli_style::palette::Token::Dim, &note)
        )?;
    }
    Ok(())
}
fn export(root: &std::path::Path, args: &clap::ArgMatches) -> Result<()> {
    use tmt_colab::export::{Bundle, DISCLOSURE, Fault};
    let layout = Layout::existing(root)?.ok_or(Fault::MissingState)?;
    let keyring = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let page_id = cli_management::resolve_page(
        &store,
        &keyring,
        args.get_one::<String>("page").expect("required page"),
    )?;
    let mut decoder = tmt_colab::decoder::Decoder::new(std::env::current_exe()?)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis()
        .try_into()?;
    let bundle = Bundle::capture(&store, &keyring, &page_id, &mut decoder, now);
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
        let mut value = serde_json::to_value(&published)?;
        value["pageId"] = json!(page_id);
        writeln!(output, "{value}")?;
    } else {
        let terminal = output.terminal();
        tmt_cli_style::detail::write(
            &mut output,
            terminal,
            "PAGE EXPORTED",
            &[
                ("page", page_id),
                ("directory", published.directory.display().to_string()),
                (
                    "files",
                    published
                        .files
                        .iter()
                        .map(|file| file.name)
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
                (
                    "discussions",
                    "included for the current epoch (conversations.json, conversations.md)".into(),
                ),
            ],
        )?;
    }
    Ok(())
}
fn page(root: &std::path::Path, args: &clap::ArgMatches) -> Result<()> {
    use tmt_colab::{
        decoder::Decoder,
        page::{self, Fault},
    };
    let (command, args) = args.subcommand().expect("required page command");
    if command == "create" {
        let source = args
            .get_one::<std::path::PathBuf>("file")
            .map(|path| page_source(path))
            .transpose()?
            .unwrap_or_default();
        return cli_management::create_page(root, args, source);
    }
    let layout = Layout::existing(root)?.ok_or(Fault::Missing)?;
    let key = Keyring::read(&layout)?;
    let store = Store::read(&layout)?;
    store.require_current_schema()?;
    let id = cli_management::resolve_page(
        &store,
        &key,
        args.get_one::<String>("page").expect("required page"),
    )?;
    let source = if command == "write" {
        Some(page_source(
            args.get_one::<std::path::PathBuf>("file")
                .expect("required file"),
        )?)
    } else {
        None
    };
    let mut decoder = Decoder::new(std::env::current_exe()?)?;
    let json_output = args.get_flag("json");
    let mut output = tmt_cli_style::stream::stdout(json_output);
    if let Some(source) = source {
        let publisher_agent = core::publisher_agent();
        // Read once after the page snapshot to prepare, and again to commit.
        let clock = || -> tmt_colab::Result<u64> {
            Ok(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis()
                .try_into()?)
        };
        let preparation = page::prepare_publication_with_clock(
            &store,
            &key,
            &id,
            tmt_colab::decoder::ContentEdit {
                source: &source,
                publisher_agent: publisher_agent.as_deref(),
            },
            args.get_one::<String>("expected-revision")
                .map(String::as_str),
            &mut decoder,
            &clock,
        )?;
        store.close()?;
        let receipt = match preparation {
            page::PublicationPreparation::Noop {
                epoch,
                membership_head,
                base_revision,
                memory_limit,
            } => page::Receipt::unchanged(
                &key,
                &id,
                &source,
                epoch,
                membership_head,
                base_revision,
                memory_limit,
            ),
            page::PublicationPreparation::Write(frozen) => {
                let published = publish_write(&layout, &key, &id, &frozen, &mut decoder, clock()?)?;
                let mut receipt = page::publication_receipt(frozen.job(), &published.record)?;
                // A write's combine can move the revision after its outcome was retained. The
                // revision read by the writer that excluded every other writer is the one a next
                // `--expected-revision` must carry; a lost reply leaves the committed one.
                if let Some(revision) = published.revision {
                    receipt.revision = revision;
                }
                receipt
            }
        };
        if json_output {
            writeln!(output, "{}", serde_json::to_string(&receipt)?)?;
        } else {
            let terminal = output.terminal();
            tmt_cli_style::detail::write(
                &mut output,
                terminal,
                if receipt.changed {
                    "PAGE WRITTEN"
                } else {
                    "PAGE UNCHANGED"
                },
                &[
                    ("page", receipt.page_id),
                    ("revision", receipt.revision),
                    ("epoch", receipt.epoch),
                ],
            )?;
        }
    } else {
        let value = page::read(&store, &key, &id, &mut decoder);
        let closed = store.close();
        let value = value?;
        closed?;
        if json_output {
            writeln!(output, "{}", serde_json::to_string(&value)?)?;
        } else {
            let mut metadata = tmt_cli_style::stream::stderr();
            let terminal = metadata.terminal();
            tmt_cli_style::detail::write(
                &mut metadata,
                terminal,
                "PAGE SOURCE",
                &[
                    ("page", value.page_id),
                    ("revision", value.revision),
                    ("membership", value.membership_head.revision),
                    ("head", value.membership_head.statement_hash),
                    ("epoch", value.epoch),
                ],
            )?;
            output.write_all(value.source.as_bytes())?;
        }
    }
    Ok(())
}
/// One frozen publication reaches exactly one writer: the serve process when it holds the
/// lifecycle lock, otherwise this process under that lock. Doubt about a serving exchange is
/// resolved by original-operation status; the batch is never re-prepared, resent or committed
/// offline behind a running server.
fn publish_write(
    layout: &Layout,
    key: &Keyring,
    page_id: &str,
    frozen: &tmt_colab::page::FrozenPublication,
    decoder: &mut tmt_colab::decoder::Decoder,
    now: u64,
) -> Result<tmt_colab::page::Published> {
    use tmt_colab::{
        page::{self, OutcomeUnknown, Published},
        publication::Outcome,
        store::Accepted,
    };
    match layout.serve_lock() {
        Ok(_lock) => {
            let mut store = Store::write_existing(layout)?;
            let committed = page::commit_publication(
                &mut store,
                key,
                frozen.job(),
                frozen.packet(),
                frozen.chain(),
                now,
            );
            if let Ok(done) = &committed
                && done.accepted == Accepted::New
                && matches!(done.record.outcome, Outcome::Committed { .. })
                && let Err(error) = page::compact::compact(
                    &mut store,
                    key,
                    page_id,
                    decoder,
                    page::compact::Trigger::default(),
                )
            {
                // The write is durable; the next write tries to combine again.
                let mut stderr = tmt_cli_style::stream::stderr();
                let terminal = stderr.terminal();
                tmt_cli_style::message::warning(
                    &mut stderr,
                    terminal,
                    &format!("The page was written but could not be combined yet: {error}"),
                    None,
                )?;
            }
            // The serve lifecycle lock excludes every other writer, so this read is the revision
            // this write and its combine produced.
            let record = committed.as_ref().ok().map(|done| &done.record);
            let revision = record
                .filter(|record| matches!(record.outcome, Outcome::Committed { .. }))
                .and_then(|_| page::revision(&store, key, page_id).ok());
            let closed = store.close();
            let record = committed?.record;
            closed?;
            Ok(Published { record, revision })
        }
        Err(error)
            if error.downcast_ref::<tmt_colab::keyring::StateFault>()
                == Some(&tmt_colab::keyring::StateFault::AlreadyServing) =>
        {
            if let Some(published) = page::ipc::publish(layout, key, frozen)? {
                return Ok(published);
            }
            let original = frozen.job().key()?;
            let status = Store::read(layout).and_then(|store| {
                let status = page::publication_status(&store, key, &original, frozen.chain(), now);
                let closed = store.close();
                let status = status?;
                closed?;
                Ok(status)
            });
            match status {
                Ok(record) if !matches!(record.outcome, Outcome::Unknown { .. }) => Ok(Published {
                    record,
                    revision: None,
                }),
                _ => Err(OutcomeUnknown {
                    operation_id: original.operation_id,
                }
                .into()),
            }
        }
        Err(error) => Err(error),
    }
}
fn page_source(path: &std::path::Path) -> Result<String> {
    use nix::poll::{PollFd, PollFlags, poll};
    use std::{
        io::Read,
        os::fd::AsFd,
        os::unix::fs::OpenOptionsExt,
        time::{Duration, Instant},
    };
    use tmt_colab::{
        decoder::BASELINE_BYTES,
        page::{Fault, SourceTooLarge},
    };
    let mut bytes = Vec::new();
    if path == std::path::Path::new("-") {
        let deadline = Instant::now() + Duration::from_secs(5);
        let stdin = std::io::stdin();
        let input = stdin.lock();
        let mut chunk = [0; 4096];
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or(Fault::Invalid)?;
            let mut fds = [PollFd::new(input.as_fd(), PollFlags::POLLIN)];
            match poll(&mut fds, remaining.as_millis().min(u16::MAX as u128) as u16) {
                Ok(0) => return Err(Fault::Invalid.into()),
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => return Err(e.into()),
                Ok(_) => {}
            }
            let size = chunk.len().min(BASELINE_BYTES + 1 - bytes.len());
            let n = match nix::unistd::read(input.as_fd(), &mut chunk[..size]) {
                Ok(n) => n,
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => return Err(e.into()),
            };
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..n]);
            if bytes.len() > BASELINE_BYTES {
                return Err(SourceTooLarge::at_least(bytes.len(), BASELINE_BYTES).into());
            }
        }
    } else {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(nix::fcntl::OFlag::O_NONBLOCK.bits())
            .open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(Fault::Invalid.into());
        }
        Read::by_ref(&mut file)
            .take((BASELINE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > BASELINE_BYTES {
            let size = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
            return Err(SourceTooLarge::exact(size.max(bytes.len()), BASELINE_BYTES).into());
        }
    }
    String::from_utf8(bytes).map_err(|_| Fault::Invalid.into())
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
                .downcast_ref::<tmt_colab::control::StopFault>()
                .map(|e| e.code())
        })
        .or_else(|| {
            error
                .downcast_ref::<tmt_colab::page::ipc::WriteError>()
                .map(|e| e.code())
        })
        .or_else(|| {
            error
                .downcast_ref::<tmt_colab::page::Fault>()
                .map(|e| e.code())
        })
        .or_else(|| {
            error
                .downcast_ref::<tmt_colab::page::SourceTooLarge>()
                .map(|e| e.code())
        })
        .or_else(|| {
            error
                .downcast_ref::<tmt_colab::page::OutcomeUnknown>()
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
            matches!(
                error.downcast_ref::<tmt_colab::store::owner::OwnerFault>(),
                Some(tmt_colab::store::owner::OwnerFault::PageCapacity(_))
            )
            .then_some("COLAB_CAPACITY")
        })
        .or_else(|| {
            error
                .downcast_ref::<tmt_colab::assets::AssetFault>()
                .map(|_| "COLAB_APP_UNAVAILABLE")
        })
        .or_else(|| schema_fault(error).and_then(tmt_colab::store::Fault::code))
        .unwrap_or("COLAB_UNAVAILABLE")
}
/// Correlation adapters preserve the source so schema recovery remains typed.
fn schema_fault<'a>(
    error: &'a (dyn std::error::Error + 'static),
) -> Option<&'a tmt_colab::store::Fault> {
    let mut cause = Some(error);
    while let Some(error) = cause {
        if let Some(fault) = error.downcast_ref::<tmt_colab::store::Fault>() {
            return Some(fault);
        }
        cause = error.source();
    }
    None
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
            let help = matches!(
                e.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            );
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
            let schema = schema_fault(error.as_ref());
            let next = schema.and_then(tmt_colab::store::Fault::next);
            let hint = schema.and_then(tmt_colab::store::Fault::hint);
            if json_output {
                let mut value = cli_failure
                    .map(|e| e.correlation.clone())
                    .unwrap_or_else(|| json!({}));
                value["error"] = json!({"code":code,"message":error.to_string()});
                if let Some((stored, supported)) =
                    schema.and_then(tmt_colab::store::Fault::schema_versions)
                {
                    value["error"]["storeSchema"] = json!(stored);
                    value["error"]["supportedSchema"] = json!(supported);
                }
                if let Some(next) = next {
                    value["next"] = json!([next]);
                }
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
                    tmt_cli_style::message::error(&mut output, terminal, &error.to_string(), hint);
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_restart_hint_parses_without_execution() {
        let fault = tmt_colab::store::Fault::OutdatedSchema(4);
        let hint = fault.hint().unwrap();
        // Explicit command boundaries in the actual presentation-owned hint.
        let command = hint
            .strip_prefix("Start or restart ")
            .unwrap()
            .strip_suffix(" to update it.")
            .unwrap();
        assert_eq!(fault.next(), Some(command));
        let words = Example {
            command,
            note: "Restart the migration-owning serve",
        }
        .argv()
        .unwrap();
        assert_eq!(&words[..2], &["tmt", "colab"]);
        grammar()
            .try_get_matches_from(
                std::iter::once("colab").chain(words[2..].iter().map(String::as_str)),
            )
            .unwrap();
        // Upgrade is a core-owned command, covered by its existing printed-command guard.
        assert_eq!(
            tmt_colab::store::Fault::UnsupportedSchema(99).hint(),
            Some("Run tmt upgrade, then try again.")
        );
        assert_eq!(
            tmt_colab::store::Fault::UnsupportedSchema(99).next(),
            Some("tmt upgrade")
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
        assert!(
            tmt_cli_style::audit::list_spelling_report(&grammar(), &["tmt", "colab"]).is_empty()
        );
    }
}
