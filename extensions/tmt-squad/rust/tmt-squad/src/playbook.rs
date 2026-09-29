//! Optional playbooks: guidance skills for lead agents, embedded in the
//! executable. Squad never runs one. `install` hands the skill to core's
//! extension-skill door (`skills.install`), owned by `squad`, so agents find it
//! in their provider skill directories; `remove` retracts only that skill.

use crate::{
    consent::consent,
    core::{Core, SquadError},
};
use clap::{Arg, ArgAction, Command};
use serde_json::{Value, json};
use tmt_cli_style::{CommandSpec, Example, OutputModes};

/// The owner recorded with core, shared with the `tmt-squad` lead skill that
/// `tmt extension install squad` offers, so uninstalling the extension removes
/// both.
const OWNER: &str = "squad";

struct Playbook {
    name: &'static str,
    skill: &'static str,
}

/// The catalog. Sources live in `extensions/tmt-squad/playbooks/`, beside
/// `skills/` and not inside it: the release archive ships every skill under
/// `skills/` and the extension installer offers them all, while a playbook is
/// installed only by asking for it here.
const PLAYBOOKS: &[Playbook] = &[Playbook {
    name: "tmux-squad",
    skill: include_str!("../../../playbooks/tmux-squad/SKILL.md"),
}];

/// `tmt squad playbook`, registered through the shared CLI style. `--json` is
/// squad's global option, so no command declares its own.
pub fn grammar() -> Command {
    let name = || {
        Arg::new("playbook")
            .required(true)
            .help("Playbook name (see `playbook list`)")
    };
    let yes = || {
        Arg::new("yes")
            .long("yes")
            .action(ArgAction::SetTrue)
            .help("Consent without a prompt")
    };
    tmt_cli_style::command(&CommandSpec {
        name: "playbook",
        summary: "Optional layouts for lead agents to propose (squad never runs them)",
        examples: &[
            Example {
                command: "tmt squad playbook list",
                note: "See which playbooks exist",
            },
            Example {
                command: "tmt squad playbook show tmux-squad",
                note: "Read one before installing it",
            },
            Example {
                command: "tmt squad playbook install tmux-squad",
                note: "Install it for your agents, after a prompt",
            },
        ],
        outputs: OutputModes::Human,
        details: "",
    })
    .subcommand_required(true)
    .subcommand(tmt_cli_style::command(&CommandSpec {
        name: "list",
        summary: "List the playbooks",
        examples: &[
            Example {
                command: "tmt squad playbook list",
                note: "List names and descriptions",
            },
            Example {
                command: "tmt squad playbook list --json",
                note: "Read them from a script",
            },
        ],
        outputs: OutputModes::Human,
        details: "",
    }))
    .subcommand(
        tmt_cli_style::command(&CommandSpec {
            name: "show",
            summary: "Print a playbook exactly as embedded",
            examples: &[Example {
                command: "tmt squad playbook show tmux-squad",
                note: "Print the tmux-squad playbook; nothing is installed",
            }],
            outputs: OutputModes::Human,
            details: "",
        })
        .arg(name()),
    )
    .subcommand(
        tmt_cli_style::command(&CommandSpec {
            name: "install",
            summary: "Show the plan, ask, then publish the playbook as an agent skill",
            examples: &[
                Example {
                    command: "tmt squad playbook install tmux-squad --print",
                    note: "See the plan first; nothing changes",
                },
                Example {
                    command: "tmt squad playbook install tmux-squad",
                    note: "Ask, then install for your agents",
                },
                Example {
                    command: "tmt squad playbook install tmux-squad --yes --force",
                    note: "Replace an unmanaged skill of that name, keeping a backup",
                },
            ],
            outputs: OutputModes::Human,
            details: "",
        })
        .arg(name())
        .arg(
            Arg::new("print")
                .long("print")
                .action(ArgAction::SetTrue)
                .help("Print the plan; change nothing"),
        )
        .arg(yes())
        .arg(
            Arg::new("force")
                .long("force")
                .action(ArgAction::SetTrue)
                .help("Back up and replace a skill of that name tmt does not manage"),
        ),
    )
    .subcommand(
        tmt_cli_style::command(&CommandSpec {
            name: "remove",
            summary: "Remove only this playbook's skill",
            examples: &[Example {
                command: "tmt squad playbook remove tmux-squad",
                note: "Ask, then remove it from your agents",
            }],
            outputs: OutputModes::Human,
            details: "",
        })
        .arg(name())
        .arg(yes()),
    )
}

fn failed(code: &str, message: impl Into<String>) -> SquadError {
    SquadError::new(code, message)
}

