//! Shared active-actor and user-or-lead admission; callers retain their errors.
use crate::{
    config::Config,
    core::{Core, SquadError},
    me,
    squad::{Member, Squad},
};
use serde_json::{Value, json};

pub struct Rules {
    pub unavailable: &'static str,
    pub denied: &'static str,
    pub denial: &'static str,
    pub reference: &'static str,
}
pub const CRON: Rules = Rules {
    unavailable: "SQUAD_CRON_IDENTITY_UNAVAILABLE",
    denied: "SQUAD_CRON_PERMISSION_DENIED",
    denial: "Only the recorded user or this squad's lead can change jobs.",
    reference: "Core returned an incomplete cron reference.",
};
pub const FOCUS: Rules = Rules {
    unavailable: "SQUAD_FOCUS_PERMISSION_DENIED",
    denied: "SQUAD_FOCUS_PERMISSION_DENIED",
    denial: "Only the recorded user or this squad's lead can manage focus.",
    reference: "Core returned an incomplete focus reference.",
};
fn failure(code: &str, message: &str) -> SquadError {
    SquadError::new(code, message)
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagementActor {
    pub id: String,
    pub name: String,
}
impl From<me::Me> for ManagementActor {
    fn from(value: me::Me) -> Self {
        Self {
            id: value.id,
            name: value.name,
        }
    }
}

/// Resolve once at invocation/opening. Apply revalidates this explicit UUID;
/// an identified member or ambiguous runtime never falls back to the user.
pub fn actor(
    core: &Core,
    config: &Config,
    explicit: Option<&str>,
    rules: &Rules,
) -> Result<ManagementActor, SquadError> {
    if let Some(selector) = explicit {
        return member(core, selector, rules);
    }
    if let Some(caller) = me::caller(core)? {
        return Ok(caller.me.into());
    }
    me::current(core, config)?
        .map(ManagementActor::from)
        .ok_or_else(|| {
            failure(
                "SQUAD_SENDER_UNKNOWN",
                "Record yourself with tmt squad me <name>, or supply --identity.",
            )
        })
}
/// Resolves an identity selector to its UUID and name through public core.
/// It admits nothing: roster membership and permission stay with `apply`.
pub fn member(core: &Core, selector: &str, rules: &Rules) -> Result<ManagementActor, SquadError> {
    let shown = core.json(&["identity", "show", selector])?;
    let value = &shown["identity"];
    Ok(ManagementActor {
        id: text(value, "id", rules)?,
        name: text(value, "name", rules)?,
    })
}
pub fn text(value: &Value, key: &str, rules: &Rules) -> Result<String, SquadError> {
    value[key]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| failure("SQUAD_CORE_UNAVAILABLE", rules.reference))
}
pub fn active_identity(core: &Core, id: &str, rules: &Rules) -> Result<String, SquadError> {
    let refs = core.api("references.resolve", json!({"identityIds": [id]}))?;
    let value = &refs["identities"][0];
    if value["id"] != id || value["found"] != true || value["retired"] != false {
        return Err(failure(
            rules.unavailable,
            "The selected identity no longer exists or has retired.",
        ));
    }
    text(value, "name", rules)
}
pub fn admit(
    core: &Core,
    config: &Config,
    selected: &Squad,
    actor: &ManagementActor,
    rules: &Rules,
) -> Result<Vec<Member>, SquadError> {
    active_identity(core, &actor.id, rules)?;
    let roster = selected.roster(core)?;
    let user = me::current(core, config)?.is_some_and(|user| user.id == actor.id);
    if !user
        && !roster
            .iter()
            .any(|member| member.id == actor.id && member.is_lead())
    {
        return Err(failure(rules.denied, rules.denial));
    }
    Ok(roster)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cron_service::test_support::{Fixture, LEAD, USER, WORKER};
    #[test]
    fn management_admission_keeps_cron_codes_and_checks_current_lead_and_actor() {
        let f = Fixture::new();
        let squad = Squad::resolve(&f.core, Some("product")).unwrap();
        for id in [USER, LEAD] {
            let actor = member(&f.core, id, &CRON).unwrap();
            assert!(admit(&f.core, &f.config, &squad, &actor, &CRON).is_ok());
            assert!(admit(&f.core, &f.config, &squad, &actor, &FOCUS).is_ok());
        }
        let worker = member(&f.core, WORKER, &CRON).unwrap();
        let refused = admit(&f.core, &f.config, &squad, &worker, &CRON).unwrap_err();
        assert_eq!(refused.code, "SQUAD_CRON_PERMISSION_DENIED");
        assert_eq!(refused.message, CRON.denial);
        assert_eq!(
            admit(&f.core, &f.config, &squad, &worker, &FOCUS)
                .unwrap_err()
                .code,
            FOCUS.denied
        );
        f.change_model(|m| m["retired"] = serde_json::json!([LEAD]));
        let lead = ManagementActor {
            id: LEAD.into(),
            name: "Sol".into(),
        };
        assert_eq!(
            admit(&f.core, &f.config, &squad, &lead, &CRON)
                .unwrap_err()
                .code,
            CRON.unavailable
        );
        assert_eq!(
            admit(&f.core, &f.config, &squad, &lead, &FOCUS)
                .unwrap_err()
                .code,
            FOCUS.denied
        );
        f.change_model(|m| m["caller"] = serde_json::json!("ambiguous"));
        assert_eq!(
            actor(&f.core, &f.config, None, &CRON).unwrap_err().code,
            "CALLER_IDENTITY_AMBIGUOUS"
        );
    }
}
