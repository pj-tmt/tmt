//! Human native page opening and the versioned extension-only launch seam.
use clap::{Arg, ArgAction, ArgMatches, Command};
use tmt_cli_style::{CommandSpec, Example, OutputModes};
use tmt_remote::{error::RemoteError, open, settings};

pub fn command(extension: bool) -> Command {
    let spec = CommandSpec {
        name: if extension { "launch" } else { "open" },
        summary: "Open a local page and connect this browser",
        examples: &[Example {
            command: "tmt remote open /settings",
            note: "Open Remote settings",
        }],
        outputs: if extension {
            OutputModes::Human
        } else {
            OutputModes::HumanAndJson
        },
        details: "Only a short path on the running local door is accepted. New browsers receive read-only access. While a manual pairing is open, the page opens without connecting a new browser. Printed links never contain a pairing code. --no-open suppresses opening.",
    };
    tmt_cli_style::command(&spec)
        .hide(extension)
        .arg(
            Arg::new("target")
                .required(extension)
                .help("Local short path, for example /p/<id>"),
        )
        .arg(
            Arg::new("open")
                .long("open")
                .action(ArgAction::SetTrue)
                .conflicts_with("no-open")
                .help("Open even when automatic opening is suppressed"),
        )
        .arg(
            Arg::new("no-open")
                .long("no-open")
                .action(ArgAction::SetTrue)
                .help("Print the short link without opening or pairing"),
        )
}

pub fn run(args: &ArgMatches, extension: bool) -> Result<(), RemoteError> {
    use std::io::Write;
    let root = super::discovery_root()?;
    let json = !extension && args.get_flag("json");
    let result = open::open_page(
        &root.join("remote"),
        args.get_one::<String>("target")
            .map(String::as_str)
            .unwrap_or("/"),
        open::flag(args),
        settings::read(&root)?.open(),
        json,
        extension || tmt_cli_style::Interaction::detect(false).stdout,
    )?;
    let mut output = tmt_cli_style::stream::stdout(extension || json);
    if extension || json {
        writeln!(
            output,
            "{}",
            serde_json::to_string(&result).expect("fixed open result")
        )?;
    } else {
        let terminal = output.terminal();
        if result.opened {
            tmt_cli_style::message::success(
                &mut output,
                terminal,
                &format!("Opened {}", result.link),
            )?;
        } else if result.state == "failed" {
            tmt_cli_style::message::warning(
                &mut output,
                terminal,
                &format!("Could not open the browser. Open {} yourself.", result.link),
                None,
            )?;
        } else {
            tmt_cli_style::message::success(
                &mut output,
                terminal,
                &format!("Open this link in your browser: {}", result.link),
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn open_help_states_what_happens_while_manual_pairing_is_open() {
        let help = super::command(false).render_long_help().to_string();
        let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(help.contains("Only a short path on the running local door is accepted. New browsers receive read-only access. While a manual pairing is open, the page opens without connecting a new browser. Printed links never contain a pairing code. --no-open suppresses opening."), "{help}");
    }
}
