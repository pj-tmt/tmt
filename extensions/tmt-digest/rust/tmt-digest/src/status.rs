//! One read-only Core observation, projected into Digest's public status contract.

use crate::{
    core::{Core, Error},
    settings::{DigestConfig, MemberSetting, Mode, canonical_uuid},
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const MAX_NUMBER: u64 = 9_007_199_254_740_991;

pub fn execute() -> Result<Value, Error> {
    let core = Core::discover()?.until(Instant::now() + Duration::from_secs(15));
    let settings = DigestConfig::load(&core.settings_path()?)?;
    let identities = inventory(&core.json(&["identity", "ls"])?)?;
    let mut stats = Vec::with_capacity(identities.len());
    for batch in identities.chunks(256) {
        let ids: Vec<_> = batch.iter().map(|(id, _)| id.as_str()).collect();
        let document = core.api("digest.stats.show", json!({"identities":ids}));
        // Failed or mismatched observations never turn known identities into an empty list.
        let rows = document
            .as_ref()
            .ok()
            .and_then(|value| value["stats"].as_array());
        let matched = rows.filter(|rows| {
            rows.len() == ids.len()
                && rows
                    .iter()
                    .zip(&ids)
                    .all(|(row, id)| row["identityId"] == *id)
        });
        stats.extend(
            (0..batch.len()).map(|index| matched.and_then(|rows| Observation::parse(&rows[index]))),
        );
    }
    let now = crate::tick::now_ms()?;
    project(&settings, &identities, &stats, now)
}

fn invalid() -> Error {
    Error::new(
        "CORE_RESPONSE_INVALID",
        "Core returned an invalid identity inventory",
    )
}
fn inventory(document: &Value) -> Result<Vec<(String, String)>, Error> {
    let mut identities = document["identities"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(|row| {
            let id = row["id"]
                .as_str()
                .filter(|id| canonical_uuid(id))
                .ok_or_else(invalid)?;
            let name = row["name"].as_str().ok_or_else(invalid)?;
            Ok((id.to_owned(), name.to_owned()))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    identities.sort_by(|a, b| a.0.cmp(&b.0));
    if identities.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(invalid());
    }
    Ok(identities)
}

#[derive(Clone, Copy)]
struct Observation {
    held: u64,
    observed: u64,
    age: Option<u64>,
}
impl Observation {
    fn parse(value: &Value) -> Option<Self> {
        let number = |key: &str| value[key].as_u64().filter(|n| *n <= MAX_NUMBER);
        Some(Self {
            held: number("heldCount")?,
            observed: number("observedAtMs")?,
            age: if value.get("oldestHeldAgeMs")?.is_null() {
                None
            } else {
                Some(number("oldestHeldAgeMs")?)
            },
        })
    }
}

fn project(
    settings: &DigestConfig,
    identities: &[(String, String)],
    observations: &[Option<Observation>],
    now: u64,
) -> Result<Value, Error> {
    if now > MAX_NUMBER {
        return Err(Error::new(
            "DIGEST_CLOCK_INVALID",
            "Status timestamp is not JS-safe",
        ));
    }
    let members = identities
        .iter()
        .zip(observations)
        .map(|((id, name), observation)| member(settings, id, name, *observation, now))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"version":1,"observedAtMs":now,"members":members}))
}

