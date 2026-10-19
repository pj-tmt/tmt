//! Hidden `tmt remote` subcommands, each with why. Only a protocol
//! entry that a shell or host invokes belongs here, and an entry needs a reason.
//! The hidden `serve --worker` option is not a subcommand and is outside this guard.

pub const HIDDEN: &[(&str, &str)] = &[
    (
        "tmt remote __activate",
        "Installed Colab's supervisor requests bounded native sync activation through Remote's CLI boundary.",
    ),
    (
        "tmt remote launch",
        "Extensions request native opening with a versioned short-only JSON result; Remote alone owns the launch code.",
    ),
];
