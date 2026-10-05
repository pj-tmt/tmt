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
