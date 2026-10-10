//! Changing a member's digest from the board. A source's `choose` label action
//! says what may be chosen and which argv applies it. This module decides who may
//! change it (the recorded user or that squad's lead, as for `tmt ops digest`) and
//! applies the chosen value. Both steps run on the lane, never on the board thread.
use super::{
    app::{App, Choice, Compose, Effect, Hint, Menu, MenuEntry, Request, RowTarget},
    notes::sanitize,
};
use crate::{
    config::Config,
    core::Core,
    digest_command::admission,
    labels::Choose,
    management::{self, ManagementActor},
    squad::Squad,
};

/// A request to change one member's digest, before anyone has checked access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub squad: String,
    pub member: String,
    pub choose: Choose,
}

/// Access granted to `actor`: what the dropdown offers, and who applies it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Granted {
    pub ask: Ask,
    pub actor: ManagementActor,
}

/// One chosen value, ready to apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Set {
    pub actor: ManagementActor,
    pub squad: String,
    pub member: String,
    pub argv: Vec<String>,
    /// What the user chose, as shown back to them.
    pub shown: String,
}

impl Granted {
    /// The change that applies `value`, shown as `shown`; `None` when the value
    /// cannot be one argument.
    pub fn set(&self, value: &str, shown: &str) -> Option<Set> {
        Some(Set {
            actor: self.actor.clone(),
            squad: self.ask.squad.clone(),
            member: self.ask.member.clone(),
            argv: self.ask.choose.argv(value).ok()?,
            shown: sanitize(shown),
        })
    }
}

/// Checks that the person at the board may change this member's digest.
pub fn grant(core: &Core, ask: Ask) -> Result<Granted, String> {
    let config = Config::load(core).map_err(|error| error.message)?;
    let squad = Squad::resolve(core, Some(&ask.squad)).map_err(|error| error.message)?;
    let actor = management::actor(core, &config, None, &management::DIGEST)
        .map_err(admission)
        .map_err(|error| error.message)?;
    management::admit(core, &config, &squad, &actor, &management::DIGEST)
        .map_err(admission)
        .map_err(|error| error.message)?;
    Ok(Granted { ask, actor })
}

/// Applies a change after checking access again: the squad's lead may have
/// changed since the dropdown opened. A refused or failed change says so and is
/// never repeated; a lost answer says the outcome is unknown.
pub fn apply(core: &Core, set: Set) -> Result<String, String> {
    let config = Config::load(core).map_err(|error| error.message)?;
    let squad = Squad::resolve(core, Some(&set.squad)).map_err(|error| error.message)?;
    management::admit(core, &config, &squad, &set.actor, &management::DIGEST)
        .map_err(admission)
        .map_err(|error| error.message)?;
    match core.succeeds(&set.argv) {
        Ok(true) => Ok(format!("Digest for {}: {}.", set.member, set.shown)),
        Ok(false) => Err(format!(
            "Digest for {} was not changed: {} was not accepted.",
            set.member, set.shown
        )),
        Err(error) => Err(format!(
            "{} Digest for {} may or may not have changed.",
            error.message, set.member
        )),
    }
}

impl App {
    /// Starts changing the digest of the row at `index`: Core is asked, on the
    /// lane, whether the person at the board may; the dropdown opens once it says so.
    pub(super) fn digest_row(&mut self, index: usize) -> Effect {
        match self.digest_ask(index) {
            Ok(ask) => Effect::Act(Request::DigestOpen(ask)),
            Err(reason) => self.say(reason),
        }
    }

    fn digest_ask(&self, index: usize) -> Result<Ask, String> {
        if self.jobs_focus {
            return Err("Tab returns to the members; this key acts on a member row.".into());
        }
        let target = self.row_target(index).ok_or("No row is selected.")?;
        let row = self.target_row(&target).ok_or("No row is selected.")?;
        let squad = match &target {
            RowTarget::Home(target) => target.squad.clone(),
            RowTarget::Member { squad, .. } | RowTarget::Lead { squad, .. } => squad.clone(),
        };
        let (Some(_), Some(member)) = (row["id"].as_str(), row["name"].as_str()) else {
            return Err("This row is not a member.".into());
        };
        let choose = self
            .digest_choice(row)
            .ok_or_else(|| format!("{member} has no digest choices to change."))?;
        Ok(Ask {
            squad,
            member: member.to_owned(),
            choose,
        })
    }

    /// The choice the member's mode chip offers, when a source supplied one.
    pub(super) fn digest_choice(&self, row: &serde_json::Value) -> Option<Choose> {
        self.labels
            .of(row["id"].as_str()?)
            .find_map(|(_, label)| label.action.clone())
    }

    /// The dropdown: the source's choices in its order, the current one marked,
    /// then the board's own `Custom…`.
    pub(super) fn open_digest(&mut self, granted: Granted) {
        // The answer arrives off the board thread; whatever the user opened since
        // keeps the screen.
        if self.modal_open() || self.input.is_some() {
            return;
        }
        let choose = &granted.ask.choose;
        let mut entries: Vec<_> = choose
            .options
            .iter()
            .filter_map(|offer| {
                let current = if offer.value == choose.current {
                    "  (current)"
                } else {
                    ""
                };
                Some(MenuEntry {
                    key: String::new(),
                    label: format!("{}{current}", sanitize(&offer.label)),
                    choice: Choice::Digest(granted.set(&offer.value, &offer.label)?),
                })
            })
            .collect();
        let selected = choose
            .options
            .iter()
            .position(|offer| offer.value == choose.current)
            .unwrap_or_default();
        let title = format!("{} digest", granted.ask.member);
        entries.push(MenuEntry {
            key: String::new(),
            label: "Custom…".into(),
            choice: Choice::DigestCustom(Box::new(granted)),
        });
        self.menu = Some(Menu {
            row_send: None,
            link: None,
            prefill: String::new(),
            title,
            entries,
            selected,
            surface: Default::default(),
        });
    }

    /// The input behind `Custom…`; the hint says what it takes.
    pub(super) fn digest_custom(&mut self, granted: Granted) -> Effect {
        let (prompt, squad) = (
            format!("{} digest · Enter sets, Esc cancels", granted.ask.member),
            granted.ask.squad.clone(),
        );
        let effect = self.ask(prompt, Compose::Digest(Box::new(granted)), squad);
        if let Some(input) = &mut self.input {
            input.hint = Some(Hint {
                text: "A duration such as 20s, 10m or 1h; empty goes back to the default.".into(),
                error: false,
            });
        }
        effect
    }

    /// Empty text clears the member's own value, which is the source's `default`.
    pub(super) fn digest_submit(&mut self, granted: Granted, text: &str) -> Effect {
        let text = text.trim();
        let (value, shown) = if text.is_empty() {
            ("default", "default")
        } else {
            (text, text)
        };
        match granted.set(value, shown) {
            Some(set) => Effect::Act(Request::Digest(set)),
            None => self.say("That is not a value digest can take; nothing changed."),
        }
    }
}

#[cfg(test)]
mod tests;
