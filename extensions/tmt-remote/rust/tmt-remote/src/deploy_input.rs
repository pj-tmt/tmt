//! Backend-neutral input selection and presentation. Callers supply recorded
//! values; this owner never discovers resources, persists state or authorizes work.
use clap::{Arg, ArgAction, Command};
use serde::Serialize;
use std::collections::BTreeMap;

pub type Values = BTreeMap<String, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    SelectExisting,
    CreateOrAdopt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultChoice {
    MustProvide,
    Value(&'static str),
    /// A planning instruction, never a resource identifier or an authorized effect.
    DuringPlanning(&'static str),
}
impl DefaultChoice {
    pub fn description(self) -> &'static str {
        match self {
            Self::MustProvide => "none: must be given",
            Self::Value(value) | Self::DuringPlanning(value) => value,
        }
    }
}

/// Authored backend metadata, not provider replies. Order is the public display order.
pub struct InputSpec {
    pub input: &'static str,
    /// Long option without its leading `--`.
    pub flag: &'static str,
    pub kind: InputKind,
    pub default: DefaultChoice,
    /// Conditional creation description; None means this input creates nothing.
    pub creates: Option<&'static str>,
    pub cost: &'static str,
    pub permissions: &'static [&'static str],
}

#[derive(Debug, PartialEq, Eq)]
pub enum SelectedValue {
    Value(String),
    /// The backend planner must resolve and freeze this before hashing its plan.
    DuringPlanning(&'static str),
}
#[derive(Debug, PartialEq, Eq)]
pub struct SelectedInput {
    pub input: &'static str,
    pub value: SelectedValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MissingInput {
    pub input: &'static str,
    pub flag: String,
    pub default: &'static str,
    pub creates: Option<&'static str>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RequiredInputs {
    pub missing: Vec<MissingInput>,
    pub permissions: Vec<&'static str>,
    pub creates: Vec<&'static str>,
}
#[derive(Debug, PartialEq, Eq)]
pub enum InputError {
    Required(RequiredInputs),
    /// The caller maps this to its existing recorded-input conflict code.
    RecordedConflict {
        input: &'static str,
        flag: String,
    },
}

/// Recorded values precede defaults. Explicit disagreement with a record refuses;
/// defaults never move a recorded deployment, even if today's defaults changed.
pub fn select(
    manifest: &[InputSpec],
    supplied: &Values,
    recorded: &Values,
    defaults: bool,
) -> Result<Vec<SelectedInput>, InputError> {
    let mut selected = Vec::new();
    let mut missing = Vec::new();
    for spec in manifest {
        let given = supplied.get(spec.input);
        let saved = recorded.get(spec.input);
        if given.zip(saved).is_some_and(|(a, b)| a != b) {
            return Err(InputError::RecordedConflict {
                input: spec.input,
                flag: format!("--{}", spec.flag),
            });
        }
        let value = given
            .or(saved)
            .map(|value| SelectedValue::Value(value.clone()));
        let value = value.or_else(|| {
            if !defaults {
                return None;
            }
            match spec.default {
                DefaultChoice::MustProvide => None,
                DefaultChoice::Value(value) => Some(SelectedValue::Value(value.into())),
                DefaultChoice::DuringPlanning(intent) => {
                    Some(SelectedValue::DuringPlanning(intent))
                }
            }
        });
        match value {
            Some(value) => selected.push(SelectedInput {
                input: spec.input,
                value,
            }),
            None => missing.push(MissingInput {
                input: spec.input,
                flag: format!("--{}", spec.flag),
                default: spec.default.description(),
                creates: spec.creates,
            }),
        }
    }
    if missing.is_empty() {
        return Ok(selected);
    }
    let mut permissions = Vec::new();
    let mut creates = Vec::new();
    // Show the whole deployment's requirements, including inputs already named.
    // Preserve first occurrence rather than sorting away the manifest's order.
    for spec in manifest {
        for permission in spec.permissions {
            if !permissions.contains(permission) {
                permissions.push(*permission);
            }
        }
        if let Some(resource) = spec.creates
            && !creates.contains(&resource)
        {
            creates.push(resource);
        }
    }
    Err(InputError::Required(RequiredInputs {
        missing,
        permissions,
        creates,
    }))
}

/// Register on an existing tmt-cli-style CommandSpec command. Backends own their
/// registration and validators; required inputs stay optional to Clap so callers
/// can emit the same typed refusal before any provider work.
pub fn register(mut command: Command, manifest: &[InputSpec]) -> Command {
    for spec in manifest {
        command = command.arg(Arg::new(spec.input).long(spec.flag).help(format!(
            "{}; --defaults: {}",
            spec.cost,
            spec.default.description()
        )));
    }
    command.arg(
        Arg::new("defaults")
            .long("defaults")
            .action(ArgAction::SetTrue)
            .help("Fill omitted deployment inputs with the listed defaults; does not authorize changes."),
    )
}

/// Build from trusted command/flag metadata and already admitted values only.
/// Equals syntax keeps leading-dash values unambiguous after shell tokenization.
pub fn defaults_hint(backend: &str, manifest: &[InputSpec], supplied: &Values) -> String {
    let mut command = format!("tmt remote deploy {}", crate::state::shell_quote(backend));
    for spec in manifest {
        if let Some(value) = supplied.get(spec.input) {
            command.push_str(&format!(
                " --{}={}",
                spec.flag,
                crate::state::shell_quote(value)
            ));
        }
    }
    command.push_str(" --defaults");
    command
}

impl RequiredInputs {
    pub fn message(&self) -> String {
        format!(
            "Missing required deployment inputs: {}.",
            self.missing
                .iter()
                .map(|input| input.flag.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
    pub fn guidance(&self, rerun: &str) -> String {
        let mut lines = Vec::new();
        for input in &self.missing {
            lines.push(format!(
                "{}: --defaults uses {}; would create: {}.",
                input.flag,
                input.default,
                input.creates.unwrap_or("nothing")
            ));
        }
        lines.push("Needs these permissions".into());
        lines.extend(self.permissions.iter().map(|value| format!("  {value}")));
        if self.permissions.is_empty() {
            lines.push("  None.".into());
        }
        lines.push("Would create".into());
        lines.extend(self.creates.iter().map(|value| format!("  {value}")));
        if self.creates.is_empty() {
            lines.push("  Nothing.".into());
        }
        lines.push(format!("Try: {rerun}"));
        lines
            .push("Rerun with the flags above, or add --defaults to use the listed values.".into());
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests;
