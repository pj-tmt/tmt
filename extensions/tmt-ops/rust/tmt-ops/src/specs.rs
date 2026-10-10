//! What `tmt ops squad -h` prints for each command: a summary, one to three
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
    ["Inspect board defaults and their sources" => "tmt ops squad config show"]
);
pub const CHECKLIST: &CommandSpec = spec!(
    "checklist", "Operate manually authored room-owned checklists",
    details = "Explicit UUIDs and exact revisions; no actor override or implicit dispatch. Outcome Unknown never authorizes blind replay.",
    ["Read a checklist" => "tmt ops squad checklist ls --room 44444444-4444-4444-8444-444444444444"]
);

pub const CHECKLIST_LS: &CommandSpec = spec!(
    "ls", "List checklist items without initializing storage",
    details = "Archive, completion and assignee filters project the authored inventory; they never replace it.",
    ["Read an exact room" => "tmt ops squad checklist ls --room 44444444-4444-4444-8444-444444444444"]
);

pub const CHECKLIST_SHOW: &CommandSpec = spec!(
    "show", "Show one exact checklist item",
    ["Inspect one item" => "tmt ops squad checklist show --room 44444444-4444-4444-8444-444444444444 --checklist 55555555-5555-4555-8555-555555555555 --item 66666666-6666-4666-8666-666666666666"]
);

pub const CHECKLIST_CREATE: &CommandSpec = spec!(
    "create", "Create an open item with supplied frozen UUIDs",
    details = "An initial assignee requires manager permission, including self-assignment. References are inert HTTP(S) provenance.",
    ["Create the first unassigned item" => "tmt ops squad checklist create 'Review the queue' --room 44444444-4444-4444-8444-444444444444 --checklist 55555555-5555-4555-8555-555555555555 --item 66666666-6666-4666-8666-666666666666 --expect-inventory absent"]
);

pub const CHECKLIST_EDIT: &CommandSpec = spec!(
    "edit", "Edit only supplied title, body or reference fields",
    ["Edit against a reviewed revision" => "tmt ops squad checklist edit --room 44444444-4444-4444-8444-444444444444 --checklist 55555555-5555-4555-8555-555555555555 --item 66666666-6666-4666-8666-666666666666 --expect-revision 1 --title 'Review the release'"]
);

pub const CHECKLIST_ASSIGN: &CommandSpec = spec!(
    "assign", "Assign an unarchived item to an exact active member UUID",
    details = "Requires manager permission.",
    ["Choose an active member" => "tmt ops squad checklist assign --room 44444444-4444-4444-8444-444444444444 --checklist 55555555-5555-4555-8555-555555555555 --item 66666666-6666-4666-8666-666666666666 --expect-revision 1 --assignee 77777777-7777-4777-8777-777777777777"]
);

pub const CHECKLIST_UNASSIGN: &CommandSpec = spec!(
    "unassign", "Clear assignment without resolving the former assignee",
    details = "Requires manager permission.",
    ["Clear an assignment" => "tmt ops squad checklist unassign --room 44444444-4444-4444-8444-444444444444 --checklist 55555555-5555-4555-8555-555555555555 --item 66666666-6666-4666-8666-666666666666 --expect-revision 1"]
);

pub const CHECKLIST_COMPLETE: &CommandSpec = spec!(
    "complete", "Complete an unarchived item without dispatch or attention changes",
    ["Acknowledge revision seven as revision eight" => "tmt ops squad checklist complete --room 44444444-4444-4444-8444-444444444444 --checklist 55555555-5555-4555-8555-555555555555 --item 66666666-6666-4666-8666-666666666666 --expect-revision 7"]
);

pub const CHECKLIST_REOPEN: &CommandSpec = spec!(
    "reopen", "Reopen an unarchived item",
    ["Reopen against its exact revision" => "tmt ops squad checklist reopen --room 44444444-4444-4444-8444-444444444444 --checklist 55555555-5555-4555-8555-555555555555 --item 66666666-6666-4666-8666-666666666666 --expect-revision 8"]
);

pub const CHECKLIST_ARCHIVE: &CommandSpec = spec!(
    "archive", "Archive an item while retaining content, completion and order",
    details = "Requires manager permission; archived items must be explicitly restored before editing.",
    ["Archive one item" => "tmt ops squad checklist archive --room 44444444-4444-4444-8444-444444444444 --checklist 55555555-5555-4555-8555-555555555555 --item 66666666-6666-4666-8666-666666666666 --expect-revision 8"]
);

