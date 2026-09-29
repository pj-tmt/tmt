//! What `tmt squad -h` prints for each command: a summary, one to three
//! examples (the common use first) and, only where a safety fact must be
//! visible, `Details`. `docs/cli-style.md` owns the rules; the grammar walk
//! in `cli_style_tests.rs` parses every example through Squad's real grammar.

use tmt_cli_style::CommandSpec;

/// `--json` is Squad's one global option, so no command asks for its own.
macro_rules! spec {
    ($name:literal, $summary:literal, [$($note:literal => $command:literal),+ $(,)?]) => {
        &CommandSpec {
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
        &CommandSpec {
            details: $details,
            ..*spec!($name, $summary, [$($note => $command),+])
        }
    };
}

pub const ROOT: &CommandSpec = spec!(
    "squad",
    "Leads, members and one board for a team of agents (alias: tmt sq)",
    [
        "Create a squad and record which saved identity is you" => "tmt squad init product --me ada",
        "See every member and what it needs from you" => "tmt squad status",
        "Open the terminal board" => "tmt squad board",
    ]
);

pub const INIT: &CommandSpec = spec!(
    "init",
    "Create the squad room squad-<name>; record which saved identity is you",
    [
        "Create a squad and say which saved identity is you" => "tmt squad init product --me ada",
        "Create another squad; your identity is already recorded" => "tmt squad init reviews",
    ]
);

pub const LEAD: &CommandSpec = spec!(
    "lead",
    "Make a saved identity the squad's lead",
    [
        "Make sol the lead" => "tmt squad lead sol",
        "Choose the squad when several exist" => "tmt squad lead sol --squad product",
    ]
);

pub const ADD: &CommandSpec = spec!(
    "add",
    "Add running agents to the squad",
    [
        "Add two running agents as members" => "tmt squad add auth-fix docs-sweep",
        "Add one to a chosen squad" => "tmt squad add worker --squad product",
    ]
);

pub const REMOVE: &CommandSpec = spec!(
    "remove",
    "Remove a member and clear its squad fields; the agent keeps running",
    [
        "Remove a member; the agent keeps running" => "tmt squad remove auth-fix",
    ]
);

pub const SET: &CommandSpec = spec!(
    "set",
    "Set member fields such as state, task, pending, note or links (field= clears)",
    [
        "Mark a member blocked on your decision" => "tmt squad set auth-fix state=blocked pending=\"approve the plan\"",
        "Clear a field with an empty value" => "tmt squad set auth-fix pending=",
        "Record a link for open and copy" => "tmt squad set auth-fix pr_link=https://github.com/acme/app/pull/412",
    ]
);

pub const STATUS: &CommandSpec = spec!(
    "status",
    "Show the squad as text, or JSON with --json",
    [
        "Show every member and what needs you" => "tmt squad status",
        "Read the squad from a script" => "tmt squad status --json",
    ]
);

pub const BOARD: &CommandSpec = spec!(
    "board",
    "Open the terminal board (prints status without a terminal)",
    [
        "Open the board in this terminal" => "tmt squad board",
        "Close after a successful jump, for a tmux popup" => "tmt squad board --popup",
    ]
);

pub const HOTKEYS: &CommandSpec = spec!(
    "hotkeys",
    "tmux prefix keys that open the board (added to tmux.conf only with consent)",
    [
        "See whether the keys are installed" => "tmt squad hotkeys show",
        "See what install would write; nothing changes" => "tmt squad hotkeys install --print",
    ]
);

pub const HOTKEYS_INSTALL: &CommandSpec = spec!(
    "install",
    "Show the plan, then add squad's source-file line and bindings",
    details = "Squad adds only its own bindings file and one source-file line to your tmux\nconfiguration, and only after you consent.",
    [
        "See the bindings and the line; change nothing" => "tmt squad hotkeys install --print",
        "Show the plan, ask, then install" => "tmt squad hotkeys install",
        "Consent without a prompt" => "tmt squad hotkeys install --yes",
    ]
);

pub const HOTKEYS_REMOVE: &CommandSpec = spec!(
    "remove",
    "Remove only squad's line and squad's bindings",
    details = "The rest of your tmux configuration is not touched.",
    [
        "Ask, then remove squad's keys" => "tmt squad hotkeys remove",
        "Remove them without a prompt" => "tmt squad hotkeys remove --yes",
    ]
);

pub const HOTKEYS_SHOW: &CommandSpec = spec!(
    "show",
    "Report the hotkeys' state; change nothing",
    [
        "See whether the keys are installed and current" => "tmt squad hotkeys show",
        "Check it from a script" => "tmt squad hotkeys show --json",
    ]
);

pub const JUMP: &CommandSpec = spec!(
    "jump",
    "Show a member's pane in your tmux client (tmt focus)",
    [
        "Show a member's pane in your tmux client" => "tmt squad jump auth-fix",
    ]
);

pub const TALK: &CommandSpec = spec!(
    "talk",
    "Send a detached request to a member, in the squad room",
    [
        "Send a request without waiting for the answer" => "tmt squad talk auth-fix \"Check the retry path\"",
        "Ask the lead of a chosen squad" => "tmt squad talk sol \"Summarize the open PRs\" --squad product",
    ]
);

pub const REPLY: &CommandSpec = spec!(
    "reply",
    "Answer what a member is waiting on you for",
    [
        "Answer what a member asked" => "tmt squad reply auth-fix \"Use postgres\"",
        "Pick the request when it asks several" => "tmt squad reply auth-fix \"Use postgres\" --request req_8f3a2c1d",
    ]
);

pub const ANNOTATE: &CommandSpec = spec!(
    "annotate",
    "Send a note about a member's row to the lead (or the member)",
    [
        "Send the lead a note about a member's row" => "tmt squad annotate auth-fix \"Split this job\"",
        "Send the note to the member instead" => "tmt squad annotate auth-fix \"Rebase first\" --to member",
    ]
);

pub const REPLIES: &CommandSpec = spec!(
    "replies",
    "Show finals to your squad requests, newest first (never acknowledges)",
    [
        "Read the answers to your requests" => "tmt squad replies",
        "Read them from a script" => "tmt squad replies --json",
    ]
);

pub const BACK: &CommandSpec = spec!(
    "back",
    "Return your tmux client to where its last squad jump came from",
    [
        "Return to where the last jump came from" => "tmt squad back",
    ]
);

pub const OPEN: &CommandSpec = spec!(
    "open",
    "Open a member's http(s) link: pr_link, link or another *_link field",
    details = "Only http and https links open; anything else is refused.",
    [
        "Open a member's pull request link" => "tmt squad open auth-fix",
        "Open another link field" => "tmt squad open auth-fix --link issue_link",
    ]
);

pub const COPY: &CommandSpec = spec!(
    "copy",
    "Copy a member's summary to the clipboard",
    [
        "Copy \"name: task (state)\"" => "tmt squad copy auth-fix",
        "Copy a Markdown list item" => "tmt squad copy auth-fix --format '- [{name}]({pr_link})'",
    ]
);

pub const SKILL: &CommandSpec = spec!(
    "skill",
    "The tmt-squad skill for lead agents",
    [
        "Print the skill for a lead agent" => "tmt squad skill show",
    ]
);

pub const SKILL_SHOW: &CommandSpec = spec!(
    "show",
    "Print the skill",
    [
        "Print the skill; redirect it into an agent's skill directory" => "tmt squad skill show",
    ]
);
