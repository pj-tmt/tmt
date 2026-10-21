//! `tmt colab settings [open on|off | wip-limit [AGENT] N|default | github-repo OWNER/REPO|none]`: show the
//! settings, or change one.
use crate::cli_management::fail;
use std::{io::Write, path::Path};
use tmt_colab::{Result, settings, wip, work_github::Repo};

fn invalid(message: &str) -> Box<dyn std::error::Error + Send + Sync> {
    fail("COLAB_INPUT_INVALID", message)
}
fn limit(value: &str) -> Result<u32> {
    value
        .parse::<u32>()
        .ok()
        .filter(|n| (1..=wip::MAX_LIMIT).contains(n) && value.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| {
            invalid(&format!(
                "A limit is a whole number from 1 to {}.",
                wip::MAX_LIMIT
            ))
        })
}

pub fn run(root: &Path, args: &clap::ArgMatches, json_output: bool) -> Result<()> {
    let value = args.get_one::<String>("value");
    let extra = args.get_one::<String>("extra");
    let loaded = match args.get_one::<String>("key").map(String::as_str) {
        None => settings::read_or_default(root),
        Some("open") => match (value, extra) {
            (Some(value), None) if ["on", "off"].contains(&value.as_str()) => {
                settings::set_open(root, value == "on")?
            }
            _ => return Err(invalid("open takes on or off.")),
        },
        Some("github-repo") => match (value, extra) {
            (None, None) => settings::read_or_default(root),
            (Some(value), None) if value == "none" => settings::set_github_repo(root, None)?,
            (Some(value), None) => settings::set_github_repo(
                root,
                Some(&Repo::parse(value).ok_or_else(|| {
                    invalid("github-repo takes OWNER/REPO (letters, digits, - _ .), or none.")
                })?),
            )?,
            _ => return Err(invalid("github-repo takes OWNER/REPO, or none.")),
        },
        Some(_) => match (value, extra) {
            (None, None) => settings::read_or_default(root),
            (Some(value), None) => settings::set_wip_limit(root, None, Some(limit(value)?))?,
            (Some(agent), Some(value)) => settings::set_wip_limit(
                root,
                Some(agent),
                if value == "default" {
                    None
                } else {
                    Some(limit(value)?)
                },
            )?,
            _ => return Err(invalid("wip-limit takes N, or AGENT N|default.")),
        },
    };
    let mut out = tmt_cli_style::stream::stdout(json_output);
    if json_output {
        writeln!(out, "{}", loaded.json())?;
    } else {
        let terminal = out.terminal();
        let shown = format!(
            "{} ({})",
            if loaded.open() { "on" } else { "off" },
            loaded.source()
        );
        let limits = loaded.wip();
        let mut wip_limit = format!(
            "{} for each agent",
            limits.default.unwrap_or(wip::DEFAULT_LIMIT)
        );
        for (agent, limit) in &limits.agents {
            wip_limit.push_str(&format!(", {agent} {limit}"));
        }
        tmt_cli_style::detail::write(
            &mut out,
            terminal,
            "COLAB SETTINGS",
            &[
                ("open", shown),
                ("wip-limit", wip_limit),
                (
                    "github-repo",
                    loaded
                        .github_repo()
                        .map_or_else(|| "none".to_owned(), Repo::slug),
                ),
            ],
        )?;
    }
    if loaded.malformed && !json_output {
        let mut err = tmt_cli_style::stream::stderr();
        let terminal = err.terminal();
        tmt_cli_style::message::warning(
            &mut err,
            terminal,
            settings::UNREADABLE,
            Some("tmt colab settings open on"),
        )?;
    }
    Ok(())
}
