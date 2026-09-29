use clap::{Arg, ArgAction, Command};
use tmt_cli_style::CommandSpec;

/// A command's help: its summary and one to three examples, which the
/// grammar walk parses (docs/cli-style.md#help).
macro_rules! spec {
    ($name:literal, $summary:literal, [$($note:literal => $command:literal),+ $(,)?]) => {
        &tmt_cli_style::CommandSpec {
            name: $name,
            summary: $summary,
            examples: &[$(tmt_cli_style::Example {
                command: $command,
                note: $note,
            }),+],
            outputs: tmt_cli_style::OutputModes::Human,
            details: "",
        }
    };
    ($name:literal, $summary:literal, details = $details:literal, [$($note:literal => $command:literal),+ $(,)?]) => {
        &tmt_cli_style::CommandSpec {
            details: $details,
            ..*spec!($name, $summary, [$($note => $command),+])
        }
    };
}

pub mod completion;
pub mod extensions;

/// The root's help; root help adds the extensions discovered on `PATH`.
pub const ROOT: &CommandSpec = spec!(
    "tmt",
    "TMT native alpha: collaborate with terminal agents through durable exchanges",
    [
        "Set up agent skills" => "tmt install",
        "Ask an agent and wait for its reply" => "tmt talk worker \"Run the tests\"",
        "Update a managed installation" => "tmt upgrade",
    ]
);

