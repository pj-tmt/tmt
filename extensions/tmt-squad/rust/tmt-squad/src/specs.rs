//! What `tmt squad -h` prints for each command: a summary, one to three
//! examples (the common use first) and, only where a safety fact must be
//! visible, `Details`. `design/cli-style.md` owns the rules; the grammar walk
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

pub const CONFIG: &CommandSpec = spec!(
    "config", "Inspect effective Squad board settings",
    ["Inspect board defaults and their sources" => "tmt squad config show"]
);
pub const CRON: &CommandSpec = spec!(
    "cron", "Manage time-based Squad jobs",
    details = "Writes require the recorded user or the squad's lead. Change announcements are best effort. Jobs are stored separately from squad.toml; no run results or catch-up.",
    ["List jobs across every squad" => "tmt squad cron ls"]
);
pub const CRON_LS: &CommandSpec = spec!("ls", "List jobs, owners and future slots",
    ["List one squad's jobs" => "tmt squad cron ls --squad product"]);
pub const CRON_SHOW: &CommandSpec = spec!("show", "Show a job's full message and next three slots",
    ["Inspect one job" => "tmt squad cron show product c1"]);
pub const CRON_ADD: &CommandSpec = spec!("add", "Create a time-based job for a squad member",
    ["Send a reminder every thirty minutes" => "tmt squad cron add product worker --every 30m 'Review the queue'",
     "Schedule a weekday reminder" => "tmt squad cron add product worker --at 09:00 --on weekdays 'Review the queue'",
     "Use five-field cron syntax" => "tmt squad cron add product worker --cron '0 */3 * * *' 'Review the queue'"]);
pub const CRON_EDIT: &CommandSpec = spec!("edit", "Change a job's message or schedule",
    ["Change the interval" => "tmt squad cron edit product c1 --every 2h"]);
pub const CRON_RM: &CommandSpec = spec!("rm", "Remove a job; retain its id counter and exchange history",
    ["Remove one job" => "tmt squad cron rm product c1"]);
pub const CRON_PAUSE: &CommandSpec = spec!("pause", "Pause a job and record who paused it",
    ["Pause one job" => "tmt squad cron pause product c1"]);
pub const CRON_RESUME: &CommandSpec = spec!("resume", "Resume a job with a current owner",
    ["Resume one job" => "tmt squad cron resume product c1"]);
pub const CRON_REASSIGN: &CommandSpec = spec!("reassign", "Assign a job to another member, retaining its pause",
    ["Choose a new owner" => "tmt squad cron reassign product c1 reviewer"]);
pub const CRON_SEND: &CommandSpec = spec!("send", "Send a job's exact message once now without changing its schedule",
    details = "Requires the recorded user or the squad's lead. Paused jobs may be sent; jobs without an owner must be reassigned.",
    ["Send one job now" => "tmt squad cron send product c1"]);
pub const CRON_RUN: &CommandSpec = spec!("run", "Keep the Squad cron clock running here until Ctrl-C",
    details = "Only one clock holds the lease. Startup and takeover start at now; periods without a clock are never caught up. Scheduled requests are anonymous.",
    ["Keep the clock in this pane" => "tmt squad cron run"]);
pub const CRON_TICK: &CommandSpec = spec!("tick", "Send scheduled slots from the last sixty seconds, then exit",
    details = "Requires the clock lease. Repeated ticks reuse slot operation IDs; no run results are stored.",
    ["Run one clock pass" => "tmt squad cron tick"]);
pub const CRON_CLOCK: &CommandSpec = spec!("clock", "Show read-only clock lease evidence: pane, pid and since",
    ["Find the current clock" => "tmt squad cron clock"]);
pub const CONFIG_SHOW: &CommandSpec = spec!(
    "show", "Show effective board settings and where they come from",
    details = "Read-only. Configured commands are displayed, never executed.",
    [
        "Inspect one squad" => "tmt squad config show --squad product",
        "Inspect an aggregate tab as JSON" => "tmt squad config show --tab all --json",
    ]
);
pub const CONFIG_SET: &CommandSpec = spec!(
    "set", "Validate and save one simple Squad setting",
    details = "Uses squad.toml only. Refuses changed files and read-only settings. Lists use JSON array syntax.",
    ["Set one squad’s refresh interval" => "tmt squad config set board.refresh 10s --squad product",
     "Hide a positional track without changing the grid" => "tmt squad config set board.hidden_columns '[\"pr_link\"]' --squad product"]
);

pub const ROOT: &CommandSpec = spec!(
    "squad",
    "Leads, members and one board for a team of agents (alias: tmt sq)",
    [
        "Open the board, or list the members without a terminal" => "tmt squad",
        "Create a squad" => "tmt squad init product",
        "List every member and what it needs from you" => "tmt squad ls",
    ]
);

pub const INIT: &CommandSpec = spec!(
    "init",
    "Create the squad room squad-<name>; never asks anything",
    [
        "Create a squad" => "tmt squad init product",
        "Create one and record your saved identity, for a script" => "tmt squad init product --me ada",
    ]
);

pub const ME: &CommandSpec = spec!(
    "me",
    "Show, set or clear which saved identity is you (for ◆ waiting on you)",
    details = "Without a recorded identity, ◆ uses the saved identity bound to your pane, and
talk, reply, annotate and replies act as the pane's identity. It follows tmt mv.",
    [
        "See who you are" => "tmt squad me",
        "Record your saved identity" => "tmt squad me ada",
        "Stop recording one" => "tmt squad me --clear",
    ]
);

pub const LEAD: &CommandSpec = spec!(
    "lead",
    "Choose a saved lead or clear leadership; former leads stay members",
    [
        "Make sol the lead" => "tmt squad lead sol",
        "Choose the squad when several exist" => "tmt squad lead sol --squad product",
        "Clear leadership without removing the former lead" => "tmt squad lead --none",
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
    "rm",
    "Remove a member and clear its squad fields; the agent keeps running",
    [
        "Remove a member; the agent keeps running" => "tmt squad rm auth-fix",
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

pub const LS: &CommandSpec = spec!(
    "ls",
    "List members or a board tab as text, or JSON with --json",
    [
        "List every member and what needs you" => "tmt squad ls",
        "List every squad lead" => "tmt squad ls --tab leads",
        "Read the squad from a script" => "tmt squad ls --json",
    ]
);

pub const BOARD: &CommandSpec = spec!(
    "board",
    "Open the terminal board (lists the members without a terminal)",
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
    "rm",
    "Remove only squad's line and squad's bindings",
    details = "The rest of your tmux configuration is not touched.",
    [
        "Ask, then remove squad's keys" => "tmt squad hotkeys rm",
        "Remove them without a prompt" => "tmt squad hotkeys rm --yes",
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
        "Show your squad's lead" => "tmt squad jump --lead",
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
