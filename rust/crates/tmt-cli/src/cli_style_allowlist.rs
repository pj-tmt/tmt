//! Commands that do not yet follow the help style, with the rules each still
//! breaks. #436 PR 2a migrates the core commands; Office's commands stay
//! listed until #355 resumes. The list only shrinks and is empty at the end:
//! the guard fails when a listed command already follows the style.

use tmt_cli_style::audit::Rule::{self, *};

pub const MIGRATING: &[(&str, &[Rule])] = &[
    // Office, reached through the reserved facade (#355).
    ("tmt office", &[Template, Examples]),
    ("tmt office avatar", &[Template, Examples]),
    ("tmt office avatar install", &[Template, Examples]),
    ("tmt office avatar ls", &[Template, Examples]),
    ("tmt office avatar preview", &[Template, Examples]),
    ("tmt office avatar rm", &[Template, Examples]),
    ("tmt office avatar show", &[Template, Examples]),
    ("tmt office avatar validate", &[Template, Examples]),
    ("tmt office block", &[Template, Examples]),
    ("tmt office block apply", &[Template, Examples]),
    ("tmt office block show", &[Template, Examples]),
    ("tmt office board", &[Template, Examples]),
    ("tmt office board rm", &[Template, Examples]),
    ("tmt office board edit", &[Template, Examples]),
    ("tmt office board ls", &[Template, Examples]),
    ("tmt office board post", &[Template, Examples]),
    ("tmt office board reply", &[Template, Examples]),
    ("tmt office board show", &[Template, Examples]),
    ("tmt office extension", &[Template, Examples]),
    ("tmt office extension validate", &[Template, Examples]),
    ("tmt office inspect", &[Template, Examples]),
    ("tmt office install", &[Template, Examples]),
    ("tmt office layout", &[Template, Examples]),
    ("tmt office layout apply", &[Template, Examples]),
    ("tmt office layout show", &[Template, Examples]),
    ("tmt office pair", &[Template, Examples]),
    ("tmt office profile", &[Template, Examples]),
    ("tmt office profile apply", &[Template, Examples]),
    ("tmt office profile show", &[Template, Examples]),
    ("tmt office prop", &[Template, Examples]),
    ("tmt office prop install", &[Template, Examples]),
    ("tmt office prop ls", &[Template, Examples]),
    ("tmt office prop preview", &[Template, Examples]),
    ("tmt office prop rm", &[Template, Examples]),
    ("tmt office prop show", &[Template, Examples]),
    ("tmt office prop validate", &[Template, Examples]),
    ("tmt office start", &[Template, Examples]),
    ("tmt office status", &[Template, Examples]),
    ("tmt office stop", &[Template, Examples]),
    ("tmt office storage", &[Template, Examples]),
    ("tmt office storage migrate", &[Template, Examples]),
    ("tmt office storage status", &[Template, Examples]),
    ("tmt office sync", &[Template, Examples]),
    ("tmt office rm", &[Template, Examples]),
    ("tmt office unpair", &[Template, Examples]),
    ("tmt office upgrade", &[Template, Examples]),
    ("tmt office whiteboard", &[Template, Examples]),
    ("tmt office whiteboard snapshot", &[Template, Examples]),
    (
        "tmt office whiteboard snapshot export",
        &[Template, Examples],
    ),
    ("tmt office whiteboard snapshot show", &[Template, Examples]),
];

/// Hidden subcommands, each with why: they are protocol entry points that
/// TMT itself, a provider, a shell or an installer invokes, or a retired name.
/// Nothing a user or an agent should type belongs here (`design/cli-style.md`,
/// "Hidden commands"). A new hidden command fails the guard until it is listed
/// with a reason; a listed one that stops being hidden or is removed fails too.
pub const HIDDEN: &[(&str, &str)] = &[
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