// This tree owns recognition, help, completion and allowed-option validation.
// Root-recognized options are inherited for placement, not universal permission.
pub fn grammar() -> Command {
    let mut root = base(ROOT);
    for id in [
        "json",
        "force",
        "config",
        "delay",
        "wait",
        "detach",
        "timeout",
        "lines",
        "no-preamble",
        "team",
        "help",
        "version",
    ] {
        root = root.arg(option(id).global(true));
    }
    root = root.subcommand(crate::office_facade::grammar::grammar().after_help(
        "To install, update or remove any official extension, see: tmt extension --help",
    ));
    root = root.subcommand(storage(spec!(
        "api",
        "Versioned JSON extension interface (one request on stdin)",
        [
            "Answer one JSON request read from stdin" => "tmt api",
        ]
    )));
    root = root.subcommand(
        storage(spec!(
            "room",
            "Manage shared communication rooms (not access controls)",
            [
                "Create a room" => "tmt room create reviewers",
                "Ask every member of a room" => "tmt room send reviewers \"Please review PR 444\"",
            ]
        ))
        .subcommand_required(true)
        .subcommand(storage(spec!(
            "create",
            "Create an empty room",
            [
                "Create a room" => "tmt room create reviewers",
            ]
        )).arg(operand("name", true)))
        .subcommand(storage(spec!(
            "list",
            "List rooms and membership counts",
            [
                "List rooms and their member counts" => "tmt room list",
                "The same, as JSON" => "tmt room list --json",
            ]
        )).visible_alias("ls"))
        .subcommand(
            storage(spec!(
                "retire",
                "Stop new room work while retaining content and history",
                [
                    "Retire a room, keeping its history" => "tmt room retire reviewers",
                ]
            ))
            .arg(operand("room", true)),
        )
        .subcommand(
            storage(spec!(
                "show",
                "Show a room by UUID or unique exact name",
                [
                    "Show a room and its members" => "tmt room show reviewers",
                ]
            )).arg(operand("room", true)),
        )
        .subcommand(
            with_options(
                storage(spec!(
                    "join",
                    "Join a room without changing other members",
                    [
                        "Join a room as this pane's identity" => "tmt room join reviewers",
                        "Add another identity" => "tmt room join reviewers --identity worker",
                    ]
                )),
                &["identity"],
            )
            .arg(operand("room", true)),
        )
        .subcommand(
            with_options(
                storage(spec!(
                    "leave",
                    "Leave a room without deleting its history",
                    [
                        "Leave a room" => "tmt room leave reviewers",
                    ]
                )),
                &["identity"],
            )
            .arg(operand("room", true)),
        )
        .subcommands(
            [
                spec!(
                    "send",
                    "Queue one replyable inbox request per room member",
                    [
                        "Ask every member for a reply" => "tmt room send reviewers \"Please review PR 444\"",
                    ]
                ),
                spec!(
                    "broadcast",
                    "Queue a no-reply announcement per room member",
                    [
                        "Announce without expecting replies" => "tmt room broadcast reviewers \"Main is green again\"",
                    ]
                ),
            ]
            .into_iter()
            .map(|spec| {
                with_options(storage(spec), &["identity"])
                    .arg(operand("room", true))
                    .arg(operand("message", true))
                    .arg(
                        Arg::new("operation-id")
                            .long("operation-id")
                            .help("Reuse a UUID for safe retry of the same composition"),
                    )
            }),
        ),
    );
    root.subcommand(
        general(spec!(
            "help",
            "Show help for a command",
            [
                "Show help for a command" => "tmt help talk",
                "Show help for a subcommand" => "tmt help identity show",
            ]
        ))
            .arg(operand("command-path", false).num_args(0..)),
    )
    .subcommand(
        internal("team", "Retired command")
            .hide(true)
            .arg(operand("scope", false)),
    )
    .subcommand(general(spec!(
        "init",
        "Create workspace settings",
        [
            "Create settings for this workspace" => "tmt init",
        ]
    )))
    .subcommand(
        base(spec!(
            "run",
            "Bind this pane and run a command with its original arguments",
            [
                "Start an agent in this pane under a temporary name" => "tmt run worker claude",
                "Keep the identity after the pane is gone" => "tmt run --save worker claude",
                "Resume the remembered session" => "tmt run --resume worker",
            ]
        ))
            .arg(option("save"))
            .arg(Arg::new("resume").long("resume").action(ArgAction::SetTrue)
                .help("Resume the remembered session; put this option before the name"))
            .arg(Arg::new("run-argv").value_name("NAME [COMMAND...]")
                .required(true).num_args(1..).trailing_var_arg(true)
                .value_parser(clap::builder::OsStringValueParser::new())),
    )
    .subcommand(
        base(spec!(
            "resume",
            "Resume an identity's remembered session in this pane",
            [
                "Resume an identity's last session" => "tmt resume worker",
                "Forget the remembered session" => "tmt resume --forget worker",
            ]
        ))
            .arg(Arg::new("forget").long("forget").action(ArgAction::SetTrue)
                .help("Forget the remembered session instead of resuming it"))
            .arg(Arg::new("retry").long("retry").action(ArgAction::SetTrue)
                .conflicts_with("forget")
                .help("Try a session marked stale once more"))
            .arg(operand("name", true).help("Identity name; TMT options go before it")),
    )
    .subcommand(
        general(spec!(
            "list",
            "List global identities, lifetime and live presence",
            [
                "List identities and whether they are live" => "tmt list",
                "List the members of a room" => "tmt list --room reviewers",
                "The same, as JSON" => "tmt list --json",
            ]
        ))
            .visible_alias("ls")
            .arg(operand("target", false).conflicts_with("room"))
            .arg(option("room"))
            .arg(list_filter("saved", "Only saved identities").conflicts_with("temp"))
            .arg(list_filter("temp", "Only temporary identities"))
            .arg(list_filter("here", "Only agents in this tmux session"))
            .arg(list_filter("all", "Show offline identities one per row")),
    )
    .subcommand(
        with_options(
            general(spec!(
                "add",
                "Bind an explicit pane; temporary unless saved",
                [
                    "Bind a pane by its tmux id" => "tmt add %3 worker",
                    "Keep the identity after the pane is gone" => "tmt add --save %3 worker",
                ]
            )),
            &["save"],
        )
        .arg(operand("pane-target", true))
        .arg(operand("name", true)),
    )
    .subcommand(
        with_options(
            general(spec!(
                "name",
                "Bind this pane; temporary unless saved",
                [
                    "Name this pane" => "tmt name worker",
                    "Keep the identity after the pane is gone" => "tmt name --save worker",
                ]
            )),
            &["save"],
        )
        .visible_alias("this")
        .arg(operand("name", true)),
    )
    .subcommand(
        with_options(
            general(spec!(
                "marked",
                "Bind the pane explicitly marked in tmux; temporary unless saved",
                [
                    "Bind the pane you marked in tmux" => "tmt marked worker",
                ]
            )),
            &["save"],
        )
        .arg(operand("name", true)),
    )
    .subcommand(
        with_options(
            general(spec!(
                "rm",
                "Retire identity and remove role/preamble; keep pane/exchanges (--force for saved)",
                [
                    "Retire a temporary identity" => "tmt rm worker",
                    "Retire a saved identity" => "tmt rm --force worker",
                ]
            )),
            &["force"],
        )
        .visible_alias("remove")
        .arg(operand("name", true)),
    )
    .subcommand(
        with_options(
            general(spec!(
                "talk",
                "Send a request and wait for its durable reply",
                [
                    "Send a message and wait for the reply" => "tmt talk worker \"Run the tests\"",
                    "Send and return at once" => "tmt talk --detach worker \"Deploy when green\"",
                    "Queue for an identity with no pane" => "tmt talk --inbox worker \"Review when free\"",
                ]
            )),
            &[
                "force",
                "delay",
                "detach",
                "timeout",
                "no-preamble",
                "identity",
                "inbox",
                "room",
            ],
        )
        .visible_alias("send")
        .arg(operand("target", true))
        .arg(operand("message", true)),
    )
    .subcommand(
        with_options(
            general(spec!(
                "check",
                "Capture diagnostic pane output",
                [
                    "Show recent output from an agent's pane" => "tmt check worker",
                    "Show the last 50 lines" => "tmt check worker 50",
                ]
            )),
            &["lines"],
        )
        .visible_alias("read")
        .arg(operand("target", true))
        .arg(operand("capture-lines", false)),
    )
    .subcommand(
        general(spec!(
            "focus",
            "Show an identity's or pane's view in your tmux client",
            [
                "Show an agent's pane in your tmux client" => "tmt focus worker",
                "Name your client and the pane it shows" => "tmt focus --client",
            ]
        ))
            .arg(operand("target", false).required_unless_present("client"))
            .arg(
                Arg::new("client")
                    .long("client")
                    .action(ArgAction::SetTrue)
                    .conflicts_with("target")
                    .help("Name your tmux client and the pane it shows; takes no target, switches nothing"),
            ),
    )
    .subcommand(
        general(spec!(
            "whoami",
            "Show this pane's verified identity",
            [
                "Show this pane's identity" => "tmt whoami",
                "Include pending work" => "tmt whoami --context",
            ]
        )).arg(
            Arg::new("context").long("context").action(ArgAction::SetTrue)
                .help("Read bounded identity and pending-work context without changing state"),
        ),
    )
    .subcommand(general(spec!(
        "unbind",
        "Detach this pane; retire temporary identity",
        [
            "Detach this pane from its identity" => "tmt unbind",
        ]
    )))
    .subcommand(
        general(spec!(
            "extension",
            "Manage consented extension integrations",
            [
                "List official extensions" => "tmt extension list",
                "Install Squad" => "tmt extension install squad",
            ]
        ))
            .subcommand_required(true)
            .subcommand(
                general(spec!(
                    "hooks",
                    "Manage lifecycle hooks for trusted extensions",
                    [
                        "List extensions with hooks enabled" => "tmt extension hooks list",
                        "Deliver lifecycle hooks to Squad" => "tmt extension hooks enable squad",
                    ]
                ))
                    .subcommand_required(true)
                    .subcommand(
                        general(spec!(
                            "enable",
                            "Trust tmt-<name> on PATH to receive lifecycle observations",
                            [
                                "Deliver lifecycle hooks to Squad" => "tmt extension hooks enable squad",
                            ]
                        ))
                        .arg(operand("name", true)),
                    )
                    .subcommand(
                        general(spec!(
                            "disable",
                            "Stop delivering hooks to an extension",
                            [
                                "Stop hooks for Squad" => "tmt extension hooks disable squad",
                            ]
                        ))
                            .arg(operand("name", true)),
                    )
                    .subcommand(general(spec!(
                        "list",
                        "List extensions with enabled hooks",
                        [
                            "List extensions with hooks enabled" => "tmt extension hooks list",
                        ]
                    ))),
            )
            .subcommand(
                extension_target(general(spec!(
                    "install",
                    "Install an official extension (office, squad)",
                    [
                        "Install Squad" => "tmt extension install squad",
                        "Install without a prompt" => "tmt extension install squad --yes",
                    ]
                )))
                    .arg(channel_option())
                    .arg(Arg::new("archive").long("archive").requires("manifest"))
                    .arg(Arg::new("manifest").long("manifest").requires("archive")),
            )
            .subcommand(
                extension_target(general(spec!(
                    "upgrade",
                    "Update an installed official extension",
                    [
                        "Update Squad" => "tmt extension upgrade squad",
                        "Install an exact version" => "tmt extension upgrade squad --to 0.1.0-alpha.2",
                    ]
                )))
                    .arg(channel_option())
                    .arg(Arg::new("to").long("to").conflicts_with("unpin"))
                    .arg(Arg::new("unpin").long("unpin").action(ArgAction::SetTrue)),
            )
            .subcommand(extension_target(general(spec!(
                "uninstall",
                "Remove an extension's commands; releases and data are kept",
                [
                    "Remove Squad's commands" => "tmt extension uninstall squad",
                ]
            ))))
            .subcommand(
                general(spec!(
                    "list",
                    "List official extensions, versions and PATH shadowing",
                    [
                        "List official extensions" => "tmt extension list",
                        "Also check for newer releases" => "tmt extension list --check",
                    ]
                ))
                    .arg(Arg::new("prefix").long("prefix"))
                    .arg(
                        Arg::new("check")
                            .long("check")
                            .action(ArgAction::SetTrue)
                            .help("Also check for a newer release (uses the network)"),
                    ),
            ),
    )
    .subcommand(
        general(spec!(
            "config",
            "View or modify settings",
            [
                "Show settings" => "tmt config show",
                "Change a setting" => "tmt config set timeout 120",
            ]
        ))
            .subcommand(general(spec!(
                "show",
                "Show settings",
                [
                    "Show settings and where each comes from" => "tmt config show",
                ]
            )))
            .subcommand(
                with_options(general(spec!(
                    "set",
                    "Set a setting",
                    [
                        "Change a workspace setting" => "tmt config set timeout 120",
                        "Change a global setting" => "tmt config set --global captureLines 200",
                    ]
                )), &["global"])
                    .arg(operand("key", true))
                    .arg(operand("value", true).allow_negative_numbers(true)),
            )
            .subcommand(general(spec!(
                "clear",
                "Clear a local setting",
                [
                    "Clear a workspace setting" => "tmt config clear timeout",
                ]
            )).arg(operand("key", false))),
    )
    .subcommand(
        general(spec!(
            "preamble",
            "Manage identity-owned preambles",
            [
                "Set an agent's preamble" => "tmt preamble set worker \"Answer in one paragraph\"",
                "Show preambles" => "tmt preamble show",
            ]
        ))
            .subcommand(general(spec!(
                "show",
                "Show preambles",
                [
                    "Show every preamble" => "tmt preamble show",
                    "Show one agent's preamble" => "tmt preamble show worker",
                ]
            )).arg(operand("agent", false)))
            .subcommand(
                general(spec!(
                    "set",
                    "Set a preamble",
                    [
                        "Set an agent's preamble" => "tmt preamble set worker \"Answer in one paragraph\"",
                    ]
                ))
                    .arg(operand("agent", true))
                    .arg(operand("content", true).num_args(1..)),
            )
            .subcommand(general(spec!(
                "clear",
                "Clear a preamble",
                [
                    "Clear an agent's preamble" => "tmt preamble clear worker",
                ]
            )).arg(operand("agent", true))),
    )
    .subcommand(
        with_options(
            storage(spec!(
                "x",
                "Inspect and acknowledge exchanges",
                [
                    "List unacknowledged exchanges" => "tmt x list",
                    "Wait for incoming inbox work" => "tmt x listen",
                ]
            )),
            &["identity", "limit", "after"],
        )
        .subcommand(with_options(
            storage(spec!(
                "list",
                "List unacknowledged exchanges",
                [
                    "List unacknowledged exchanges" => "tmt x list",
                    "Show at most 20" => "tmt x list --limit 20",
                ]
            )),
            &["identity", "limit", "after"],
        ))
        .subcommand(
            with_options(
                storage(spec!(
                    "show",
                    "Show retained exchange content",
                    [
                        "Show one exchange" => "tmt x show req_0f8e4b52-3c1d-4a6e-9b7f-2d5c8a1e6f30",
                    ]
                )),
                &["identity", "incoming"],
            )
            .arg(operand("request-id", true)),
        )
        .subcommand(
            with_options(
                storage(spec!(
                    "ack",
                    "Acknowledge an observed revision",
                    [
                        "Acknowledge the revision you read" => "tmt x ack --revision 3 req_0f8e4b52-3c1d-4a6e-9b7f-2d5c8a1e6f30",
                    ]
                )),
                &["identity", "incoming"],
            )
            .arg(option("revision").required(true))
            .arg(operand("request-id", true)),
        )
        .subcommand(with_options(
            storage(spec!(
                "ackall",
                "Acknowledge the current identity snapshot",
                [
                    "Acknowledge everything shown so far" => "tmt x ackall",
                ]
            )),
            &["identity", "incoming"],
        ))
        .subcommand(with_options(
            storage(spec!(
                "listen",
                "Wait for incoming inbox activity",
                [
                    "Wait for new inbox work" => "tmt x listen",
                    "Give up after ten minutes" => "tmt x listen --timeout 10m",
                ]
            )),
            &["identity", "room", "timeout", "debounce"],
        )),
    )
    .subcommand(
        storage(spec!(
            "identity",
            "Manage identity records, metadata and self-reported status",
            [
                "Create a saved identity" => "tmt identity create reviewer",
                "Show an identity" => "tmt identity show reviewer",
            ]
        ))
        .subcommand_required(true)
        .subcommand(storage(spec!(
            "create",
            "Create or save an identity",
            [
                "Create a saved identity" => "tmt identity create reviewer",
            ]
        )).arg(operand("name", true)))
        .subcommand(storage(spec!(
            "show",
            "Show an identity by name or UUID, or the verified caller",
            [
                "Show an identity" => "tmt identity show reviewer",
                "Show this pane's identity" => "tmt identity show",
            ]
        )).arg(operand("name", false)))
        .subcommand(with_options(
            storage(spec!(
                "list",
                "List non-retired identities",
                [
                    "List identities" => "tmt identity list",
                    "Only those with matching metadata" => "tmt identity list --where team=infra",
                ]
            )),
            &["where", "has"],
        ))
        .subcommand(
            storage(spec!(
                "status",
                "Manage expiring self-reported activity, not endpoint presence",
                [
                    "Report what you are doing" => "tmt identity status set \"Reviewing PR 444\"",
                    "Show your status" => "tmt identity status show",
                ]
            ))
            .subcommand_required(true)
            .subcommand(with_options(
                storage(spec!(
                    "show",
                    "Show current or stale status",
                    [
                        "Show your status" => "tmt identity status show",
                        "Show another identity's status" => "tmt identity status show --identity reviewer",
                    ]
                )),
                &["identity"],
            ))
            .subcommand(
                with_options(
                    storage(spec!(
                        "set",
                        "Replace status and renew its expiry",
                        [
                            "Report what you are doing" => "tmt identity status set \"Reviewing PR 444\"",
                            "For thirty minutes" => "tmt identity status set \"Running the suite\" --for 30m",
                        ]
                    )),
                    &["identity", "mood", "for"],
                )
                .arg(operand("activity", true)),
            )
            .subcommand(with_options(
                storage(spec!(
                    "clear",
                    "Clear this identity's self-reported status",
                    [
                        "Clear your status" => "tmt identity status clear",
                    ]
                )),
                &["identity"],
            )),
        )
        .subcommand(
            storage(spec!(
                "meta",
                "Manage descriptive identity metadata",
                [
                    "Set a metadata value" => "tmt identity meta set team infra",
                    "List metadata" => "tmt identity meta list",
                ]
            ))
                .subcommand_required(true)
                .subcommand(
                    with_options(storage(spec!(
                        "set",
                        "Set a metadata value",
                        [
                            "Set a metadata value" => "tmt identity meta set team infra",
                        ]
                    )), &["identity"])
                        .arg(operand("key", true))
                        .arg(operand("value", true)),
                )
                .subcommand(
                    with_options(storage(spec!(
                        "get",
                        "Get a metadata value",
                        [
                            "Read one value" => "tmt identity meta get team",
                        ]
                    )), &["identity"])
                        .arg(operand("key", true)),
                )
                .subcommand(with_options(
                    storage(spec!(
                        "list",
                        "List metadata values",
                        [
                            "List metadata" => "tmt identity meta list",
                            "For another identity" => "tmt identity meta list --identity reviewer",
                        ]
                    )),
                    &["identity"],
                ))
                .subcommand(
                    with_options(storage(spec!(
                        "rm",
                        "Remove a metadata value",
                        [
                            "Remove a value" => "tmt identity meta rm team",
                        ]
                    )), &["identity"])
                        .arg(operand("key", true)),
                ),
        ),
    )
    .subcommand(
        storage(spec!(
            "notes",
            "Access saved identity notes",
            [
                "Print the path of your notes file" => "tmt notes path",
            ]
        ))
            .subcommand_required(true)
            .subcommand(with_options(
                storage(spec!(
                    "path",
                    "Initialize and print the local Markdown path",
                    details = "Saved identities only. Edit the returned Markdown file directly; an Office\nnotebook object reads this same file. Office reads never create missing notes.",
                    [
                        "Print the path of your notes file" => "tmt notes path",
                        "For another saved identity" => "tmt notes path --identity reviewer",
                    ]
                )),
                &["identity"],
            )),
    )
    .subcommand(
        with_options(general(spec!(
            "role",
            "Manage role profiles",
            [
                "Set your role" => "tmt role set \"Reviews Rust changes\"",
                "Show your role" => "tmt role show",
            ]
        )), &["identity"])
            .subcommand_required(true)
            .subcommand(with_options(general(spec!(
                "show",
                "Show a role",
                [
                    "Show your role" => "tmt role show",
                    "Show another identity's role" => "tmt role show --identity reviewer",
                ]
            )), &["identity"]))
            .subcommand(
                with_options(
                    general(spec!(
                        "set",
                        "Set a role from inline text or file",
                        [
                            "Set your role" => "tmt role set \"Reviews Rust changes\"",
                            "Read it from a file" => "tmt role set --file role.md",
                        ]
                    )),
                    &["identity", "file"],
                )
                .arg(operand("content", false)),
            )
            .subcommand(with_options(
                general(spec!(
                    "clear",
                    "Clear a role",
                    [
                        "Clear your role" => "tmt role clear",
                    ]
                )),
                &["identity"],
            )),
    )
    .subcommand(
        with_options(
            storage(spec!(
                "reply",
                "Submit an exact final response",
                [
                    "Reply with a message" => "tmt reply req_0f8e4b52-3c1d-4a6e-9b7f-2d5c8a1e6f30 --receipt v2_7OxG2uwfdAFFMd0qNWJDnA --message \"Done: tests pass\"",
                    "Reply with a file's content" => "tmt reply req_0f8e4b52-3c1d-4a6e-9b7f-2d5c8a1e6f30 --receipt v2_7OxG2uwfdAFFMd0qNWJDnA --file answer.md",
                ]
            )),
            &["file", "message", "stdin"],
        )
        .arg(option("receipt").required(true))
        .arg(operand("request-id", true)),
    )
    .subcommand(
        storage(spec!(
            "result",
            "Retrieve a retained final response",
            [
                "Print a request's final response" => "tmt result req_0f8e4b52-3c1d-4a6e-9b7f-2d5c8a1e6f30",
            ]
        )).arg(operand("request-id", true)),
    )
    .subcommand(
        with_options(
            general(spec!(
                "install",
                "Install or refresh agent skills",
                [
                    "Install skills for every agent" => "tmt install",
                    "Only for Claude" => "tmt install claude",
                ]
            )),
            &["force", "dir"],
        )
        .arg(
            operand("agent", false)
                .value_parser(
                    tmt_core::skill_provider::Provider::ALL
                        .into_iter()
                        .map(|provider| provider.as_str())
                        .chain(["all"])
                        .collect::<Vec<_>>(),
                )
                .ignore_case(true),
        ),
    )
    .subcommand(general(spec!(
        "setup",
        "Inspect or consent to agent lifecycle integration",
        [
            "Show lifecycle integration status" => "tmt setup",
            "Enable it for Claude" => "tmt setup claude --yes",
            "Remove it for Claude" => "tmt setup claude --remove",
        ]
    ))
        .arg(operand("provider", false).value_parser(["claude", "codex"]))
        .arg(Arg::new("remove").long("remove").action(ArgAction::SetTrue).requires("provider"))
        .arg(Arg::new("yes").long("yes").action(ArgAction::SetTrue).requires("provider")))
    .subcommand(internal("__hook", "Internal bounded provider lifecycle callback").hide(true)
        .arg(operand("provider", true).value_parser(["claude", "codex"]))
        .arg(Arg::new("worker").long("worker").hide(true).action(ArgAction::SetTrue)))
    .subcommand(internal("__request-observer", "Internal bounded request timeout observer").hide(true)
        .arg(operand("request-id", true)))
    .subcommand(general(spec!(
        "completion",
        "Generate shell completion",
        [
            "Print the zsh completion script" => "tmt completion zsh",
        ]
    )).arg(operand("shell", false)))
    .subcommand(internal("__complete", "Internal shell completion context").hide(true)
        .arg(Arg::new("words").num_args(0..).trailing_var_arg(true)
            .value_parser(clap::builder::OsStringValueParser::new())))
    .subcommand(
        general(spec!(
            "upgrade",
            "Upgrade the native CLI and refresh managed skills",
            [
                "Update to the latest release on your channel" => "tmt upgrade",
                "Switch to the stable channel" => "tmt upgrade --channel stable",
                "Install an exact version" => "tmt upgrade --to 5.0.0-alpha.8",
            ]
        ))
        .visible_alias("update")
        .arg(
            Arg::new("channel").long("channel").value_parser(
                tmt_core::native_install::Channel::ALL.map(|channel| channel.as_str()),
            ),
        )
        .arg(Arg::new("to").long("to").conflicts_with("unpin"))
        .arg(Arg::new("unpin").long("unpin").action(ArgAction::SetTrue)),
    )
    .subcommand(internal("__native-refresh-skills", "Internal managed skill refresh").hide(true))
    .subcommand(
        internal("__native-install", "Internal offline native installation")
            .hide(true)
            .arg(
                Arg::new("product")
                    .long("product")
                    .default_value("cli")
                    .value_parser(
                        tmt_core::native_install::Product::ALL.map(|product| product.as_str()),
                    ),
            )
            .arg(Arg::new("archive").long("archive").required(true))
            .arg(Arg::new("manifest").long("manifest").required(true))
            .arg(Arg::new("prefix").long("prefix").required(true))
            .arg(
                Arg::new("channel")
                    .long("channel")
                    .required(true)
                    .value_parser(
                        tmt_core::native_install::Channel::ALL.map(|channel| channel.as_str()),
                    ),
            )
            .arg(
                Arg::new("pin")
                    .long("pin")
                    .action(ArgAction::SetTrue)
                    .conflicts_with("unpin"),
            )
            .arg(Arg::new("unpin").long("unpin").action(ArgAction::SetTrue)),
    )
    .subcommand(with_options(
        general(spec!(
            "learn",
            "Read agent guidance",
            [
                "Read the tmux-team guidance" => "tmt learn",
                "Print a bundled skill" => "tmt learn --skill tmt-inbox",
            ]
        )),
        &["skill"],
    ))
}

