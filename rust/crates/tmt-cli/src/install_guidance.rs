//! One read-only projection of the install planner's facts for people and screens.
//! A verified install is not evidence that a service is running or a device is paired.

use serde_json::{Value, json};
use std::{
    io::{self, Write},
    path::Path,
};
use tmt_adapters::{
    drivers::{Detection, Registry},
    native_install,
    skill_installation::ProviderEnvironment,
};
use tmt_cli_style::{Terminal, Token};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Use,
    Install,
    Repair,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Action {
    pub id: String,
    pub kind: Kind,
    pub argv: Vec<String>,
    pub description: String,
}

impl Action {
    fn new(id: &str, kind: Kind, argv: Vec<String>, description: &str) -> Self {
        Self {
            id: id.into(),
            kind,
            argv,
            description: description.into(),
        }
    }

    pub(crate) fn repair(name: &str, argv: Vec<String>, description: &str) -> Self {
        Self::new(&format!("repair-{name}"), Kind::Repair, argv, description)
    }

    pub(crate) fn line(&self) -> String {
        format!("{}  — {}", command(&self.argv), self.description)
    }
}

#[derive(Default, Debug)]
pub(crate) struct Guidance {
    pub actions: Vec<Action>,
}

impl Guidance {
    pub(crate) fn document(&self) -> Value {
        json!({"actions": self.actions.iter().map(|action| json!({
            "id": action.id, "kind": match action.kind { Kind::Use => "use", Kind::Install => "install", Kind::Repair => "repair" },
            "argv": action.argv, "description": action.description
        })).collect::<Vec<_>>()})
    }

    pub(crate) fn missing(&self, name: &str) -> bool {
        self.actions
            .iter()
            .any(|action| action.id == format!("install-{name}"))
    }

    pub(crate) fn write(
        &self,
        title: &str,
        terminal: Terminal,
        output: &mut impl Write,
    ) -> io::Result<()> {
        if self.actions.is_empty() {
            return Ok(());
        }
        writeln!(
            output,
            "{}",
            terminal.paint(Token::Title, &format!("{title}:"))
        )?;
        for action in &self.actions {
            writeln!(output, "  {}", tmt_cli_style::table::escape(&action.line()))?;
        }
        Ok(())
    }
}

