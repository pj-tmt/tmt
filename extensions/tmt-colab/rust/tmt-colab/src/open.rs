//! CLI flag and interaction inputs for the shared browser opener.
pub use tmt_invoke::open::{Flag, Outcome};

pub fn flag(args: &clap::ArgMatches) -> Flag {
    match (args.get_flag("open"), args.get_flag("no-open")) {
        (_, true) => Flag::NoOpen,
        (true, false) => Flag::Open,
        _ => Flag::Unset,
    }
}

pub fn open_link(link: &str, flag: Flag, setting: bool, json: bool) -> Outcome {
    tmt_invoke::open::open_link(
        link,
        flag,
        setting,
        json,
        tmt_cli_style::Interaction::detect(false).stdout,
    )
}

/// The `open` row value for a link, and the warning to print when the opener failed.
pub fn describe(outcome: &Outcome, link: &str) -> (String, Option<String>) {
    match outcome {
        Outcome::Opened => (format!("opened in your browser: {link}"), None),
        Outcome::Skipped | Outcome::NoOpener => (link.to_owned(), None),
        Outcome::Failed(why) => (
            link.to_owned(),
            Some(format!(
                "Could not open the browser ({why}); the link is above"
            )),
        ),
    }
}