fn bare(name: &'static str) -> Command {
    Command::new(name)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .args_override_self(true)
}

fn base(spec: &CommandSpec) -> Command {
    tmt_cli_style::apply(bare(spec.name), spec, &[])
}

fn storage(spec: &CommandSpec) -> Command {
    base(spec).arg(option("json"))
}

fn general(spec: &CommandSpec) -> Command {
    with_options(storage(spec), &["wait", "team"])
}

/// Hidden internal entry points: no help page, so no examples.
fn internal(name: &'static str, about: &'static str) -> Command {
    with_options(
        bare(name).about(about).arg(option("json")),
        &["wait", "team"],
    )
}

fn with_options(mut command: Command, ids: &[&'static str]) -> Command {
    for id in ids {
        command = command.arg(option(id));
    }
    command
}

/// Consented extension installation: the extension name, --yes and --prefix.
fn extension_target(command: Command) -> Command {
    command
        .arg(operand("name", true))
        .arg(Arg::new("yes").long("yes").action(ArgAction::SetTrue))
        .arg(Arg::new("prefix").long("prefix"))
}

fn channel_option() -> Arg {
    Arg::new("channel")
        .long("channel")
        .value_parser(tmt_core::native_install::Channel::ALL.map(|channel| channel.as_str()))
}

/// A `tmt ls` filter; filters narrow the list, never a single identity.
fn list_filter(id: &'static str, help: &'static str) -> Arg {
    Arg::new(id)
        .long(id)
        .help(help)
        .action(ArgAction::SetTrue)
        .conflicts_with("target")
}

fn operand(id: &'static str, required: bool) -> Arg {
    Arg::new(id).required(required)
}

pub fn root_allowed(id: &str) -> bool {
    matches!(id, "json" | "help" | "version" | "team")
}

/// Completion generators include hidden and inherited arguments. Project only
/// advertised, command-owned arguments so rejection-only syntax is never offered.
pub fn public_grammar(definition: &Command, root: bool) -> Command {
    let mut result = tmt_cli_style::frame(Command::new(definition.get_name().to_owned()))
        .subcommand_required(definition.is_subcommand_required_set())
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .visible_aliases(definition.get_visible_aliases().map(str::to_owned));
    if let Some(about) = definition.get_about() {
        result = result.about(about.clone());
    }
    if let Some(after_help) = definition.get_after_help() {
        result = result.after_help(after_help.clone());
    }
    for argument in definition.get_arguments() {
        if !argument.is_hide_set() && (!root || root_allowed(argument.get_id().as_str())) {
            result = result.arg(argument.clone().global(false));
        }
    }
    for child in definition
        .get_subcommands()
        .filter(|child| !child.is_hide_set())
    {
        result = result.subcommand(public_grammar(child, false));
    }
    for group in definition.get_groups() {
        if group.get_args().next().is_some()
            && group.get_args().all(|id| {
                result
                    .get_arguments()
                    .any(|argument| argument.get_id() == id)
            })
        {
            result = result.group(group.clone());
        }
    }
    result.arg(option("help").hide(false).global(false))
}

/// Resolve public command paths without dispatching any runtime operation.
pub fn help_command(path: &[String]) -> Result<Command, String> {
    let definition = grammar();
    let mut selected = &definition;
    let mut names = vec!["tmt".to_owned()];
    for name in path {
        selected = selected
            .find_subcommand(name)
            .filter(|command| !command.is_hide_set())
            .ok_or_else(|| format!("Unknown command '{name}' after {}.", names.join(" ")))?;
        names.push(selected.get_name().to_owned());
    }
    Ok(public_grammar(selected, path.is_empty()).bin_name(names.join(" ")))
}

fn option(id: &'static str) -> Arg {
    let flag = |description: &'static str| {
        Arg::new(id)
            .long(id)
            .help(description)
            .action(ArgAction::SetTrue)
    };
    let value = |description: &'static str| {
        Arg::new(id)
            .long(id)
            .help(description)
            .action(ArgAction::Set)
            .allow_hyphen_values(matches!(
                id,
                "config" | "delay" | "timeout" | "lines" | "team"
            ))
    };
    match id {
        "json" => flag("Output one JSON document"),
        "force" => {
            flag("Authorize the command's documented protected replacement or removal").short('f')
        }
        "save" => flag("Preserve this identity after its pane is gone").short('s'),
        "help" => flag("Show help").short('h').hide(true),
        "version" => flag("Show version").short('V').hide(true),
        "wait" => flag("Retired; use timeout or detach").hide(true),
        "detach" => flag("Return after sending"),
        "inbox" => flag("Queue for an identity without tmux delivery"),
        "incoming" => flag("Use recipient-facing request attention"),
        "no-preamble" => flag("Skip the recipient preamble"),
        "stdin" => flag("Read complete input through EOF"),
        "skill" => Arg::new(id)
            .long(id)
            .help("Print an exact bundled skill (default: tmux-team)")
            .num_args(0..=1)
            .default_missing_value("tmux-team")
            .value_parser([
                "tmux-team",
                "tmt-inbox",
                "tmt-office",
                "tmt-prop-create",
                "tmt-avatar-create",
            ]),
        "global" => flag("Edit global settings").short('g'),
        "config" => value("Unsupported path override").hide(true),
        "team" => value("Retired scope").hide(true),
        "timeout" => value("Observer timeout in seconds or with ms/s/m suffix"),
        "delay" => value("Pre-send delay in seconds or with ms/s/m suffix"),
        "debounce" => value("Trailing quiet interval in seconds or with ms/s/m suffix"),
        "mood" => value("Optional self-reported mood, up to 32 UTF-8 bytes"),
        "for" => value("Status duration: seconds or ms/s/m; default 60m, range 1s to 1440m"),
        "lines" => value("Diagnostic capture line count"),
        "identity" => value("Select an explicit identity").global(true),
        "room" => value("Restrict to a room UUID or unique exact name"),
        "file" => value("Read content from a regular file"),
        "message" => value("Submit exact inline content"),
        "receipt" => value("Receipt supplied by talk"),
        "dir" => value("Custom skills directory"),
        "limit" => value("Maximum exchanges, 1 through 200").global(true),
        "after" => value("List revisions after this cursor").global(true),
        "revision" => value("Observed revision to acknowledge"),
        "where" => Arg::new(id)
            .long(id)
            .help("Require exact metadata KEY=VALUE")
            .action(ArgAction::Append),
        "has" => Arg::new(id)
            .long(id)
            .help("Require a metadata KEY")
            .action(ArgAction::Append),
        _ => unreachable!("unknown grammar option"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar_is_internally_consistent() {
        grammar().debug_assert();
    }
}
