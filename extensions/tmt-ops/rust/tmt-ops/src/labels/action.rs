//! The one action a label can carry: choosing a value for a setting. The source
//! says which values it offers and which argv applies one; Ops checks that argv
//! against the source's namespace, takes the chosen value as one whole element
//! and spawns `tmt` with the array, never through a shell. `run` actions have no
//! consumer here, so such a label stays display-only.
use serde_json::Value;

const VALUE: &str = "{value}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub label: String,
    pub value: String,
}

/// Why an action is not offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Kind,
    ArgvShape,
    Namespace,
    Placeholder,
    Options,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choose {
    pub options: Vec<Offer>,
    /// The offered value in force now.
    pub current: String,
    argv: Vec<String>,
}

impl Choose {
    /// Reads an action. `namespace` is the source the document came from, known
    /// from the command that was run, never from the response.
    pub fn parse(namespace: &str, action: &Value) -> Result<Self, Refusal> {
        if action["kind"].as_str() != Some("choose") {
            return Err(Refusal::Kind);
        }
        let argv = action["argv"]
            .as_array()
            .filter(|argv| !argv.is_empty())
            .and_then(|argv| {
                argv.iter()
                    .map(|element| element.as_str().filter(|text| !text.contains('\0')))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or(Refusal::ArgvShape)?;
        if argv[0] != namespace {
            return Err(Refusal::Namespace);
        }
        let whole = argv.iter().filter(|element| **element == VALUE).count();
        let mentioned = argv
            .iter()
            .filter(|element| element.contains(VALUE))
            .count();
        if whole != 1 || mentioned != 1 || argv.iter().any(|element| other_placeholder(element)) {
            return Err(Refusal::Placeholder);
        }
        let options = action["options"]
            .as_array()
            .and_then(|options| options.iter().map(offer).collect::<Option<Vec<_>>>())
            .filter(|options| !options.is_empty())
            .ok_or(Refusal::Options)?;
        let current = action["current"]
            .as_str()
            .filter(|current| options.iter().any(|offer| offer.value == *current))
            .ok_or(Refusal::Options)?;
        Ok(Self {
            options,
            current: current.to_owned(),
            argv: argv.into_iter().map(str::to_owned).collect(),
        })
    }

    /// The argv that applies `value`: the one `{value}` element replaced whole.
    /// `value` may be any text, such as a custom duration, but not hold a NUL.
    pub fn argv(&self, value: &str) -> Result<Vec<String>, Refusal> {
        if value.contains('\0') {
            return Err(Refusal::ArgvShape);
        }
        Ok(self
            .argv
            .iter()
            .map(|element| if element == VALUE { value } else { element }.to_owned())
            .collect())
    }
}

fn offer(option: &Value) -> Option<Offer> {
    let label = option["label"]
        .as_str()
        .filter(|label| !label.is_empty() && !label.chars().any(char::is_control))?;
    let value = option["value"]
        .as_str()
        .filter(|value| !value.is_empty() && !value.contains('\0'))?;
    Some(Offer {
        label: label.to_owned(),
        value: value.to_owned(),
    })
}

/// A whole element shaped like a placeholder other than `{value}`.
fn other_placeholder(element: &str) -> bool {
    element != VALUE
        && element
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
            .is_some_and(|name| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            })
}

#[cfg(test)]
mod tests;
