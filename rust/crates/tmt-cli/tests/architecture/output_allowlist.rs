//! The output guard's two lists. Both only shrink, and the guard fails on a
//! stale entry in either.

use super::output::Exact;

/// Functions whose output is an exact byte stream or a terminal protocol,
/// not styled text. Only the named function is exempt.
pub const EXACT_BODIES: &[Exact] = &[
    Exact {
        package: "tmt-cli",
        file: "api_command.rs",
        function: "execute",
        reason: "the versioned `tmt api` JSON response body",
    },
    Exact {
        package: "tmt-cli",
        file: "guidance_command.rs",
        function: "execute",
        reason: "`tmt learn` prints guidance and skill text byte for byte",
    },
    Exact {
        package: "tmt-cli",
        file: "main.rs",
        function: "write_exact",
        reason: "the version, completion candidates and completion scripts are read by shells and scripts",
    },
    Exact {
        package: "tmt-cli",
        file: "request_observer_command.rs",
        function: "announce_pid",
        reason: "the detached observer log's `observer_pid=<pid>` line, which diagnostics and tests read",
    },
    Exact {
        package: "tmt-cli",
        file: "provider_hook_command.rs",
        function: "execute",
        reason: "provider hook protocol: context on stdout, one fixed line on stderr",
    },
    Exact {
        package: "tmt-squad",
        file: "board/mod.rs",
        function: "run",
        reason: "the ratatui board owns the terminal; its colors come from `Token::color`",
    },
    Exact {
        package: "tmt-squad",
        file: "board/terminal/background.rs",
        function: "query",
        reason: "bounded OSC 11 background query owned by the board terminal lifecycle",
    },
    Exact {
        package: "tmt-squad",
        file: "board/terminal/background.rs",
        function: "seed",
        reason: "recognizes partial OSC protocol bytes consumed during the startup query",
    },
    Exact {
        package: "tmt-squad",
        file: "board/terminal.rs",
        function: "enter",
        reason: "the ratatui board enters the alternate screen",
    },
    Exact {
        package: "tmt-squad",
        file: "board/terminal.rs",
        function: "leave",
        reason: "the ratatui board restores the terminal",
    },
    Exact {
        package: "tmt-squad",
        file: "board/notes.rs",
        function: "<module>",
        reason: "the ESC constant that strips escape sequences from untrusted note text",
    },
    Exact {
        package: "tmt-squad",
        file: "effects.rs",
        function: "osc52",
        reason: "an OSC 52 clipboard write, a terminal protocol rather than styling",
    },
];

/// Files that still write around the style layer. #436 PR 2a migrates core,
/// 2b Squad; Office's command crate stays until #355 resumes.
pub const MIGRATING: &[(&str, &str)] = &[
    ("tmt-office-command", "office_avatar_command.rs"),
    ("tmt-office-command", "office_block_command.rs"),
    ("tmt-office-command", "office_board_command.rs"),
    ("tmt-office-command", "office_command.rs"),
    ("tmt-office-command", "office_extension_command.rs"),
    ("tmt-office-command", "office_layout_command.rs"),
    ("tmt-office-command", "office_pairing_command.rs"),
    ("tmt-office-command", "office_profile_command.rs"),
    ("tmt-office-command", "office_prop_command.rs"),
    ("tmt-office-command", "office_storage_command.rs"),
    ("tmt-office-command", "office_whiteboard_command.rs"),
    ("tmt-office-command", "public.rs"),
];