fn member(
    settings: &DigestConfig,
    id: &str,
    name: &str,
    observation: Option<Observation>,
    now: u64,
) -> Result<Value, Error> {
    let setting = settings.member_setting(id)?;
    let mode = settings.effective(id)?;
    let inherited = setting == MemberSetting::Default;
    let value = if inherited {
        "default".into()
    } else {
        normalized(&mode)
    };
    let interval = match mode {
        Mode::Interval { milliseconds, .. } => Some(milliseconds),
        _ => None,
    };
    let held = observation.map(|stats| stats.held);
    let deadline = observation
        .filter(|stats| stats.held > 0)
        .and_then(|stats| {
            stats
                .observed
                .checked_sub(stats.age?)
                .and_then(|oldest| oldest.checked_add(interval?))
                .filter(|deadline| *deadline <= MAX_NUMBER)
        });
    let time = if interval.is_some() && held.is_some_and(|n| n >= settings.flush_count as u64) {
        Some(0)
    } else {
        deadline.map(|deadline| deadline.saturating_sub(now))
    };
    let mut labels = vec![
        json!({"text":mode_label(&mode),"colorClass":if mode == Mode::Off {"dim"} else {"text"},
        "action":choose(&settings.default,&mode,id,&value)}),
    ];
    if let Some(count) = held.filter(|n| *n > 0) {
        labels.push(label(format!("{count} held"), "muted"));
    }
    if mode != Mode::Off && held != Some(0) {
        let (text, color) = match held {
            None => ("Unknown".into(), "dim"),
            Some(_) if mode == Mode::Auto => ("When idle".into(), "muted"),
            Some(_) => match time {
                None => ("Unknown".into(), "dim"),
                Some(0) => ("Due now".into(), "waiting"),
                Some(ms) => (format!("In {}", countdown(ms)), "muted"),
            },
        };
        labels.push(label(text, color));
    }
    let mut row = json!({"identityId":id,"name":name,
        "mode":match mode { Mode::Auto => "auto", Mode::Off => "off", Mode::Interval {..} => "interval" },
        "intervalMs":interval,"settingSource":if inherited {"default"} else {"member"},
        "settingValue":value,"heldCount":held,"statsObservedAtMs":observation.map(|stats| stats.observed),
        "nextSendAtMs":deadline,"timeToSendMs":time,"savings":null,"labels":labels});
    if !inherited {
        if let Some(setter) = settings.setter(id) {
            row["setByIdentityId"] = json!(setter);
        }
        if let Some(at) = settings.set_at_ms(id) {
            row["setAtMs"] = json!(at);
        }
    }
    Ok(row)
}
fn label(text: impl Into<String>, color: &str) -> Value {
    json!({"text":text.into(),"colorClass":color,"action":null})
}
fn duration(ms: u64) -> String {
    for (unit, suffix) in [
        (86_400_000, "d"),
        (3_600_000, "h"),
        (60_000, "m"),
        (1000, "s"),
        (1, "ms"),
    ] {
        if ms.is_multiple_of(unit) {
            return format!("{}{suffix}", ms / unit);
        }
    }
    unreachable!("every integer is an exact millisecond duration")
}
fn countdown(ms: u64) -> String {
    // Round in seconds first: the JS-safe upper bound may round above MAX_NUMBER ms.
    let seconds = ms.div_ceil(1000);
    for (unit, suffix) in [(86_400, "d"), (3600, "h"), (60, "m"), (1, "s")] {
        if seconds.is_multiple_of(unit) {
            return format!("{}{suffix}", seconds / unit);
        }
    }
    unreachable!("every integer is an exact second duration")
}
fn normalized(mode: &Mode) -> String {
    match mode {
        Mode::Interval { milliseconds, .. } => duration(*milliseconds),
        _ => mode.text().into(),
    }
}
fn mode_label(mode: &Mode) -> String {
    match mode {
        Mode::Auto => "Auto".into(),
        Mode::Off => "Off".into(),
        Mode::Interval { milliseconds, .. } => format!("Every {}", duration(*milliseconds)),
    }
}
fn choose(default: &Mode, mode: &Mode, id: &str, current: &str) -> Value {
    let mut options = vec![
        json!({"label":format!("Default ({})",mode_label(default)),"value":"default"}),
        json!({"label":"Auto","value":"auto"}),
    ];
    const PRESETS: [&str; 5] = ["1m", "5m", "10m", "30m", "1h"];
    options.extend(
        PRESETS
            .iter()
            .map(|value| json!({"label":format!("Every {value}"),"value":value})),
    );
    let custom = normalized(mode);
    if matches!(mode, Mode::Interval { .. }) && !PRESETS.contains(&custom.as_str()) {
        options.push(json!({"label":format!("Every {custom} (custom)"),"value":custom}));
    }
    options.push(json!({"label":"Off","value":"off"}));
    json!({"kind":"choose","options":options,"current":current,"argv":["digest",id,"{value}"]})
}

#[cfg(test)]
#[path = "status/tests.rs"]
mod tests;
