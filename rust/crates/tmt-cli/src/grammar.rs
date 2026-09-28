use clap::{Arg, ArgAction, Command};

pub mod completion;
pub mod extensions;

// This tree owns recognition, help, completion and allowed-option validation.
// Root-recognized options are inherited for placement, not universal permission.
pub fn grammar() -> Command {
    let mut root = Command::new("tmt")
        .about("Collaborate with agents through durable tmux exchanges")
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .args_override_self(true);
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
    root = root.subcommand(office_commands());
    root = root.subcommand(storage(
        "api",
        "Versioned JSON extension interface (one request on stdin)",
    ));
    root = root.subcommand(
        storage(
            "room",
            "Manage shared communication rooms (not access controls)",
        )
        .subcommand_required(true)
        .subcommand(storage("create", "Create an empty room").arg(operand("name", true)))
        .subcommand(storage("list", "List rooms and membership counts").visible_alias("ls"))
        .subcommand(
            storage(
                "retire",
                "Stop new room work while retaining content and history",
            )
            .arg(operand("room", true)),
        )
        .subcommand(
            storage("show", "Show a room by UUID or unique exact name").arg(operand("room", true)),
        )
        .subcommand(
            with_options(
                storage("join", "Join a room without changing other members"),
                &["identity"],
            )
            .arg(operand("room", true)),
        )
        .subcommand(
            with_options(
                storage("leave", "Leave a room without deleting its history"),
                &["identity"],
            )
            .arg(operand("room", true)),
        )
        .subcommands(
            [
                ("send", "Queue one replyable inbox request per room member"),
                ("broadcast", "Queue a no-reply announcement per room member"),
            ]
            .into_iter()
            .map(|(name, about)| {
                with_options(storage(name, about), &["identity"])
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
        general("help", "Show help for a command")
            .arg(operand("command-path", false).num_args(0..)),
    )
    .subcommand(
        general("team", "Retired command")
            .hide(true)
            .arg(operand("scope", false)),
    )
    .subcommand(general("init", "Create workspace settings"))
    .subcommand(
        base("run", "Bind this pane and run a command with its original arguments")
            .arg(option("save"))
            .arg(Arg::new("resume").long("resume").action(ArgAction::SetTrue)
                .help("Resume the remembered session; put this option before the name"))
            .arg(Arg::new("run-argv").value_name("NAME [COMMAND...]")
                .required(true).num_args(1..).trailing_var_arg(true)
                .value_parser(clap::builder::OsStringValueParser::new())),
    )
    .subcommand(
        general("list", "List global identities, lifetime and live presence")
            .visible_alias("ls")
            .arg(operand("target", false).conflicts_with("room"))
            .arg(option("room")),
    )
    .subcommand(
        with_options(
            general("add", "Bind an explicit pane; temporary unless saved"),
            &["save"],
        )
        .arg(operand("pane-target", true))
        .arg(operand("name", true)),
    )
    .subcommand(
        with_options(
            general("name", "Bind this pane; temporary unless saved"),
            &["save"],
        )
        .visible_alias("this")
        .arg(operand("name", true)),
    )
    .subcommand(
        with_options(
            general(
                "marked",
                "Bind the pane explicitly marked in tmux; temporary unless saved",
            ),
            &["save"],
        )
        .arg(operand("name", true)),
    )
    .subcommand(
        with_options(
            general(
                "rm",
                "Retire identity and remove role/preamble; keep pane/exchanges (--force for saved)",
            ),
            &["force"],
        )
        .visible_alias("remove")
        .arg(operand("name", true)),
    )
    .subcommand(
        with_options(
            general("talk", "Send a request and wait for its durable reply"),
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
            general("check", "Capture diagnostic pane output"),
            &["lines"],
        )
        .visible_alias("read")
        .arg(operand("target", true))
        .arg(operand("capture-lines", false)),
    )
    .subcommand(
        general("whoami", "Show this pane's verified identity").arg(
            Arg::new("context").long("context").action(ArgAction::SetTrue)
                .help("Read bounded identity and pending-work context without changing state"),
        ),
    )
    .subcommand(general(
        "unbind",
        "Detach this pane; retire temporary identity",
    ))
    .subcommand(
        general("config", "View or modify settings")
            .subcommand(general("show", "Show settings"))
            .subcommand(
                with_options(general("set", "Set a setting"), &["global"])
                    .arg(operand("key", true))
                    .arg(operand("value", true).allow_negative_numbers(true)),
            )
            .subcommand(general("clear", "Clear a local setting").arg(operand("key", false))),
    )
    .subcommand(
        general("preamble", "Manage identity-owned preambles")
            .subcommand(general("show", "Show preambles").arg(operand("agent", false)))
            .subcommand(
                general("set", "Set a preamble")
                    .arg(operand("agent", true))
                    .arg(operand("content", true).num_args(1..)),
            )
            .subcommand(general("clear", "Clear a preamble").arg(operand("agent", true))),
    )
    .subcommand(
        with_options(
            storage("x", "Inspect and acknowledge exchanges"),
            &["identity", "limit", "after"],
        )
        .subcommand(with_options(
            storage("list", "List unacknowledged exchanges"),
            &["identity", "limit", "after"],
        ))
        .subcommand(
            with_options(
                storage("show", "Show retained exchange content"),
                &["identity", "incoming"],
            )
            .arg(operand("request-id", true)),
        )
        .subcommand(
            with_options(
                storage("ack", "Acknowledge an observed revision"),
                &["identity", "incoming"],
            )
            .arg(option("revision").required(true))
            .arg(operand("request-id", true)),
        )
        .subcommand(with_options(
            storage("ackall", "Acknowledge the current identity snapshot"),
            &["identity", "incoming"],
        ))
        .subcommand(with_options(
            storage("listen", "Wait for incoming inbox activity"),
            &["identity", "room", "timeout", "debounce"],
        )),
    )
    .subcommand(
        storage(
            "identity",
            "Manage identity records, metadata and self-reported status",
        )
        .subcommand_required(true)
        .subcommand(storage("create", "Create or save an identity").arg(operand("name", true)))
        .subcommand(storage("show", "Show a named identity or the verified caller").arg(operand("name", false)))
        .subcommand(with_options(
            storage("list", "List non-retired identities"),
            &["where", "has"],
        ))
        .subcommand(
            storage(
                "status",
                "Manage expiring self-reported activity, not endpoint presence",
            )
            .subcommand_required(true)
            .subcommand(with_options(
                storage("show", "Show current or stale status"),
                &["identity"],
            ))
            .subcommand(
                with_options(
                    storage("set", "Replace status and renew its expiry"),
                    &["identity", "mood", "for"],
                )
                .arg(operand("activity", true)),
            )
            .subcommand(with_options(
                storage("clear", "Clear this identity's self-reported status"),
                &["identity"],
            )),
        )
        .subcommand(
            storage("meta", "Manage descriptive identity metadata")
                .subcommand_required(true)
                .subcommand(
                    with_options(storage("set", "Set a metadata value"), &["identity"])
                        .arg(operand("key", true))
                        .arg(operand("value", true)),
                )
                .subcommand(
                    with_options(storage("get", "Get a metadata value"), &["identity"])
                        .arg(operand("key", true)),
                )
                .subcommand(with_options(
                    storage("list", "List metadata values"),
                    &["identity"],
                ))
                .subcommand(
                    with_options(storage("rm", "Remove a metadata value"), &["identity"])
                        .arg(operand("key", true)),
                ),
        ),
    )
    .subcommand(
        storage("notes", "Access saved identity notes")
            .subcommand_required(true)
            .subcommand(with_options(
                storage("path", "Initialize and print the local Markdown path")
                    .after_help("Saved identities only. Edit the returned Markdown file directly; an Office notebook object reads this same file. Office reads never create missing notes."),
                &["identity"],
            )),
    )
    .subcommand(
        with_options(general("role", "Manage role profiles"), &["identity"])
            .subcommand_required(true)
            .subcommand(with_options(general("show", "Show a role"), &["identity"]))
            .subcommand(
                with_options(
                    general("set", "Set a role from inline text or file"),
                    &["identity", "file"],
                )
                .arg(operand("content", false)),
            )
            .subcommand(with_options(
                general("clear", "Clear a role"),
                &["identity"],
            )),
    )
    .subcommand(
        with_options(
            storage("reply", "Submit an exact final response"),
            &["file", "message", "stdin"],
        )
        .arg(option("receipt").required(true))
        .arg(operand("request-id", true)),
    )
    .subcommand(
        storage("result", "Retrieve a retained final response").arg(operand("request-id", true)),
    )
    .subcommand(
        with_options(
            general("install", "Install or refresh agent skills"),
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
    .subcommand(general("setup", "Inspect or consent to agent lifecycle integration")
        .arg(operand("provider", false).value_parser(["claude", "codex"]))
        .arg(Arg::new("remove").long("remove").action(ArgAction::SetTrue).requires("provider"))
        .arg(Arg::new("yes").long("yes").action(ArgAction::SetTrue).requires("provider")))
    .subcommand(general("__hook", "Internal bounded provider lifecycle callback").hide(true)
        .arg(operand("provider", true).value_parser(["claude", "codex"]))
        .arg(Arg::new("worker").long("worker").hide(true).action(ArgAction::SetTrue)))
    .subcommand(general("__request-observer", "Internal bounded request timeout observer").hide(true)
        .arg(operand("request-id", true)))
    .subcommand(general("completion", "Generate shell completion").arg(operand("shell", false)))
    .subcommand(general("__complete", "Internal shell completion context").hide(true)
        .arg(Arg::new("words").num_args(0..).trailing_var_arg(true)
            .value_parser(clap::builder::OsStringValueParser::new())))
    .subcommand(
        general(
            "upgrade",
            "Upgrade the native CLI and refresh managed skills",
        )
        .visible_alias("update")
        .arg(
            Arg::new("channel").long("channel").value_parser(
                tmt_core::native_install::Channel::ALL.map(|channel| channel.as_str()),
            ),
        )
        .arg(Arg::new("to").long("to").conflicts_with("unpin"))
        .arg(Arg::new("unpin").long("unpin").action(ArgAction::SetTrue)),
    )
    .subcommand(general("__native-refresh-skills", "Internal managed skill refresh").hide(true))
    .subcommand(
        general("__native-install", "Internal offline native installation")
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
        general("learn", "Read agent guidance"),
        &["skill"],
    ))
}

fn base(name: &'static str, about: &'static str) -> Command {
    Command::new(name)
        .about(about)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .args_override_self(true)
}

fn storage(name: &'static str, about: &'static str) -> Command {
    base(name, about).arg(option("json"))
}

fn general(name: &'static str, about: &'static str) -> Command {
    with_options(storage(name, about), &["wait", "team"])
}

fn office(name: &'static str, about: &'static str) -> Command {
    general(name, about).arg(Arg::new("prefix").long("prefix").global(true))
}

fn office_commands() -> Command {
    office("office", "Manage the optional Office companion")
        .subcommand(
            office("start", "Start or reuse the local Office service").arg(
                Arg::new("port")
                    .long("port")
                    .value_parser(clap::value_parser!(u16).range(1..)),
            ),
        )
        .subcommand(office("stop", "Stop the local Office service"))
        .subcommand(office_scope(
            office("unpair", "Revoke the selected identity's Office pairing"),
            true,
        ))
        .subcommand(office(
            "sync",
            "Deliver pending identity retirement hooks to Office",
        ))
        .subcommand(
            office("layout", "Read or edit the complete local Office layout")
                .subcommand_required(true)
                .subcommand(office(
                    "show",
                    "Show the local world, layout and save revision",
                ))
                .subcommand(
                    office("apply", "Atomically apply a complete local world layout")
                        .arg(
                            Arg::new("file")
                                .long("file")
                                .required(true)
                                .help("World layout JSON, containing version, map and objects"),
                        )
                        .arg(
                            Arg::new("if-revision")
                                .long("if-revision")
                                .required(true)
                                .value_parser(
                                    clap::value_parser!(u64)
                                        .range(0..=tmt_core::limits::MAX_JS_SAFE_INTEGER),
                                ),
                        )
                        .arg(
                            Arg::new("legacy-basis")
                                .long("legacy-basis")
                                .help("Exact legacyBasis from show; required only at revision 0"),
                        ),
                ),
        )
        .subcommand(
            office("block", "Read or edit a remote Firestore Office block")
                .subcommand_required(true)
                .subcommand(office_scope(
                    office("show", "Show the selected Office block")
                        .arg(operand("block-id", false)),
                    true,
                ))
                .subcommand(office_scope(
                    office("apply", "Apply a complete layout to an Office block")
                        .arg(operand("block-id", false))
                        .arg(Arg::new("file").long("file").required(true))
                        .arg(
                            Arg::new("if-revision")
                                .long("if-revision")
                                .required(true)
                                .value_parser(
                                    clap::value_parser!(u64)
                                        .range(0..tmt_core::office_block::MAX_REVISION),
                                ),
                        ),
                    true,
                )),
        )
        .subcommand(
            office(
                "profile",
                "Read or edit a local Office presentation profile",
            )
            .subcommand_required(true)
            .subcommand(
                office(
                    "show",
                    "Show the selected identity's local presentation profile",
                )
                .arg(
                    Arg::new("local")
                        .long("local")
                        .required(true)
                        .action(ArgAction::SetTrue),
                )
                .arg(option("identity")),
            )
            .subcommand(
                office(
                    "apply",
                    "Apply the selected identity's local presentation profile",
                )
                .arg(
                    Arg::new("local")
                        .long("local")
                        .required(true)
                        .action(ArgAction::SetTrue),
                )
                .arg(option("identity"))
                .arg(Arg::new("file").long("file").required(true))
                .arg(
                    Arg::new("if-revision")
                        .long("if-revision")
                        .required(true)
                        .value_parser(
                            clap::value_parser!(u64)
                                .range(0..=tmt_core::office_profile::MAX_REVISION),
                        ),
                ),
            ),
        )
        .subcommand(office_prop_commands())
        .subcommand(office_avatar_commands())
        .subcommand(
            office("extension", "Check data-only World extension descriptions")
                .subcommand_required(true)
                .subcommand(
                    office(
                        "validate",
                        "Validate definition and instance structure; does not install or authorize",
                    )
                    .arg(Arg::new("file").long("file").required(true))
                    .arg(Arg::new("instance").long("instance").required(true)),
                ),
        )
        .subcommand(office_board_commands())
        .subcommand(
            office("whiteboard", "Read local immutable whiteboard snapshots")
                .subcommand_required(true)
                .subcommand(
                    office(
                        "snapshot",
                        "Read a captured revision without starting the web service",
                    )
                    .subcommand_required(true)
                    .subcommand(
                        office(
                            "show",
                            "Print the retained scene, selection and annotation as JSON",
                        )
                        .arg(
                            Arg::new("snapshot-reference")
                                .required(true)
                                .value_name("ID_OR_REFERENCE"),
                        ),
                    )
                    .subcommand(
                        office(
                            "export",
                            "Export the stored PNG to a new file (never overwrites)",
                        )
                        .arg(
                            Arg::new("snapshot-reference")
                                .required(true)
                                .value_name("ID_OR_REFERENCE"),
                        )
                        .arg(
                            Arg::new("output")
                                .long("output")
                                .required(true)
                                .value_name("PATH"),
                        ),
                    ),
                ),
        )
        .subcommand(office_scope(
            office(
                "inspect",
                "Check access to the paired identity's assigned block",
            ),
            true,
        ))
        .subcommand(office_scope(
            office(
                "status",
                "Inspect the installed companion and local service",
            ),
            false,
        ))
        .subcommand(
            office_scope(
                office(
                    "pair",
                    "Pair an existing identity with a private Office world",
                ),
                true,
            )
            .arg(
                Arg::new("read-only")
                    .long("read-only")
                    .action(ArgAction::SetTrue),
            )
            .arg(
                Arg::new("timeout")
                    .long("timeout")
                    .default_value("300")
                    .value_parser(clap::value_parser!(u64).range(1..=300)),
            ),
        )
        .subcommand(
            with_options(
                office("install", "Explicitly install the Office companion"),
                &["force"],
            )
            .arg(Arg::new("yes").long("yes").action(ArgAction::SetTrue))
            .arg(Arg::new("archive").long("archive").requires("manifest"))
            .arg(Arg::new("manifest").long("manifest").requires("archive"))
            .arg(Arg::new("channel").long("channel").value_parser(
                tmt_core::native_install::Channel::ALL.map(|channel| channel.as_str()),
            )),
        )
        .subcommand(
            with_options(
                office("upgrade", "Explicitly update the Office companion"),
                &["force"],
            )
            .arg(Arg::new("channel").long("channel").value_parser(
                tmt_core::native_install::Channel::ALL.map(|channel| channel.as_str()),
            )),
        )
        .subcommand(
            office(
                "uninstall",
                "Deactivate Office without deleting retained data",
            )
            .arg(Arg::new("yes").long("yes").action(ArgAction::SetTrue)),
        )
}

fn office_prop_commands() -> Command {
    let local = |command: Command| {
        command.arg(
            Arg::new("local")
                .long("local")
                .required(true)
                .action(ArgAction::SetTrue),
        )
    };
    let revision = |command: Command| {
        command.arg(
            Arg::new("if-revision")
                .long("if-revision")
                .required(true)
                .value_parser(
                    clap::value_parser!(u64).range(0..=tmt_core::limits::MAX_JS_SAFE_INTEGER),
                ),
        )
    };
    office("prop", "Manage local data-only Office prop packs")
        .subcommand_required(true)
        .subcommand(
            office("validate", "Validate one bounded data-only prop pack")
                .arg(Arg::new("file").long("file").required(true)),
        )
        .subcommand(
            office(
                "preview",
                "Preview one prop pack in the running local Office",
            )
            .arg(Arg::new("file").long("file").required(true)),
        )
        .subcommand(revision(local(
            office("install", "Install one prop pack into the local catalog")
                .arg(Arg::new("file").long("file").required(true)),
        )))
        .subcommand(revision(local(
            office("remove", "Remove one installed prop pack").arg(operand("prop-digest", true)),
        )))
        .subcommand(
            local(office("list", "List the local prop catalog"))
                .arg(
                    Arg::new("prop-limit")
                        .long("limit")
                        .default_value("20")
                        .value_parser(clap::value_parser!(u64).range(1..=20)),
                )
                .arg(Arg::new("cursor").long("cursor")),
        )
        .subcommand(
            local(office("show", "Show one local or built-in prop pack"))
                .arg(operand("prop-digest", true)),
        )
}

fn office_avatar_commands() -> Command {
    let local = |command: Command| {
        command.arg(
            Arg::new("local")
                .long("local")
                .required(true)
                .action(ArgAction::SetTrue),
        )
    };
    let revision = |command: Command| {
        command.arg(
            Arg::new("if-revision")
                .long("if-revision")
                .required(true)
                .value_parser(
                    clap::value_parser!(u64).range(0..=tmt_core::limits::MAX_JS_SAFE_INTEGER),
                ),
        )
    };
    office("avatar", "Manage local data-only Office avatar packs")
        .subcommand_required(true)
        .subcommand(
            office("validate", "Validate one bounded data-only avatar pack")
                .arg(Arg::new("file").long("file").required(true)),
        )
        .subcommand(
            office(
                "preview",
                "Preview one avatar pack in the running local Office",
            )
            .arg(Arg::new("file").long("file").required(true)),
        )
        .subcommand(revision(local(
            office("install", "Install one avatar pack into the local catalog")
                .arg(Arg::new("file").long("file").required(true)),
        )))
        .subcommand(revision(local(
            office("remove", "Remove one installed avatar pack")
                .arg(operand("avatar-digest", true)),
        )))
        .subcommand(
            local(office("list", "List the local avatar catalog"))
                .arg(
                    Arg::new("avatar-limit")
                        .long("limit")
                        .default_value("20")
                        .value_parser(clap::value_parser!(u64).range(1..=20)),
                )
                .arg(Arg::new("cursor").long("cursor")),
        )
        .subcommand(
            local(office("show", "Show one installed avatar pack"))
                .arg(operand("avatar-digest", true)),
        )
}

fn office_board_commands() -> Command {
    let category = |command: Command| {
        command
            .arg(Arg::new("repo").long("repo"))
            .arg(
                Arg::new("room")
                    .long("room")
                    .help("Meeting UUID or exact unambiguous name"),
            )
            .arg(
                Arg::new("general")
                    .long("general")
                    .action(ArgAction::SetTrue),
            )
            .group(
                clap::ArgGroup::new("board-category")
                    .args(["repo", "general", "room"])
                    .required(true),
            )
    };
    let actor = |command: Command| {
        command
            .arg(option("identity"))
            .arg(Arg::new("owner").long("owner").action(ArgAction::SetTrue))
            .group(clap::ArgGroup::new("board-actor").args(["identity", "owner"]))
    };
    let body = |command: Command, required: bool| {
        command
            .arg(Arg::new("body").long("body").allow_hyphen_values(true))
            .arg(Arg::new("file").long("file").allow_hyphen_values(true))
            .group(
                clap::ArgGroup::new("board-body")
                    .args(["body", "file"])
                    .required(required),
            )
    };
    let operation = |command: Command| command.arg(Arg::new("operation-id").long("operation-id"));
    let revision = |command: Command| {
        command.arg(
            Arg::new("if-revision")
                .long("if-revision")
                .required(true)
                .value_parser(clap::value_parser!(u64).range(1..)),
        )
    };
    office("board", "Use the local Office discussion board")
        .subcommand_required(true)
        .subcommand(operation(body(
            actor(
                category(office("post", "Post a board thread")).arg(
                    Arg::new("title")
                        .long("title")
                        .required(true)
                        .allow_hyphen_values(true),
                ),
            ),
            true,
        )))
        .subcommand(category(
            office("list", "List board threads")
                .arg(
                    Arg::new("view")
                        .long("view")
                        .default_value("recent")
                        .value_parser(["recent", "updated"]),
                )
                .arg(
                    Arg::new("author-id")
                        .long("author-id")
                        .conflicts_with("owner"),
                )
                .arg(Arg::new("owner").long("owner").action(ArgAction::SetTrue))
                .arg(Arg::new("since").long("since"))
                .arg(
                    Arg::new("board-limit")
                        .long("limit")
                        .default_value("20")
                        .value_parser(clap::value_parser!(u32).range(1..=50)),
                )
                .arg(Arg::new("cursor").long("cursor")),
        ))
        .subcommand(
            office("show", "Show a board thread")
                .arg(operand("thread-id", true))
                .arg(
                    Arg::new("reply-limit")
                        .long("reply-limit")
                        .default_value("20")
                        .value_parser(clap::value_parser!(u32).range(1..=50)),
                )
                .arg(Arg::new("reply-cursor").long("reply-cursor")),
        )
        .subcommand(operation(body(
            actor(office("reply", "Reply to a board thread").arg(operand("thread-id", true))),
            true,
        )))
        .subcommand(operation(revision(
            body(
                actor(
                    office("edit", "Edit a board entry")
                        .arg(operand("entry-id", true))
                        .arg(Arg::new("title").long("title").allow_hyphen_values(true)),
                ),
                false,
            )
            .group(
                clap::ArgGroup::new("board-edit-fields")
                    .args(["title", "body", "file"])
                    .multiple(true)
                    .required(true),
            ),
        )))
        .subcommand(operation(revision(actor(
            office("delete", "Delete a board entry")
                .arg(operand("entry-id", true))
                .arg(
                    Arg::new("moderate")
                        .long("moderate")
                        .requires("owner")
                        .action(ArgAction::SetTrue),
                ),
        ))))
}

fn office_scope(command: Command, required: bool) -> Command {
    command
        .arg(Arg::new("world").long("world").required(required))
        .arg(option("identity").requires("world"))
        .arg(
            Arg::new("emulator")
                .long("emulator")
                .requires("world")
                .action(ArgAction::SetTrue),
        )
}

fn with_options(mut command: Command, ids: &[&'static str]) -> Command {
    for id in ids {
        command = command.arg(option(id));
    }
    command
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
    let mut result = Command::new(definition.get_name().to_owned())
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
