use clap::{Arg, ArgAction, Command};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
const DISCLOSURE: &str = "Widening access requires --yes. Shared history includes deleted text, snapshots, comments and agent replies, up to the 64 most recent epochs. Current history shares the current epoch window. Editors can change page scripts for every viewer. Public mode is loopback-only; previously public content remains public.";
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
        .help("Confirm deletion or widening after reviewing disclosure")
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
fn role() -> Arg {
    Arg::new("role").value_parser(["viewer", "commenter", "editor"])
}
fn seed(command: Command) -> Command {
    command
        .arg(page())
        .arg(
            Arg::new("seed-file")
                .long("seed-file")
                .required(true)
                .help("Owned private file, or - for stdin; canonical base64url seed32"),
        )
        .arg(
            Arg::new("link-id")
                .long("link-id")
                .help("New UUID; retain for explicit retry"),
        )
        .arg(role().long("role").default_value("viewer"))
}
pub fn extend(root: Command) -> Command {
    let members = cmd!("members", "Inspect or change page members", "tmt colab share members ls 10000000-0000-4000-8000-000000000001", "Removal and role changes apply to the complete stored page assignment.").subcommand_required(true)
        .subcommand(cmd!("ls", "List page members", "tmt colab share members ls 10000000-0000-4000-8000-000000000001", "Lists verified assignments and revocation without changing state.").alias("list").arg(page()))
        .subcommand(cmd!("add", "Add a named member", "tmt colab share members add 10000000-0000-4000-8000-000000000001 --file member.json --yes", DISCLOSURE)
            .arg(page()).arg(Arg::new("file").long("file").required(true).help("Strict member selection JSON file, or - for stdin")))
        .subcommand(cmd!("remove", "Remove a member and rotate affected pages", "tmt colab share members remove 10000000-0000-4000-8000-000000000001 20000000-0000-4000-8000-000000000001", "Revokes the member and its devices; copied plaintext cannot be recalled.").arg(page()).arg(Arg::new("member").required(true).index(2)))
        .subcommand(cmd!("role", "Change a member role", "tmt colab share members role 10000000-0000-4000-8000-000000000001 20000000-0000-4000-8000-000000000001 viewer", DISCLOSURE).arg(page()).arg(Arg::new("member").required(true).index(2)).arg(role().required(true).index(3)));
    let links = cmd!("link", "Manage caller-held sharing links", "tmt colab share link add 10000000-0000-4000-8000-000000000001 --seed-file seed --yes", DISCLOSURE)
        .subcommand_required(true)
        .subcommand(cmd!("ls", "List page links", "tmt colab share link ls 10000000-0000-4000-8000-000000000001", "Never returns bearer seeds or private keys.").alias("list").arg(page()))
        .subcommand(seed(cmd!("add", "Add a fresh sharing link", "tmt colab share link add 10000000-0000-4000-8000-000000000001 --seed-file seed --yes", DISCLOSURE)))
        .subcommand(seed(cmd!("reset", "Revoke a link and create its replacement atomically", "tmt colab share link reset 10000000-0000-4000-8000-000000000001 20000000-0000-4000-8000-000000000001 --seed-file seed --yes", "Reset revokes the old link and its devices and rotates affected pages. Distribute the caller-held replacement seed only after success.").arg(Arg::new("link").required(true).index(2))))
        .subcommand(cmd!("remove", "Revoke a link and its certified devices", "tmt colab share link remove 10000000-0000-4000-8000-000000000001 20000000-0000-4000-8000-000000000001", "Rotates affected pages; copied plaintext cannot be recalled.").arg(page()).arg(Arg::new("link").required(true).index(2)));
    let share = mutation(
        cmd!(
            "share",
            "Inspect or change sharing authority",
            "tmt colab share members ls 10000000-0000-4000-8000-000000000001",
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
    .subcommand(
        cmd!(
            "history",
            "Select history for later joins",
            "tmt colab share history 10000000-0000-4000-8000-000000000001 current",
            DISCLOSURE
        )
        .arg(page())
        .arg(
            Arg::new("mode")
                .index(2)
                .required(true)
                .value_parser(["shared", "current"]),
        ),
    )
    .subcommand(members)
    .subcommand(links);
    root.subcommand(cmd!("ls", "List local pages", "tmt colab ls --json", "Archived pages need --archived. Local expiry is advisory and never deletes data. Expiry times are unavailable pending #1350.")
        .alias("list").arg(Arg::new("archived").long("archived").action(ArgAction::SetTrue)))
        .subcommand(cmd!("show", "Inspect one local page", "tmt colab show 10000000-0000-4000-8000-000000000001 --json", "Archived titles and discussions are unavailable. Expiry times are unavailable pending #1350; local data is never automatically deleted.").arg(page()))
        .subcommand(share)
        .subcommand(mutation(cmd!("retention", "Inspect or set local retention policy", "tmt colab retention 10000000-0000-4000-8000-000000000001 --days 30", "No option reads policy. Local expiry never deletes or denies access; expiry times are unavailable pending #1350.").arg(page()))
            .arg(Arg::new("days").long("days").value_parser(clap::value_parser!(u64).range(1..=9007199254740991)).conflicts_with("forever"))
            .arg(Arg::new("forever").long("forever").action(ArgAction::SetTrue)))
        .subcommand(mutation(cmd!("archive", "Hide a page and freeze its writes", "tmt colab archive 10000000-0000-4000-8000-000000000001", "Archived pages remain readable; this does not delete ciphertext.").arg(page())))
        .subcommand(mutation(cmd!("delete", "Delete page access and ciphertext", "tmt colab delete 10000000-0000-4000-8000-000000000001 --yes", "Requires --yes. Deletion cannot recall offline copies and does not promise secure erasure.").arg(page())))
}
