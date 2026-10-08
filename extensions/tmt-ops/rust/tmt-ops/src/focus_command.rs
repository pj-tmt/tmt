//! Focus grammar and management over the existing trusted Core policy seam.
use crate::{
    config::Config,
    core::{Core, SquadError},
    focus, management,
    squad::Squad,
};
use clap::{Arg, ArgMatches, Command};
use serde_json::{Value, json};
use tmt_cli_style::{Terminal, message};

const MAX_MS: u64 = 86_400_000;
pub fn grammar() -> Command {
    tmt_cli_style::command(crate::specs::FOCUS)
        .arg(
            Arg::new("member")
                .required(true)
                .help("Active member or lead"),
        )
        .arg(
            Arg::new("duration")
                .allow_hyphen_values(true)
                .help("Whole s/m/h segments, 1s through 24h; off clears; omit to show"),
        )
        .arg(crate::squad_option())
}
fn invalid() -> SquadError {
    SquadError::new(
        "SQUAD_FOCUS_DURATION_INVALID",
        "Use whole s/m/h segments from 1s through 24h (for example 30m or 1h30m), or off.",
    )
}
fn duration(text: &str) -> Result<u64, SquadError> {
    let mut total = 0u64;
    let mut number = 0u64;
    let mut digits = false;
    for byte in text.bytes() {
        if byte.is_ascii_digit() {
            digits = true;
            number = number
                .checked_mul(10)
                .and_then(|n| n.checked_add(u64::from(byte - b'0')))
                .ok_or_else(invalid)?;
        } else {
            let unit = match byte {
                b's' => 1000,
                b'm' => 60_000,
                b'h' => 3_600_000,
                _ => return Err(invalid()),
            };
            if !digits {
                return Err(invalid());
            }
            total = total
                .checked_add(number.checked_mul(unit).ok_or_else(invalid)?)
                .ok_or_else(invalid)?;
            number = 0;
            digits = false;
        }
    }
    if digits || !(1000..=MAX_MS).contains(&total) {
        return Err(invalid());
    }
    Ok(total)
}
fn admission(error: SquadError) -> SquadError {
    match error.code.as_str() {
        "CALLER_IDENTITY_AMBIGUOUS"
        | "SQUAD_SENDER_UNKNOWN"
        | "NAME_NOT_FOUND"
        | "SQUAD_FOCUS_PERMISSION_DENIED" => SquadError::hinted(
            "SQUAD_FOCUS_PERMISSION_DENIED",
            &error.message,
            " ",
            "Ask the recorded user or this squad's current lead to manage focus.",
        ),
        _ => error,
    }
}
pub fn run(core: &Core, config: &Config, matches: &ArgMatches) -> Result<Value, SquadError> {
    let input = matches.get_one::<String>("duration").map(String::as_str);
    let window = input
        .filter(|input| *input != "off")
        .map(duration)
        .transpose()?;
    let squad = Squad::resolve(core, matches.get_one::<String>("squad").map(String::as_str))?;
    let actor = management::actor(core, config, None, &management::FOCUS).map_err(admission)?;
    let roster =
        management::admit(core, config, &squad, &actor, &management::FOCUS).map_err(admission)?;
    let target = management::member(
        core,
        matches.get_one::<String>("member").expect("required"),
        &management::FOCUS,
    )?;
    if !roster.iter().any(|member| member.id == target.id) {
        return Err(SquadError::new(
            "SQUAD_NOT_A_MEMBER",
            "The focus target must be an active member of this squad.",
        ));
    }
    let shown = focus::read_policies(core, std::slice::from_ref(&target.id))?;
    let policy = &shown[&target.id];
    if input.is_none() {
        let mut document = policy_document(policy);
        document["member"] = json!(target.name);
        return Ok(document);
    }
    let owner = config.me_id()?.ok_or_else(|| {
        SquadError::hinted(
            "SQUAD_FOCUS_OWNER_REQUIRED",
            "No owner UUID is recorded.",
            " ",
            "Record the saved owner with tmt squad me <name>.",
        )
    })?;
    let mut fields = json!({"identityId":target.id,"ownerIdentityId":owner,
        "setterIdentityId":actor.id,"expectedRevision":policy.revision});
    let operation = if let Some(window) = window {
        fields["untilMs"] = json!(
            crate::status::now_ms()
                .checked_add(window)
                .filter(|n| *n <= 9_007_199_254_740_991)
                .ok_or_else(invalid)?
        );
        "focus.policy.set"
    } else {
        "focus.policy.clear"
    };
    let mut document = core.api(operation, fields).map_err(|error| {
        if error.code == "FOCUS_REVISION_CONFLICT" {
            SquadError::hinted(&error.code, &error.message, " ", "Reload and retry.")
        } else {
            error
        }
    })?;
    document["member"] = json!(target.name);
    Ok(document)
}
fn policy_document(policy: &focus::Policy) -> Value {
    let mut value = policy.row();
    value["identityId"] = json!(policy.identity_id);
    value["revision"] = json!(policy.revision);
    value
}
pub fn text(document: &Value, terminal: Terminal) -> String {
    let mut bytes = Vec::new();
    let line = if document["active"] == true {
        let row = json!({"focus":document});
        format!(
            "{}: {}",
            document["member"].as_str().unwrap_or("–"),
            focus::label(&row, crate::status::now_ms()).unwrap_or_else(|| "focus off".into())
        )
    } else {
        format!("{}: focus off", document["member"].as_str().unwrap_or("–"))
    };
    let _ = message::success(&mut bytes, terminal, &line);
    String::from_utf8(bytes).unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn human_policy_names_the_member_instead_of_its_uuid() {
        let document = json!({"member":"worker", "identityId":"33333333-3333-4333-8333-333333333333", "active":false});
        let output = text(&document, Terminal::PLAIN);
        assert!(output.contains("worker: focus off"));
        assert!(!output.contains(document["identityId"].as_str().unwrap()));
    }
    #[test]
    fn duration_segments_have_checked_positive_bounds() {
        for (input, expected) in [
            ("1s", 1000),
            ("30m", 1_800_000),
            ("1h30m", 5_400_000),
            ("24h", MAX_MS),
            ("23h59m60s", MAX_MS),
        ] {
            assert_eq!(duration(input).unwrap(), expected, "{input}");
        }
        for input in [
            "",
            "0s",
            "0h0m",
            "-1m",
            "+1s",
            "1",
            "m",
            "1d",
            "1.5h",
            "1m 1s",
            "24h1s",
            "999999999999999999999999h",
            "1秒",
            "1h2",
        ] {
            assert_eq!(
                duration(input).unwrap_err().code,
                "SQUAD_FOCUS_DURATION_INVALID",
                "{input}"
            );
        }
    }
    #[test]
    fn members_ambiguous_and_retired_callers_never_reach_focus_writes() {
        use crate::cron_service::test_support::{Fixture, LEAD, WORKER};
        let f = Fixture::new();
        let flags = |show: bool| {
            let mut args = vec!["focus", "worker", "--squad", "product"];
            if !show {
                args.push("30m");
            }
            grammar().try_get_matches_from(args).unwrap()
        };
        for caller in [WORKER, "ambiguous", LEAD] {
            f.change_model(|m| {
                m["caller"] = json!(caller);
                m["retired"] = if caller == LEAD {
                    json!([LEAD])
                } else {
                    json!([])
                };
            });
            for show in [false, true] {
                assert_eq!(
                    run(&f.core, &f.config, &flags(show)).unwrap_err().code,
                    "SQUAD_FOCUS_PERMISSION_DENIED"
                );
            }
        }
        assert!(f.model()["calls"].as_array().unwrap().iter().all(|call| {
            !call["request"]["operation"]
                .as_str()
                .unwrap()
                .starts_with("focus.")
        }));
    }
    #[test]
    fn optional_window_preserves_known_options_and_rejects_cadence() {
        let flags = grammar()
            .try_get_matches_from(["focus", "worker", "--squad", "product"])
            .unwrap();
        assert!(flags.get_one::<String>("duration").is_none());
        assert_eq!(
            flags.get_one::<String>("squad").map(String::as_str),
            Some("product")
        );
        assert!(
            grammar()
                .try_get_matches_from(["focus", "worker", "30m", "--every", "1m"])
                .is_err()
        );
    }
}