fn find(name: &str) -> Result<&'static Playbook, SquadError> {
    PLAYBOOKS
        .iter()
        .find(|playbook| playbook.name == name)
        .ok_or_else(|| {
            let known: Vec<&str> = PLAYBOOKS.iter().map(|playbook| playbook.name).collect();
            failed(
                "SQUAD_PLAYBOOK_UNKNOWN",
                format!(
                    "No playbook named '{name}'. Available: {}.",
                    known.join(", ")
                ),
            )
        })
}

/// The `description:` line of the skill's frontmatter.
fn description(skill: &str) -> &str {
    skill
        .lines()
        .skip(1)
        .take_while(|line| *line != "---")
        .find_map(|line| line.strip_prefix("description: "))
        .unwrap_or_default()
}

pub fn catalog() -> Value {
    json!({"playbooks": PLAYBOOKS.iter().map(|playbook| json!({
        "name": playbook.name,
        "description": description(playbook.skill),
    })).collect::<Vec<_>>()})
}

/// The exact embedded bytes.
pub fn embedded(name: &str) -> Result<Value, SquadError> {
    let playbook = find(name)?;
    Ok(json!({"name": playbook.name, "content": playbook.skill}))
}

/// `install <name> [--print] [--yes] [--force]`.
pub fn install(
    core: &Core,
    name: &str,
    print: bool,
    yes: bool,
    force: bool,
) -> Result<Value, SquadError> {
    let playbook = find(name)?;
    let plan = format!(
        "tmt squad playbook install will publish the skill '{}' ({} bytes) into the skill directory of each agent tmt detects, owned by '{OWNER}'.\n  {}\n  Squad never runs a playbook; it is guidance for your lead agent.",
        playbook.name,
        playbook.skill.len(),
        if force {
            "A path there that tmt does not manage is backed up and replaced."
        } else {
            "A path there that tmt does not manage is left alone, and the install is refused."
        },
    );
    if print {
        return Ok(json!({
            "name": playbook.name,
            "owner": OWNER,
            "bytes": playbook.skill.len(),
            "force": force,
            "plan": plan,
        }));
    }
    consent(yes, &plan, "Install this playbook?")?;
    let published = core
        .api(
            "skills.install",
            json!({
                "owner": OWNER,
                "consent": true,
                "force": force,
                "skills": [{
                    "name": playbook.name,
                    "files": [{"path": "SKILL.md", "content": playbook.skill}],
                }],
            }),
        )
        .map_err(|error| match error.code.as_str() {
            "SKILL_CONFLICT" => conflict(&error),
            _ => error,
        })?;
    let changed = published["published"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|target| target["changed"] == true);
    Ok(json!({
        "name": playbook.name,
        "owner": OWNER,
        "published": published["published"],
        "changed": changed,
    }))
}

/// `remove <name> [--yes]`: only this playbook, never the owner's other skills.
pub fn remove(core: &Core, name: &str, yes: bool) -> Result<Value, SquadError> {
    let playbook = find(name)?;
    let plan = format!(
        "tmt squad playbook remove will remove the skill '{}' that squad published into your agents' skill directories.\n  A copy you replaced or edited is kept, and squad's other skills (such as tmt-squad) stay.",
        playbook.name
    );
    consent(yes, &plan, "Remove this playbook?")?;
    let removed = core.api(
        "skills.remove",
        json!({"owner": OWNER, "consent": true, "skills": [playbook.name]}),
    )?;
    let removed_paths = removed["removed"].as_array().map_or(0, Vec::len);
    Ok(json!({
        "name": playbook.name,
        "owner": OWNER,
        "removed": removed["removed"],
        "kept": removed["kept"],
        "changed": removed_paths > 0,
    }))
}

