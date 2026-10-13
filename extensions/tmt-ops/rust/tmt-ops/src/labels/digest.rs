//! Digest's status v1 projection, acquired by the existing labels reader.
//! Labels/actions retain their supplier-owned parser and are keyed by the same
//! UUIDs in the enclosing snapshot. No scheduling or delivery decisions live here.
use super::Rows;
use serde_json::Value;
use std::collections::BTreeMap;

const SAFE: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Interval,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingSource {
    Default,
    Member,
}

/// Numeric evidence is nullable: unavailable counts and times are never zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub mode: Mode,
    pub interval_ms: Option<u64>,
    pub setting_source: SettingSource,
    pub setting_value: String,
    pub set_by_identity_id: Option<String>,
    pub set_at_ms: Option<u64>,
    pub held_count: Option<u64>,
    pub stats_observed_at_ms: Option<u64>,
    pub next_send_at_ms: Option<u64>,
    pub time_to_send_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub observed_at_ms: u64,
    pub members: BTreeMap<String, Member>,
}

impl Status {
    /// The namespace comes from the invoked command, never the response.
    /// Malformed typed evidence is unavailable; valid generic labels can remain.
    pub(super) fn from_source(source: &str, document: &Value, labels: &Rows) -> Option<Self> {
        if source != "digest" || document["version"] != 1 {
            return None;
        }
        let observed_at_ms = number(document.get("observedAtMs")?)?;
        let mut members = BTreeMap::new();
        for row in document["members"].as_array()? {
            let id = identity(row.get("identityId")?)?;
            // Typed and label consumers must refer to the same admitted member.
            if !labels.contains_key(id) || members.contains_key(id) {
                return None;
            }
            members.insert(id.to_owned(), Member::parse(row)?);
        }
        Some(Self {
            observed_at_ms,
            members,
        })
    }
}

fn number(value: &Value) -> Option<u64> {
    value.as_u64().filter(|value| *value <= SAFE)
}

fn nullable(row: &Value, field: &str) -> Option<Option<u64>> {
    match row.get(field)? {
        Value::Null => Some(None),
        value => number(value).map(Some),
    }
}

fn identity(value: &Value) -> Option<&str> {
    value.as_str().filter(|id| {
        crate::config::uuid_like(id) && !id.bytes().any(|byte| byte.is_ascii_uppercase())
    })
}

impl Member {
    fn parse(row: &Value) -> Option<Self> {
        let mode = match row["mode"].as_str()? {
            "auto" => Mode::Auto,
            "interval" => Mode::Interval,
            "off" => Mode::Off,
            _ => return None,
        };
        let interval_ms = nullable(row, "intervalMs")?;
        if match mode {
            Mode::Interval => !interval_ms.is_some_and(|ms| ms > 0),
            Mode::Auto | Mode::Off => interval_ms.is_some(),
        } {
            return None;
        }
        let setting_source = match row["settingSource"].as_str()? {
            "default" => SettingSource::Default,
            "member" => SettingSource::Member,
            _ => return None,
        };
        // Duration grammar belongs to Digest. Retain its supplied value instead
        // of interpreting it with Ops' different command-duration parser.
        let setting_value = row["settingValue"].as_str()?;
        if setting_value.is_empty()
            || setting_value.chars().any(char::is_control)
            || (setting_source == SettingSource::Default) != (setting_value == "default")
            || (setting_source == SettingSource::Member
                && match mode {
                    Mode::Auto => setting_value != "auto",
                    Mode::Off => setting_value != "off",
                    Mode::Interval => matches!(setting_value, "auto" | "off"),
                })
        {
            return None;
        }
        let set_by_identity_id = match row.get("setByIdentityId") {
            None => None,
            Some(value) => Some(identity(value)?),
        };
        let set_at_ms = match row.get("setAtMs") {
            None => None,
            Some(value) => Some(number(value)?),
        };
        if setting_source == SettingSource::Default
            && (set_by_identity_id.is_some() || set_at_ms.is_some())
        {
            return None;
        }
        let next_send_at_ms = nullable(row, "nextSendAtMs")?;
        let time_to_send_ms = nullable(row, "timeToSendMs")?;
        if mode != Mode::Interval && (next_send_at_ms.is_some() || time_to_send_ms.is_some()) {
            return None;
        }
        Some(Self {
            name: row["name"].as_str()?.to_owned(),
            mode,
            interval_ms,
            setting_source,
            setting_value: setting_value.to_owned(),
            set_by_identity_id: set_by_identity_id.map(str::to_owned),
            set_at_ms,
            held_count: nullable(row, "heldCount")?,
            stats_observed_at_ms: nullable(row, "statsObservedAtMs")?,
            next_send_at_ms,
            time_to_send_ms,
        })
    }
}

#[cfg(test)]
mod tests;