pub const CHECKLIST_RESTORE: &CommandSpec = spec!(
    "restore", "Restore an archived item without changing completion or order",
    details = "Requires manager permission.",
    ["Restore one item" => "tmt ops squad checklist restore --room 44444444-4444-4444-8444-444444444444 --checklist 55555555-5555-4555-8555-555555555555 --item 66666666-6666-4666-8666-666666666666 --expect-revision 9"]
);

pub const CHECKLIST_DELETE: &CommandSpec = spec!(
    "delete", "Delete an exact item and retain its minimal tombstone",
    details = "Requires manager permission and matching item/revision confirmation. UUIDs cannot be reused; Unknown remains Unknown after readback.",
    ["Confirm the exact item and revision" => "tmt ops squad checklist delete --room 44444444-4444-4444-8444-444444444444 --checklist 55555555-5555-4555-8555-555555555555 --item 66666666-6666-4666-8666-666666666666 --expect-revision 9 --expect-inventory 1 --confirm-item 66666666-6666-4666-8666-666666666666 --confirm-revision 9"]
);

pub const CHECKLIST_REORDER: &CommandSpec = spec!(
    "reorder", "Reorder all nondeleted items including archived items",
    details = "Requires manager permission; a filtered item array is not the authored inventory.",
    ["Submit the full reviewed order" => "tmt ops squad checklist reorder --room 44444444-4444-4444-8444-444444444444 --checklist 55555555-5555-4555-8555-555555555555 --expect-inventory 1 --order '[\"66666666-6666-4666-8666-666666666666\"]'"]
);

pub const FOCUS: &CommandSpec = spec!(
    "focus", "Show, set or clear a member focus window",
    details = "Showing, setting and clearing focus all require the recorded user or the current squad lead. Whole s/m/h segments from 1s through 24h; no recurring cadence. Revision conflicts require reload and retry.",
    ["Focus for thirty minutes" => "tmt ops squad focus worker 30m",
     "Inspect the current policy" => "tmt ops squad focus worker",
     "Clear focus" => "tmt ops squad focus worker off"]
);

pub const CRON: &CommandSpec = spec!(
    "cron", "Manage time-based Squad jobs",
    details = "Writes require the recorded user or the squad's lead. Change announcements are best effort. Jobs are stored separately from ops.toml; no run results or catch-up.",
    ["List jobs across every squad" => "tmt ops squad cron ls"]
);
pub const CRON_LS: &CommandSpec = spec!("ls", "List jobs, owners and future slots",
    ["List one squad's jobs" => "tmt ops squad cron ls --squad product"]);
pub const CRON_SHOW: &CommandSpec = spec!("show", "Show a job's full message and next three slots",
    ["Inspect one job" => "tmt ops squad cron show product c1"]);
pub const CRON_ADD: &CommandSpec = spec!("add", "Create a time-based job for a squad member",
    ["Send a reminder every thirty minutes" => "tmt ops squad cron add product worker --every 30m 'Review the queue'",
     "Schedule a weekday reminder" => "tmt ops squad cron add product worker --at 09:00 --on weekdays 'Review the queue'",
     "Use five-field cron syntax" => "tmt ops squad cron add product worker --cron '0 */3 * * *' 'Review the queue'"]);
pub const CRON_EDIT: &CommandSpec = spec!("edit", "Change a job's message or schedule",
    ["Change the interval" => "tmt ops squad cron edit product c1 --every 2h"]);
pub const CRON_RM: &CommandSpec = spec!("rm", "Remove a job; retain its id counter and exchange history",
    ["Remove one job" => "tmt ops squad cron rm product c1"]);
pub const CRON_PAUSE: &CommandSpec = spec!("pause", "Pause a job and record who paused it",
    ["Pause one job" => "tmt ops squad cron pause product c1"]);
pub const CRON_RESUME: &CommandSpec = spec!("resume", "Resume a job with a current owner",
    ["Resume one job" => "tmt ops squad cron resume product c1"]);
pub const CRON_REASSIGN: &CommandSpec = spec!("reassign", "Assign a job to another member, retaining its pause",
    ["Choose a new owner" => "tmt ops squad cron reassign product c1 reviewer"]);
pub const CRON_SEND: &CommandSpec = spec!("send", "Send a job's exact message once now without changing its schedule",
    details = "Requires the recorded user or the squad's lead. Paused jobs may be sent; jobs without an owner must be reassigned.",
    ["Send one job now" => "tmt ops squad cron send product c1"]);
