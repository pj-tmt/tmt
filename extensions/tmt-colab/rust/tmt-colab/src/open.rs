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

/// A committed page opens for the owner, independently of the agent's output mode.
/// Keep the shared flag, display, CI, opener discovery and launch policy; only the
/// terminal/JSON interaction gates do not apply to this creation effect.
pub fn open_created_link(link: &str, flag: Flag, setting: bool) -> Outcome {
    tmt_invoke::open::open_link(link, flag, setting, false, true)
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