pub(crate) fn command(argv: &[String]) -> String {
    argv.iter()
        .map(|word| crate::output::shell_word(word))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Local receipts/links only. An optional observation never changes the operation's result.
/// `confirmed` holds this invocation's existing acquisition/upgrade results, not a new fetch.
pub(crate) fn read(prefix: Option<&Path>, confirmed: &[Value]) -> Guidance {
    let prefix = prefix
        .map(Path::to_path_buf)
        .or_else(|| native_install::default_install_prefix().ok());
    let search = std::env::var_os("PATH").unwrap_or_default();
    prefix
        .and_then(|prefix| {
            crate::extension_install_command::listing_with_guidance(
                &prefix, false, &search, confirmed,
            )
            .ok()
        })
        .map(|(_, _, guidance)| guidance)
        .unwrap_or_else(|| project_with_agent(&[], &[], Vec::new(), None, None))
}

pub(crate) fn project(
    rows: &[Value],
    confirmed: &[Value],
    recovery: Vec<Action>,
    prefix: Option<&Path>,
    search: &std::ffi::OsStr,
) -> Guidance {
    let agent = ProviderEnvironment::capture().ok().and_then(|env| {
        let registry = Registry::builtin();
        let env = ProviderEnvironment::from_parts(
            env.home(),
            env.resolve(Path::new(".")),
            std::env::split_paths(search).collect(),
            registry
                .env_vars()
                .into_iter()
                .filter_map(|name| env.var(name).map(|value| (name, value.to_path_buf()))),
        );
        registry
            .detect(&env)
            .into_iter()
            .find(|(_, detection)| matches!(detection, Detection::Present { .. }))
            .map(|(driver, _)| driver.name())
    });
    project_with_agent(rows, confirmed, recovery, prefix, agent)
}

fn ready(row: &Value) -> bool {
    row["installed"] == true
        && row["onPath"] == true
        && row["status"].is_null()
        && row["shadowedBy"].as_array().is_none_or(Vec::is_empty)
        && row["cli"]["compatible"] != false
}

fn current(row: &Value, confirmed: &[Value]) -> bool {
    ready(row)
        && (row["update"] == "current"
            || confirmed.iter().any(|result| {
                (result["product"] == row["name"] || result["extension"] == row["name"])
                    && result["version"].is_string()
                    && result["version"] == row["version"]
                    && matches!(
                        result["status"].as_str(),
                        Some("changed" | "unchanged" | "installed")
                    )
            }))
}

fn feature(name: &str) -> &str {
    match name {
        "ops" => "the terminal workspace",
        "colab" => "browser spaces",
        "remote" => "browser connections and cloud setup",
        "digest" => "batched agent attention",
        _ => name,
    }
}

fn project_with_agent(
    rows: &[Value],
    confirmed: &[Value],
    recovery: Vec<Action>,
    prefix: Option<&Path>,
    agent: Option<&str>,
) -> Guidance {
    let mut guidance = Guidance::default();
    for action in recovery {
        if !guidance
            .actions
            .iter()
            .any(|prior| prior.argv == action.argv)
        {
            guidance.actions.push(action);
        }
    }
    let mut uses = Vec::new();
    let mut add = |id: &str, words: &[&str], description: &str| {
        let action = Action::new(
            id,
            Kind::Use,
            words.iter().map(|word| (*word).into()).collect(),
            description,
        );
        if !uses.iter().any(|item: &Action| item.argv == action.argv) {
            uses.push(action);
        }
    };
    if let Some(agent) = agent {
        add(
            "run-agent",
            &["tmt", "run", agent],
            "Start an agent with TMT",
        );
    } else {
        add(
            "setup",
            &["tmt", "setup"],
            "Set up agents and optional features",
        );
    }
    let is_current = |name: &str| {
        rows.iter()
            .any(|row| row["name"] == name && current(row, confirmed))
    };
    let can_start_colab = rows.iter().any(|row| {
        row["name"] == "colab"
            && current(row, confirmed)
            && row["dependencies"].as_array().is_some_and(|dependencies| {
                dependencies
                    .iter()
                    .all(|dependency| dependency.as_str().is_some_and(is_current))
            })
    });
    for row in rows {
        let Some(name) = row["name"].as_str() else {
            continue;
        };
        // Missing means no managed activation, not damage or an executable from elsewhere.
        let shadowed = row["shadowedBy"]
            .as_array()
            .is_some_and(|paths| !paths.is_empty());
        if row["missing"] == true && !shadowed {
            let mut argv = vec![
                "tmt".into(),
                "extension".into(),
                "install".into(),
                name.into(),
                "--yes".into(),
            ];
            if let Some(prefix) = prefix {
                argv.extend(["--prefix".into(), prefix.to_string_lossy().into_owned()]);
            }
            let mut description = format!("Add {}", feature(name));
            let dependencies = row["dependencies"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            if !dependencies.is_empty() {
                description.push_str(&format!("; needs {}", dependencies.join(", ")));
            }
            guidance.actions.push(Action::new(
                &format!("install-{name}"),
                Kind::Install,
                argv,
                &description,
            ));
            continue;
        }
        if !row["status"].is_null() && row["status"] != "unmanaged" {
            continue;
        }
        if current(row, confirmed) {
            match name {
                "ops" => add(
                    "ops-ui",
                    &["tmt", "ops", "ui"],
                    "Open the terminal workspace",
                ),
                "colab" if can_start_colab => {}
                "remote" => {
                    if can_start_colab {
                        add(
                            "colab-serve",
                            &["tmt", "colab", "serve"],
                            "Start the browser space and its Remote door",
                        );
                    } else {
                        add(
                            "remote-serve",
                            &["tmt", "remote", "serve"],
                            "Start the browser door",
                        );
                    }
                    add(
                        "remote-pair",
                        &["tmt", "remote", "pair"],
                        "After the door starts, pair your browser (user action)",
                    );
                    add(
                        "remote-deploy",
                        &["tmt", "remote", "deploy", "firestore", "--help"],
                        "Inspect cloud sharing setup and approval",
                    );
                }
                _ => add(
                    &format!("help-{name}"),
                    &["tmt", name, "--help"],
                    &format!("Explore {}", feature(name)),
                ),
            }
        } else {
            // The local inventory has no latest-version fact. Never probe an extension or fetch one.
            add("upgrade", &["tmt", "upgrade"], "Review available updates");
            add(
                &format!("help-{name}"),
                &["tmt", name, "--help"],
                &format!("Explore {}", feature(name)),
            );
        }
    }
    // Discovery is bounded; recovery and one install per missing piece remain visible.
    let installs = guidance.actions.split_off(
        guidance
            .actions
            .iter()
            .take_while(|a| a.kind == Kind::Repair)
            .count(),
    );
    guidance.actions.extend(uses.into_iter().take(5));
    guidance.actions.extend(installs);
    guidance
}

#[cfg(test)]
#[path = "install_guidance/tests.rs"]
mod tests;
