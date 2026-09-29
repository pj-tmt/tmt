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
/// 2b Squad; Office's command crate stays until #355 resumes. `tmt result`
/// shares `response_command::execute` with `tmt reply`, so 2a gives its exact
/// body its own function before exempting it.
pub const MIGRATING: &[(&str, &str)] = &[
    ("tmt-cli", "binding_command.rs"),
    ("tmt-cli", "check_command.rs"),
    ("tmt-cli", "config_command.rs"),
    ("tmt-cli", "consent.rs"),
    ("tmt-cli", "context_command.rs"),
    ("tmt-cli", "exchange_command/listen.rs"),
    ("tmt-cli", "exchange_command/presentation.rs"),
    ("tmt-cli", "extension_command.rs"),
    ("tmt-cli", "extension_hooks_command.rs"),
    ("tmt-cli", "extension_install_command.rs"),
    ("tmt-cli", "focus_command.rs"),
    ("tmt-cli", "identity_command.rs"),
    ("tmt-cli", "identity_context.rs"),
    ("tmt-cli", "init_command.rs"),
    ("tmt-cli", "install_command.rs"),
    ("tmt-cli", "main.rs"),
    ("tmt-cli", "native_install_command.rs"),
    ("tmt-cli", "native_upgrade_command.rs"),
    ("tmt-cli", "notes_command.rs"),
    ("tmt-cli", "profile_command.rs"),
    ("tmt-cli", "request_observer_command.rs"),
    ("tmt-cli", "response_command.rs"),
    ("tmt-cli", "resume_command.rs"),
    ("tmt-cli", "room_command.rs"),
    ("tmt-cli", "room_command/dispatch.rs"),
    ("tmt-cli", "run_command.rs"),
    ("tmt-cli", "setup_command.rs"),
    ("tmt-cli", "skill_refresh_command.rs"),
    ("tmt-cli", "skill_reminder.rs"),
    ("tmt-cli", "talk_command.rs"),
    ("tmt-cli", "talk_command/presentation.rs"),
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
    ("tmt-squad", "consent.rs"),
    ("tmt-squad", "main.rs"),
    ("tmt-squad", "membership.rs"),
];
