//! CLI flag and interaction inputs for the shared browser opener.
pub use tmt_invoke::open::{Flag, Outcome};

pub fn flag(args: &clap::ArgMatches) -> Flag {
    match (args.get_flag("open"), args.get_flag("no-open")) {
        (_, true) => Flag::NoOpen,
        (true, false) => Flag::Open,
        _ => Flag::Unset,
    }
}

/// How an explicit `tmt colab open` request decides: it opens even without a terminal or with the
/// open setting off, but, like every browser handoff, never under an agent (`TMT_AGENT`) unless
/// `--open` asks for it. `--no-open` always wins.
pub fn explicit_flag(args: &clap::ArgMatches, agent: bool) -> Flag {
    explicit(flag(args), agent)
}

fn explicit(flag: Flag, agent: bool) -> Flag {
    match flag {
        Flag::Unset if agent => Flag::NoOpen,
        Flag::Unset => Flag::Open,
        other => other,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_open_prints_the_link_under_an_agent_unless_open_is_passed() {
        assert_eq!(explicit(Flag::Unset, true), Flag::NoOpen);
        assert_eq!(explicit(Flag::Open, true), Flag::Open);
        assert_eq!(explicit(Flag::NoOpen, true), Flag::NoOpen);
    }

    #[test]
    fn an_explicit_open_outside_an_agent_opens_unless_no_open_is_passed() {
        assert_eq!(explicit(Flag::Unset, false), Flag::Open);
        assert_eq!(explicit(Flag::Open, false), Flag::Open);
        assert_eq!(explicit(Flag::NoOpen, false), Flag::NoOpen);
    }
}