pub const CRON_RUN: &CommandSpec = spec!("run", "Keep the Squad cron clock running here until Ctrl-C",
    details = "Only one clock holds the lease. Startup and takeover start at now; periods without a clock are never caught up. Scheduled requests are anonymous.",
    ["Keep the clock in this pane" => "tmt ops squad cron run"]);
pub const CRON_TICK: &CommandSpec = spec!("tick", "Send scheduled slots from the last sixty seconds, then exit",
    details = "Requires the clock lease. Repeated ticks reuse slot operation IDs; no run results are stored.",
    ["Run one clock pass" => "tmt ops squad cron tick"]);
pub const CRON_CLOCK: &CommandSpec = spec!("clock", "Show read-only clock lease evidence: pane, pid and since",
    ["Find the current clock" => "tmt ops squad cron clock"]);
pub const CONFIG_SHOW: &CommandSpec = spec!(
    "show", "Show effective board settings and where they come from",
    details = "Read-only. Configured commands are displayed, never executed.",
    [
        "Inspect one squad" => "tmt ops squad config show --squad product",
        "Inspect an aggregate tab as JSON" => "tmt ops squad config show --tab all --json",
    ]
);
pub const CONFIG_SET: &CommandSpec = spec!(
    "set", "Validate and save one simple Squad setting",
    details = "Uses ops.toml only. Refuses changed files and read-only settings. Lists use JSON array syntax.",
    ["Set one squad’s refresh interval" => "tmt ops squad config set board.refresh 10s --squad product",
     "Hide a positional track without changing the grid" => "tmt ops squad config set board.hidden_columns '[\"pr_link\"]' --squad product"]
);

pub const ROOT: &CommandSpec = spec!(
    "ops", "TMT Ops: squads and a terminal board",
    ["Open the board" => "tmt ops ui",
     "List squad members" => "tmt ops squad ls"]
);
pub const SQUAD: &CommandSpec = spec!(
    "squad", "Manage squads and their members (alias: sq)",
    ["Create a squad" => "tmt ops squad init product",
     "List every member" => "tmt ops squad ls"]
);

pub const INIT: &CommandSpec = spec!(
    "init",
    "Create the squad room squad-<name>; never asks anything",
    [
        "Create a squad" => "tmt ops squad init product",
        "Create one and record your saved identity, for a script" => "tmt ops squad init product --me ada",
    ]
);

pub const ME: &CommandSpec = spec!(
    "me",
    "Show, set or clear which saved identity is you (for ◆ waiting on you)",
    details = "Without a recorded identity, ◆ uses the saved identity bound to your pane, and
the board's talk, answer and annotate act as the pane's identity. It follows tmt mv.",
    [
        "See who you are" => "tmt ops squad me",
        "Record your saved identity" => "tmt ops squad me ada",
        "Stop recording one" => "tmt ops squad me --clear",
    ]
);

pub const LEAD: &CommandSpec = spec!(
    "lead",
    "Choose a saved lead or clear leadership; former leads stay members",
    [
        "Make sol the lead" => "tmt ops squad lead sol",
        "Choose the squad when several exist" => "tmt ops squad lead sol --squad product",
        "Clear leadership without removing the former lead" => "tmt ops squad lead --none",
    ]
);

pub const ADD: &CommandSpec = spec!(
    "add",
    "Add running agents to the squad",
    [
        "Add two running agents as members" => "tmt ops squad add auth-fix docs-sweep",
        "Add one to a chosen squad" => "tmt ops squad add worker --squad product",
    ]
);

pub const REMOVE: &CommandSpec = spec!(
    "rm",
    "Remove a member and clear its squad fields; the agent keeps running",
    [
        "Remove a member; the agent keeps running" => "tmt ops squad rm auth-fix",
    ]
);

pub const SET: &CommandSpec = spec!(
    "set",
    "Set member fields such as state, task, pending, note or links (field= clears)",
    [
        "Mark a member blocked on your decision" => "tmt ops squad set auth-fix state=blocked pending=\"approve the plan\"",
        "Clear a field with an empty value" => "tmt ops squad set auth-fix pending=",
        "Record a link for open and copy" => "tmt ops squad set auth-fix pr_link=https://github.com/acme/app/pull/412",
    ]
);

