//! Commands still migrating to the shared help style. The guard rejects stale entries.

use tmt_cli_style::audit::Rule;

pub const MIGRATING: &[(&str, &[Rule])] = &[];

/// Hidden subcommands, each with why: they are protocol entry points that
/// TMT itself, a provider, a shell or an installer invokes, or a retired name.
/// Nothing a user or an agent should type belongs here (`design/cli-style.md`,
/// "Hidden commands"). A new hidden command fails the guard until it is listed
/// with a reason; a listed one that stops being hidden or is removed fails too.
pub const HIDDEN: &[(&str, &str)] = &[
    (
        "tmt __digest-hook",
        "Session-only provider callback admits and settles a launch-scoped Digest checklist.",
    ),
    (
        "tmt team",
        "retired; answers UNSUPPORTED_TEAM for scripts that still call it",
    ),
    (
        "tmt __complete",
        "shell completion scripts ask it for candidates",
    ),
    (
        "tmt __completion-script",
        "prints the completion script a shell sources",
    ),
    (
        "tmt __consumption-sample",
        "bounded sampler that hook-driven runs spawn",
    ),
    (
        "tmt __hook",
        "provider lifecycle callback registered in provider settings",
    ),
    (
        "tmt __native-install",
        "offline installation handoff from the bootstrap and upgrade",
    ),
    (
        "tmt __native-schema",
        "compiled application-schema evidence exported by native preparation and archive verification",
    ),
    (
        "tmt __native-refresh-skills",
        "managed skill refresh run by tmt upgrade",
    ),
    (
        "tmt __native-rename-database",
        "Core database rename run by tmt upgrade",
    ),
    (
        "tmt __native-upgrade-extensions",
        "extension upgrade plan/apply run by tmt upgrade",
    ),
    (
        "tmt __channel-server",
        "stdio message-channel server a provider starts for tmt run --channel",
    ),
    (
        "tmt __request-observer",
        "detached timeout observer that talk starts for a request",
    ),
    (
        "tmt __reply-notice-worker",
        "detached worker that delivers a reply notice batch",
    ),
];
