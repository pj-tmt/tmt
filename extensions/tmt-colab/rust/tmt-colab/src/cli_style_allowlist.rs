//! Hidden `tmt colab` subcommands, each with why. Every other command is a user action shown in
//! help (`design/cli-style.md`, "Hidden commands"). Only a protocol entry that a host invokes
//! belongs here, and an entry needs a reason.

pub const HIDDEN: &[(&str, &str)] = &[(
    "tmt colab deploy-declaration",
    "Remote runs it by this fixed name through the installed tmt to read the compiled-in Firestore declaration (remote-channel-v1.md, authorized deploy); no person needs it",
)];
