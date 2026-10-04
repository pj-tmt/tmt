use clap::{Arg, ArgAction, Command};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
const DISCLOSURE: &str = "Widening access requires --yes. Shared history includes deleted text, snapshots, comments and agent replies, up to the 64 most recent epochs. Current history shares the current epoch window. Public mode is loopback-only; previously public content remains public.";
const READER_LINK: &str = "Prints readerPath, the only place the link seed appears; open it under your Remote door address (the one tmt remote pair printed). Anyone holding the link who can reach that door can read this page, but cannot edit it or ask agents. Without --seed-file a fresh seed is generated; to retry a possibly committed add, pass the same --link-id and --seed-file. Widening access requires --yes. Shared history includes deleted text, snapshots, comments and agent replies, up to the 64 most recent epochs. Current history shares the current epoch window.";
const RESET_LINK: &str = "Reset revokes the old link and its devices and rotates affected pages, then prints the replacement readerPath (the only place its seed appears). Without --seed-file a fresh seed is generated; to retry a possibly committed reset, pass the same --link-id and --seed-file. Anyone who can reach the Remote door and holds the new link can read the page.";
macro_rules! cmd {
    ($name:literal, $summary:literal, $example:literal, $details:expr) => {
        tmt_cli_style::command(&CommandSpec {
            name: $name,
            summary: $summary,
            examples: &[Example {
                command: $example,
                note: $summary,
            }],
            outputs: OutputModes::HumanAndJson,
            details: $details,
        })
    };
}
fn page() -> Arg {
    Arg::new("page").required(true).index(1)
}
fn yes() -> Arg {
    Arg::new("yes")
        .long("yes")
        .action(ArgAction::SetTrue)
        .global(true)
        .help("Confirm widening after reviewing disclosure")
}
fn mutation(command: Command) -> Command {
    command
        .mut_arg("json", |arg| arg.global(true))
        .arg(yes())
        .arg(
            Arg::new("operation-id")
                .long("operation-id")
                .global(true)
                .help("Frozen UUID for an explicit retry"),
        )
        .arg(
            Arg::new("expected-revision")
                .long("expected-revision")
                .global(true)
                .help("Frozen owner revision; never refreshed on retry"),
        )
}
fn seed(command: Command) -> Command {
    command
        .arg(page())
        .arg(
            Arg::new("seed-file")
                .long("seed-file")
                .help("Owned private file, or - for stdin; canonical base64url seed32. Absent: a fresh seed is generated"),
        )
        .arg(
            Arg::new("link-id")
                .long("link-id")
                .help("New UUID; retain for explicit retry"),
        )
}
pub fn extend(root: Command) -> Command {
    let links = cmd!("link", "Manage caller-held sharing links", "tmt colab share link add 10000000-0000-4000-8000-000000000001 --seed-file seed --yes", DISCLOSURE)
        .subcommand_required(true)
        .subcommand(cmd!("ls", "List page links", "tmt colab share link ls 10000000-0000-4000-8000-000000000001", "Never returns bearer seeds or private keys.").alias("list").arg(page()))
        .subcommand(seed(cmd!("add", "Add a fresh read-only sharing link", "tmt colab share link add 10000000-0000-4000-8000-000000000001 --yes", READER_LINK)))
        .subcommand(seed(cmd!("reset", "Revoke a link and create its replacement atomically", "tmt colab share link reset 10000000-0000-4000-8000-000000000001 20000000-0000-4000-8000-000000000001 --yes", RESET_LINK).arg(Arg::new("link").required(true).index(2))))
        .subcommand(cmd!("remove", "Revoke a link and its certified devices", "tmt colab share link remove 10000000-0000-4000-8000-000000000001 20000000-0000-4000-8000-000000000001", "Rotates affected pages; copied plaintext cannot be recalled.").arg(page()).arg(Arg::new("link").required(true).index(2)));
    let share = mutation(
        cmd!(
            "share",
            "Inspect or change sharing authority",
            "tmt colab share link ls 10000000-0000-4000-8000-000000000001",
            DISCLOSURE
        )
        .subcommand_required(true),
    )
    .subcommand(
        cmd!(
            "mode",
            "Select the page audience",
            "tmt colab share mode 10000000-0000-4000-8000-000000000001 private",
            DISCLOSURE
        )
        .arg(page())
        .arg(
            Arg::new("mode")
                .index(2)
                .required(true)
                .value_parser(["private", "link", "public"]),
        ),
    )
    .subcommand(links);
    root.subcommand(cmd!("ls", "List local pages", "tmt colab ls --json", "Archived pages need --archived. Local expiry is advisory and never deletes data. Expiry times are not available yet.")
        .alias("list").arg(Arg::new("archived").long("archived").action(ArgAction::SetTrue)))
        .subcommand(cmd!("show", "Inspect one local page", "tmt colab show 10000000-0000-4000-8000-000000000001 --json", "Archived titles and discussions are unavailable. Expiry times are not available yet; local data is never automatically deleted.").arg(page()))
        .subcommand(share)
}
