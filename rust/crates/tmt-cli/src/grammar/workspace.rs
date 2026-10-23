//! Workspace recovery preview and explicitly requested layout creation.

use crate::grammar::{base, storage};
use clap::{Arg, ArgAction, Command};
use tmt_core::workspace::SnapshotName;

pub(in crate::grammar) fn workspace() -> Command {
    base(spec!("workspace", "Inspect and restore remembered workspace recovery", [
        "Preview a remembered workspace" => "tmt workspace show",
    ])).subcommand(storage(spec!("show", "Preview layout and pane recovery without changing state", details = "A name already in use is added to or restored beside, never overwritten. Unknown live state grants no restore authority; each launch must recheck its identity and remembered session. Select the original socket explicitly when its pane is gone.", [
        "Preview this server's snapshot" => "tmt workspace show",
        "Select a snapshot after its server is gone" => "tmt workspace show --socket /tmp/workspace.sock --json",
    ])).arg(socket()).arg(selection()))
    .subcommand(storage(spec!("restore", "Restore layout, remembered agents and TMT commands", details = "A session in use is never overwritten: the fresh default session takes the saved windows after its own, any other name restores as <name>-restored, and a session this snapshot already restored is skipped. Agents use tmt resume; missing or stale sessions need you. Recorded TMT commands restart with their arguments. Failures retain partial creations. Use workspace show to preview.", [
        "Restore the workspace after its server is gone" => "tmt workspace restore --socket /tmp/workspace.sock",
        "Restore only the layout" => "tmt workspace restore --layout-only --socket /tmp/workspace.sock",
    ])).arg(socket()).arg(selection()).arg(Arg::new("expect").long("expect").value_name("DIGEST").value_parser(digest).help("Restore only the exact snapshot bytes previewed by show --json")).arg(Arg::new("layout-only").long("layout-only").action(ArgAction::SetTrue).help("Create layout and shells without reviving identities or recorded commands")))
    .subcommand(storage(spec!("save", "Save the latest captured workspace under an immutable name", details = "Copies the published capture without refreshing it. At most 16 names per socket; use rm to make room. Names use lowercase ASCII letters, digits, underscores and hyphens; latest is reserved.", [
        "Save the current capture" => "tmt workspace save work",
    ])).arg(socket()).arg(name(true)))
    .subcommand(storage(spec!("ls", "List retained generations and named snapshots for one socket", [
        "List recovery snapshots" => "tmt workspace ls --json",
    ])).alias("list").arg(socket()))
    .subcommand(storage(spec!("rm", "Remove one named workspace snapshot", [
        "Remove a saved snapshot" => "tmt workspace rm work",
    ])).arg(socket()).arg(name(true)))
}

fn selection() -> Arg {
    Arg::new("name")
        .value_name("SNAPSHOT")
        .value_parser(clap::value_parser!(tmt_core::workspace::SnapshotSelector))
        .help("Named snapshot or g:<generation>. Without it, restore picks the newest protected generation that has agents.")
}

fn name(required: bool) -> Arg {
    Arg::new("name")
        .value_name("NAME")
        .required(required)
        .value_parser(clap::value_parser!(SnapshotName))
        .help("Named snapshot; omission selects latest")
}

fn digest(value: &str) -> Result<String, String> {
    tmt_core::content_digest::is_sha256(value)
        .then(|| value.to_owned())
        .ok_or_else(|| "Expected a 64-character lowercase SHA-256 digest.".to_owned())
}

fn socket() -> Arg {
    Arg::new("socket")
        .long("socket")
        .value_name("PATH")
        .help("Exact original tmux socket path; no default-server or directory search")
}