/// Human output; `show` prints the embedded bytes untouched.
pub fn text(document: &Value, terminal: tmt_cli_style::Terminal) -> String {
    let done = |line: String| {
        let mut output = Vec::new();
        let _ = tmt_cli_style::message::success(&mut output, terminal, &line);
        String::from_utf8(output).unwrap_or_default()
    };
    let string = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    if let Some(content) = document["content"].as_str() {
        return content.to_owned();
    }
    if let Some(playbooks) = document["playbooks"].as_array() {
        return playbooks
            .iter()
            .map(|playbook| {
                format!(
                    "{}  {}\n",
                    string(&playbook["name"]),
                    string(&playbook["description"])
                )
            })
            .collect();
    }
    if let Some(plan) = document["plan"].as_str() {
        return format!("{plan}\n");
    }
    if let Some(published) = document["published"].as_array() {
        if document["changed"] != true {
            return "Already installed; nothing changed.\n".into();
        }
        return published
            .iter()
            .filter(|target| target["changed"] == true)
            .map(|target| {
                done(format!(
                    "Installed {}{}: {}{}",
                    string(&document["name"]),
                    target["agent"]
                        .as_str()
                        .map_or(String::new(), |agent| format!(" for {agent}")),
                    string(&target["target"]),
                    target["backup"]
                        .as_str()
                        .map_or(String::new(), |backup| format!(" (backup: {backup})")),
                ))
            })
            .collect();
    }
    let paths = |key: &str| -> Vec<String> {
        document[key]
            .as_array()
            .into_iter()
            .flatten()
            .map(&string)
            .collect()
    };
    let mut output = if document["changed"] == true {
        done(format!(
            "Removed {}: {}",
            string(&document["name"]),
            paths("removed").join(", ")
        ))
    } else {
        format!(
            "{} was not installed; nothing changed.\n",
            string(&document["name"])
        )
    };
    if !paths("kept").is_empty() {
        output.push_str(&format!(
            "Kept (no longer squad's copy): {}\n",
            paths("kept").join(", ")
        ));
    }
    output
}

/// Core refused to replace a folder squad does not own.
fn conflict(error: &SquadError) -> SquadError {
    SquadError::hinted(
        &error.code,
        &format!("{} Nothing was changed", error.message),
        "; ",
        "pass --force to back that path up and replace it.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skill_conflict_keeps_its_json_message_and_splits_the_next_step() {
        let error = conflict(&SquadError::new(
            "SKILL_CONFLICT",
            "The folder /h/.claude/skills/tmux-squad is not managed by squad.",
        ));
        assert_eq!(
            error.to_json().to_string(),
            r#"{"error":{"code":"SKILL_CONFLICT","message":"The folder /h/.claude/skills/tmux-squad is not managed by squad. Nothing was changed; pass --force to back that path up and replace it."}}"#
        );
        assert_eq!(
            error.human(),
            (
                "The folder /h/.claude/skills/tmux-squad is not managed by squad. Nothing was changed",
                Some("pass --force to back that path up and replace it.")
            )
        );
    }
    use std::{collections::BTreeSet, fs, path::Path};

    fn crate_dir() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn each_playbook_is_its_source_file_and_names_itself() {
        for playbook in PLAYBOOKS {
            let source = crate_dir()
                .join("../../playbooks")
                .join(playbook.name)
                .join("SKILL.md");
            assert_eq!(
                fs::read_to_string(&source).unwrap(),
                playbook.skill,
                "{}",
                source.display()
            );
            assert!(
                playbook
                    .skill
                    .starts_with(&format!("---\nname: {}\n", playbook.name)),
                "frontmatter names {}",
                playbook.name
            );
            assert!(!description(playbook.skill).is_empty());
        }
        let on_disk: BTreeSet<String> = fs::read_dir(crate_dir().join("../../playbooks"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        let catalog: BTreeSet<String> = PLAYBOOKS.iter().map(|p| p.name.to_owned()).collect();
        assert_eq!(on_disk, catalog, "every source directory is in the catalog");
    }

    #[test]
    fn no_playbook_is_in_the_release_skills_tree() {
        // The archive ships every skill directory under `skills/`, and
        // `tmt extension install squad` offers them all.
        let shipped: BTreeSet<String> = fs::read_dir(crate_dir().join("../../skills"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(shipped.contains("tmt-squad"), "positive control");
        for playbook in PLAYBOOKS {
            assert!(!shipped.contains(playbook.name), "{}", playbook.name);
        }
    }

    #[test]
    fn show_returns_the_exact_bytes_and_unknown_names_are_refused() {
        let shown = embedded("tmux-squad").unwrap();
        assert_eq!(shown["content"], PLAYBOOKS[0].skill);
        assert_eq!(
            text(&shown, tmt_cli_style::Terminal::PLAIN),
            PLAYBOOKS[0].skill
        );
        let error = embedded("herdr-squad").unwrap_err();
        assert_eq!(error.code, "SQUAD_PLAYBOOK_UNKNOWN");
        assert!(error.message.contains("tmux-squad"));
    }

    #[test]
    fn list_reports_name_and_description() {
        let listed = catalog();
        assert_eq!(listed["playbooks"][0]["name"], "tmux-squad");
        assert!(
            text(&listed, tmt_cli_style::Terminal::PLAIN)
                .starts_with("tmux-squad  Propose a tmux layout"),
            "{}",
            text(&listed, tmt_cli_style::Terminal::PLAIN)
        );
    }
}
