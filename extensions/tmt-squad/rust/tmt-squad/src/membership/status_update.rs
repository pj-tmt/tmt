//! Exact raw status preview and atomic public mutation, separate from notification.
use crate::{
    config::Config,
    core::{Core, SquadError},
    me,
    send::{Intent, LeadRecipient},
    squad::Squad,
};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub squad: String,
    pub identity: String,
    pub actor: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Preview {
    pub target: Target,
    pub room: String,
    pub namespace: String,
    pub keys: [String; 2],
    pub name: String,
    pub actor_name: String,
    pub pending: Option<String>,
    pub state: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Submit {
    pub preview: Preview,
    pub clear_pending: bool,
    pub state: Option<String>,
    pub reason: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    Refused(String),
    Conflict(Result<Preview, String>),
    Unknown(String),
    Applied {
        intent: Intent,
        notification: Result<String, String>,
    },
}
fn refuse(message: &str) -> SquadError {
    SquadError::new("SQUAD_ACTION_REFUSED", message)
}
pub(crate) fn valid_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
}
fn actor(core: &Core, config: &Config, expected: &str) -> Result<String, SquadError> {
    let actor = match me::current(core, config)? {
        Some(actor) => Some(actor),
        None => me::you(None, me::caller(core)?.as_ref()),
    };
    actor
        .filter(|actor| actor.id == expected)
        .map(|actor| actor.name)
        .ok_or_else(|| refuse("The actor changed; action refused."))
}
pub(crate) fn read(core: &Core, target: &Target) -> Result<Preview, SquadError> {
    let squad = Squad::resolve(core, Some(&target.squad))?;
    let config = Config::load(core)?;
    let actor_name = actor(core, &config, &target.actor)?;
    let member = squad
        .roster(core)?
        .into_iter()
        .find(|member| member.id == target.identity)
        .ok_or_else(|| refuse("The selected UUID is no longer in this squad; action refused."))?;
    Ok(Preview {
        target: target.clone(),
        room: squad.room_id.clone(),
        namespace: squad.prefix(),
        keys: [squad.key("pending")?, squad.key("state")?],
        name: member.name,
        actor_name,
        pending: member.fields.get("pending").cloned(),
        state: member.fields.get("state").cloned(),
    })
}
fn context(core: &Core, preview: &Preview) -> Result<Config, SquadError> {
    let config = Config::load(core)?;
    actor(core, &config, &preview.target.actor)?;
    let current = Squad::resolve(core, Some(&preview.target.squad))?;
    if current.room_id != preview.room
        || current.prefix() != preview.namespace
        || [current.key("pending")?, current.key("state")?] != preview.keys
    {
        return Err(refuse(
            "The room or metadata namespace changed; action refused.",
        ));
    }
    if !current
        .roster(core)?
        .iter()
        .any(|member| member.id == preview.target.identity && member.name == preview.name)
    {
        return Err(refuse(
            "The selected identity or squad membership changed; action refused.",
        ));
    }
    Ok(config)
}
fn expected(value: &Option<String>) -> Result<Value, SquadError> {
    match value {
        None => Ok(json!("absent")),
        Some(value) if valid_value(value) => Ok(json!({"value":value})),
        Some(_) => Err(refuse(
            "Unsupported raw metadata value (empty, control characters or over 1024 UTF-8 bytes); nothing applied.",
        )),
    }
}
impl Submit {
    fn changes(&self) -> Result<(Vec<Value>, Vec<String>), SquadError> {
        if self.reason.trim().is_empty() {
            return Err(refuse("Enter a reason before applying status."));
        }
        let mut changes = Vec::new();
        let mut lines = Vec::new();
        if self.clear_pending {
            let expect = expected(&self.preview.pending)?;
            changes.push(json!({"key":self.preview.keys[0],"expect":expect,"then":"remove"}));
            if let Some(old) = &self.preview.pending {
                lines.push(format!("pending: {old} → (empty)"));
            }
        }
        if let Some(next) = &self.state {
            if !valid_value(next) {
                return Err(refuse(
                    "Replacement state must be 1–1024 UTF-8 bytes without control characters.",
                ));
            }
            let expect = expected(&self.preview.state)?;
            changes.push(json!({"key":self.preview.keys[1],"expect":expect,"then":{"set":next}}));
            if self.preview.state.as_ref() != Some(next) {
                lines.push(format!(
                    "state: {} → {next}",
                    self.preview.state.as_deref().unwrap_or("(absent)")
                ));
            }
        }
        if lines.is_empty() {
            return Err(refuse(
                "Choose Clear pending or an explicit different state. Nothing applied.",
            ));
        }
        Ok((changes, lines))
    }
}
pub(crate) fn apply(core: &Core, submit: &Submit) -> Outcome {
    let prepare = || -> Result<_, SquadError> {
        let (changes, lines) = submit.changes()?;
        let config = context(core, &submit.preview)?;
        let preview = &submit.preview;
        let text = format!(
            "{} updated board status for {}/{}\n{}\nReason: {}",
            preview.actor_name,
            preview.target.squad,
            preview.name,
            lines.join("\n"),
            submit.reason.trim()
        );
        let intent = Intent::new(
            &preview.target.actor,
            vec![LeadRecipient {
                squad: preview.target.squad.clone(),
                id: preview.target.identity.clone(),
                name: preview.name.clone(),
            }],
            "announcement",
            Some(&preview.room),
            &text,
        )?;
        Ok((config, changes, intent))
    };
    let (config, changes, mut intent) = match prepare() {
        Ok(prepared) => prepared,
        Err(error) => return Outcome::Refused(error.message),
    };
    let selected_keys: Vec<_> = changes
        .iter()
        .filter_map(|change| change["key"].as_str())
        .map(str::to_owned)
        .collect();
    let result = core.api(
        "identity.meta.apply",
        json!({
            "identityId": submit.preview.target.identity,
            "changes": changes,
        }),
    );
    match result {
        Ok(result) if result["identityId"] == submit.preview.target.identity && result["changed"] == true => {},
        Ok(_) => return Outcome::Unknown(
            "Apply outcome unknown; no notification sent. Inspect current raw metadata before starting a new update.".into()
        ),
        Err(error) if error.code == "METADATA_CONFLICT" => {
            let valid = error.current.as_ref().is_some_and(|current| {
                current.as_object().is_some_and(|map| {
                    !map.is_empty() && map.iter().all(|(key,value)| {
                        selected_keys.contains(key) && (value.is_null() || value.is_string())
                    })
                })
            });
            if !valid {
                return Outcome::Unknown("Apply returned an invalid conflict document; no notification sent or replay permitted.".into());
            }
            let current = read(core, &submit.preview.target).and_then(|current| {
                if current.room != submit.preview.room || current.namespace != submit.preview.namespace || current.keys != submit.preview.keys {
                    return Err(refuse("The room or metadata namespace changed; action refused."));
                }
                Ok(current)
            }).map_err(|error|error.message);
            return Outcome::Conflict(current);
        },
        Err(error) if matches!(error.code.as_str(),"SQUAD_CORE_UNAVAILABLE" | "API_UNAVAILABLE" | "STORAGE_UNAVAILABLE") => {
            return Outcome::Unknown(format!("Apply outcome unknown; no notification sent or replay permitted. {}",error.message));
        },
        Err(error) => return Outcome::Refused(error.message),
    }
    // Metadata is confirmed applied before any dispatch. This retained intent
    // is the only action a notification retry can execute.
    let notification = notification_result(&mut intent, core, &config);
    Outcome::Applied {
        intent,
        notification,
    }
}
fn notification_result(
    intent: &mut Intent,
    core: &Core,
    config: &Config,
) -> Result<String, String> {
    let result = intent.attempt(core, config).map_err(|error| error.message);
    if intent.accepted && !intent.queued {
        return Err(
            "Recipient unavailable; notification acceptance is settled and cannot be replayed."
                .into(),
        );
    }
    result
}

pub(crate) fn retry(core: &Core, preview: &Preview, mut intent: Intent) -> Outcome {
    let notification = context(core, preview)
        .map_err(|error| {
            format!(
                "Notification context refused; status remains applied. {}",
                error.message
            )
        })
        .and_then(|config| notification_result(&mut intent, core, &config));
    Outcome::Applied {
        intent,
        notification,
    }
}

#[cfg(test)]
mod tests;