pub const LS: &CommandSpec = spec!(
    "ls",
    "List members or a board tab as text, or JSON with --json",
    [
        "List every member and what needs you" => "tmt ops squad ls",
        "List every squad lead" => "tmt ops squad ls --tab leads",
        "Read the squad from a script" => "tmt ops squad ls --json",
    ]
);

pub const UI: &CommandSpec = spec!(
    "ui",
    "Open the terminal board (lists the members without a terminal)",
    [
        "Open the board in this terminal" => "tmt ops ui",
        "Close after a successful jump, for a tmux popup" => "tmt ops ui --popup",
        "Reload every open board once it is idle, after an upgrade" => "tmt ops ui --reload-all",
    ]
);

pub const HOTKEYS: &CommandSpec = spec!(
    "hotkeys",
    "tmux prefix keys that open the board (added to tmux.conf only with consent)",
    [
        "See whether the keys are installed" => "tmt ops hotkeys show",
        "See what install would write; nothing changes" => "tmt ops hotkeys install --print",
    ]
);

pub const HOTKEYS_INSTALL: &CommandSpec = spec!(
    "install",
    "Show the plan, then add squad's source-file line and bindings",
    details = "Squad adds only its own bindings file and one source-file line to your tmux\nconfiguration, and only after you consent.",
    [
        "See the bindings and the line; change nothing" => "tmt ops hotkeys install --print",
        "Show the plan, ask, then install" => "tmt ops hotkeys install",
        "Consent without a prompt" => "tmt ops hotkeys install --yes",
    ]
);

pub const HOTKEYS_REMOVE: &CommandSpec = spec!(
    "rm",
    "Remove only squad's line and squad's bindings",
    details = "The rest of your tmux configuration is not touched.",
    [
        "Ask, then remove squad's keys" => "tmt ops hotkeys rm",
        "Remove them without a prompt" => "tmt ops hotkeys rm --yes",
    ]
);

pub const HOTKEYS_SHOW: &CommandSpec = spec!(
    "show",
    "Report the hotkeys' state; change nothing",
    [
        "See whether the keys are installed and current" => "tmt ops hotkeys show",
        "Check it from a script" => "tmt ops hotkeys show --json",
    ]
);

pub const JUMP: &CommandSpec = spec!(
    "jump",
    "Show a member's pane in your tmux client (tmt focus)",
    [
        "Show a member's pane in your tmux client" => "tmt ops squad jump auth-fix",
        "Show your squad's lead" => "tmt ops squad jump --lead",
    ]
);

pub const BACK: &CommandSpec = spec!(
    "back",
    "Return your tmux client to where its last squad jump came from",
    [
        "Return to where the last jump came from" => "tmt ops squad back",
    ]
);

pub const OPEN: &CommandSpec = spec!(
    "open",
    "Open a member's http(s) link: pr_link, link or another *_link field",
    details = "Only http and https links open; anything else is refused.",
    [
        "Open a member's pull request link" => "tmt ops squad open auth-fix",
        "Open another link field" => "tmt ops squad open auth-fix --link issue_link",
    ]
);

pub const COPY: &CommandSpec = spec!(
    "copy",
    "Copy a member's summary to the clipboard",
    [
        "Copy \"name: task (state)\"" => "tmt ops squad copy auth-fix",
        "Copy a Markdown list item" => "tmt ops squad copy auth-fix --format '- [{name}]({pr_link})'",
    ]
);

pub const SKILL: &CommandSpec = spec!(
    "skill",
    "The tmt-ops skill for lead agents",
    [
        "Print the skill for a lead agent" => "tmt ops skill show",
    ]
);

pub const SKILL_SHOW: &CommandSpec = spec!(
    "show",
    "Print the skill",
    [
        "Print the skill; redirect it into an agent's skill directory" => "tmt ops skill show",
    ]
);

/// Offline files are checked against the built-in schema; the board never loads them.
pub const LAYOUT: &CommandSpec = spec!("layout", "Check markup authoring files offline",
    ["Check an authoring file" => "tmt ops squad layout validate board.xml"]);
pub const LAYOUT_VALIDATE: &CommandSpec = spec!("validate", "Validate XML, styles and declared Squad bindings",
    details = "Offline authoring check only. The board does not load this file. No core or config discovery.",
    ["Check an authoring file" => "tmt ops squad layout validate board.xml",
     "Get the validation result as JSON" => "tmt ops squad layout validate board.xml --json"]);
