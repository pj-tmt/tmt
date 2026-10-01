//! The public Office command subtree shared by the core facade and companion.

use clap::{Arg, ArgAction, Command};

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

pub fn grammar() -> Command {
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
                                        .range(0..tmt_office_model::office_block::MAX_REVISION),
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
                                .range(0..=tmt_office_model::office_profile::MAX_REVISION),
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
            office("storage", "Report or migrate where Office data is stored")
                .subcommand_required(true)
                .subcommand(office("status", "Show where Office data is stored"))
                .subcommand(
                    office(
                        "migrate",
                        "Move Office data into its own database, after a backup",
                    )
                    .arg(Arg::new("yes").long("yes").action(ArgAction::SetTrue)),
                ),
        )
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
            office("rm", "Deactivate Office without deleting retained data")
                .alias("uninstall")
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
            office("rm", "Remove one installed prop pack")
                .alias("remove")
                .arg(operand("prop-digest", true)),
        )))
        .subcommand(
            local(office("ls", "List the local prop catalog").alias("list"))
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
            office("rm", "Remove one installed avatar pack")
                .alias("remove")
                .arg(operand("avatar-digest", true)),
        )))
        .subcommand(
            local(office("ls", "List the local avatar catalog").alias("list"))
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
            office("ls", "List board threads")
                .alias("list")
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
            office("rm", "Delete a board entry")
                .alias("delete")
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

fn option(id: &'static str) -> Arg {
    match id {
        "json" => Arg::new(id)
            .long(id)
            .help("Output one JSON document")
            .action(ArgAction::SetTrue),
        "wait" => Arg::new(id)
            .long(id)
            .help("Retired; use timeout or detach")
            .hide(true)
            .action(ArgAction::SetTrue),
        "team" => Arg::new(id)
            .long(id)
            .help("Retired scope")
            .hide(true)
            .action(ArgAction::Set)
            .allow_hyphen_values(true),
        "identity" => Arg::new(id)
            .long(id)
            .help("Select an explicit identity")
            .global(true)
            .action(ArgAction::Set),
        "force" => Arg::new(id)
            .long(id)
            .short('f')
            .help("Authorize the command's documented protected replacement or removal")
            .action(ArgAction::SetTrue),
        _ => unreachable!("unknown Office grammar option"),
    }
}
