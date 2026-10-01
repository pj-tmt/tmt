//! Column sources: a column whose value comes from the member's public
//! projection (`from`) rather than a squad field, shown in a `format`. The
//! paths are a public contract: they name TMT's normalized member data (the
//! `ls --json` row and identity metadata), never a driver's raw state, so a
//! member that switches drivers keeps its columns.

use crate::squad::Member;
use serde_json::Value;

/// Every path `from` accepts, for the validation message and the guide.
pub const PATHS: &str = "member, presence, cwd, target, session.driver, session.model, \
     session.usage.tokens, session.usage.remaining, meta.<key>, meta.squad.<field>, \
     fields.<provided field>";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Origin {
    Member,
    Presence,
    Cwd,
    Target,
    SessionDriver,
    SessionModel,
    UsageTokens,
    /// The context window less what is used; only when the driver states a
    /// window.
    UsageRemaining,
    /// Any identity metadata key outside this squad's fields.
    Meta(String),
    /// `meta.squad.<field>`: this squad's `squad.<name>.<field>`.
    SquadField(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    /// A token count: `950`, `487k`, `1.2M`.
    Tokens,
    /// Time since a millisecond timestamp: `42s`, `5m`, `3h`, `2d`.
    Age,
    /// A whole number with thousands separators: `12,345`.
    Count,
}

impl Format {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "text" => Some(Self::Text),
            "tokens" => Some(Self::Tokens),
            "age" => Some(Self::Age),
            "count" => Some(Self::Count),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Tokens => "tokens",
            Self::Age => "age",
            Self::Count => "count",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnSource {
    /// As written, for `ls --json`.
    pub path: String,
    origin: Origin,
}

fn metadata_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && !key.starts_with('.')
        && !key.ends_with('.')
        && !key.contains("..")
        && key.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
}

impl ColumnSource {
    /// None for a path outside [`PATHS`]. `field` checks a squad field
    /// name; `provided` whether a field provider of that name exists.
    pub fn parse(
        path: &str,
        field: impl Fn(&str) -> bool,
        provided: impl Fn(&str) -> bool,
    ) -> Option<Self> {
        let origin = match path {
            "member" => Origin::Member,
            "presence" => Origin::Presence,
            "cwd" => Origin::Cwd,
            "target" => Origin::Target,
            "session.driver" => Origin::SessionDriver,
            "session.model" => Origin::SessionModel,
            "session.usage.tokens" => Origin::UsageTokens,
            "session.usage.remaining" => Origin::UsageRemaining,
            // A provider's value is the member's field of its name.
            path if path.starts_with("fields.") => {
                let name = &path["fields.".len()..];
                Origin::SquadField(provided(name).then(|| name.to_owned())?)
            }
            path => match path.strip_prefix("meta.")? {
                key if key.starts_with("squad.") => {
                    let name = &key["squad.".len()..];
                    Origin::SquadField(field(name).then(|| name.to_owned())?)
                }
                key => Origin::Meta(metadata_key(key).then(|| key.to_owned())?),
            },
        };
        Some(Self {
            path: path.to_owned(),
            origin,
        })
    }

    /// Whether the member needs identity metadata beyond its squad's fields.
    pub fn reads_metadata(&self) -> bool {
        matches!(self.origin, Origin::Meta(_))
    }

    /// The member's value, or None when it has none.
    fn raw(&self, member: &Member) -> Option<Value> {
        let seen = &member.seen;
        let text = |value: &Value| value.as_str().map(|text| Value::from(text.to_owned()));
        let usage = &seen["resume"]["usage"];
        match &self.origin {
            Origin::Member => Some(member.name.clone().into()),
            Origin::Presence => Some(member.presence.clone().into()),
            Origin::Cwd => text(&seen["cwd"]),
            Origin::Target => text(&seen["target"]),
            Origin::SessionDriver => text(&seen["resume"]["driver"]),
            Origin::SessionModel => text(&seen["resume"]["model"]),
            Origin::UsageTokens => usage["tokens"].as_u64().map(Value::from),
            Origin::UsageRemaining => {
                let (used, window) = (usage["tokens"].as_u64()?, usage["windowTokens"].as_u64()?);
                Some(window.saturating_sub(used).into())
            }
            Origin::Meta(key) => member.meta.get(key).cloned().map(Value::from),
            Origin::SquadField(field) => member.fields.get(field).cloned().map(Value::from),
        }
    }

    /// The member's value as a number, for sorting, when it is one.
    pub fn number(&self, member: &Member) -> Option<f64> {
        number(&self.raw(member)?)
    }

    /// The member's value shown in `format` at `now_ms`; None when missing
    /// or empty.
    pub fn value(&self, member: &Member, format: Format, now_ms: u64) -> Option<String> {
        let raw = self.raw(member)?;
        let shown = render_value(&raw, format, now_ms)?;
        (!shown.is_empty()).then_some(shown)
    }
}

/// A number, whether core sent one or the value is numeric text.
fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .filter(|number: &f64| number.is_finite())
}

/// A value in `format`. A value the format cannot read shows as it is.
fn render_value(value: &Value, format: Format, now_ms: u64) -> Option<String> {
    let text = || match value {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    };
    let shown: fn(f64, u64) -> String = match format {
        Format::Text => return text(),
        Format::Tokens => |number, _| tokens(number),
        Format::Age => |number, now_ms| age(now_ms as f64 - number),
        Format::Count => |number, _| count(number),
    };
    number(value)
        .map(|number| shown(number, now_ms))
        .or_else(text)
}

fn tokens(number: f64) -> String {
    let number = number.max(0.0);
    if number < 1_000.0 {
        format!("{number:.0}")
    } else if number < 999_500.0 {
        format!("{:.0}k", number / 1_000.0)
    } else {
        format!("{:.1}M", number / 1_000_000.0)
    }
}

fn age(elapsed_ms: f64) -> String {
    let seconds = (elapsed_ms / 1_000.0).max(0.0) as u64;
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3_600 => format!("{}m", seconds / 60),
        3_600..86_400 => format!("{}h", seconds / 3_600),
        _ => format!("{}d", seconds / 86_400),
    }
}

fn count(number: f64) -> String {
    let whole = number.round() as i64;
    let digits = whole.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if whole < 0 {
        grouped.insert(0, '-');
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn field(name: &str) -> bool {
        name == "state" || name == "pr_link"
    }

    fn provided(name: &str) -> bool {
        name == "pr_state"
    }

    fn bind(path: &str) -> ColumnSource {
        ColumnSource::parse(path, field, provided).unwrap_or_else(|| panic!("{path}"))
    }

    fn member(seen: Value) -> Member {
        Member {
            lead_marker: None,
            id: "R".into(),
            name: "rin".into(),
            lifetime: "saved".into(),
            presence: "active".into(),
            pane: Value::Null,
            activity: Value::Null,
            fields: BTreeMap::from([("state".into(), "working".into())]),
            meta: BTreeMap::from([("team.role".into(), "reviewer".into())]),
            numbers: BTreeMap::new(),
            failed: Default::default(),
            seen,
        }
    }

    #[test]
    fn only_the_documented_paths_bind() {
        for path in [
            "member",
            "presence",
            "cwd",
            "target",
            "session.driver",
            "session.model",
            "session.usage.tokens",
            "session.usage.remaining",
            "meta.team.role",
            "meta.squad.state",
            "fields.pr_state",
        ] {
            assert_eq!(bind(path).path, path);
        }
        for path in [
            "",
            "name",
            "session",
            "session.usage",
            "session.usage.windowTokens",
            "resume.model",
            "meta.",
            "meta.Team",
            "meta.a..b",
            "meta.squad.",
            "meta.squad.Bad",
            "fields.other",
            "fields.",
        ] {
            assert!(
                ColumnSource::parse(path, field, provided).is_none(),
                "{path}"
            );
        }
        assert!(bind("meta.team.role").reads_metadata());
        assert!(!bind("meta.squad.state").reads_metadata());
        assert!(!bind("session.model").reads_metadata());
    }

    /// Values come from the `ls --json` row's normalized `resume`, whatever
    /// the driver; a missing value is None, never a placeholder.
    #[test]
    fn each_path_reads_the_member_s_public_projection() {
        let seen = json!({
            "cwd": "/src/app", "target": "main:1.0",
            "resume": {"driver": "codex", "model": "gpt-5.3-codex", "session": "s",
                       "usage": {"tokens": 487_123, "windowTokens": 1_000_000, "observedAtMs": 1}},
        });
        let rin = member(seen);
        let shown = |path: &str| bind(path).value(&rin, Format::Text, 0);
        assert_eq!(shown("member").as_deref(), Some("rin"));
        assert_eq!(shown("presence").as_deref(), Some("active"));
        assert_eq!(shown("cwd").as_deref(), Some("/src/app"));
        assert_eq!(shown("target").as_deref(), Some("main:1.0"));
        assert_eq!(shown("session.driver").as_deref(), Some("codex"));
        assert_eq!(shown("session.model").as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(shown("session.usage.tokens").as_deref(), Some("487123"));
        assert_eq!(shown("session.usage.remaining").as_deref(), Some("512877"));
        assert_eq!(shown("meta.team.role").as_deref(), Some("reviewer"));
        assert_eq!(shown("meta.squad.state").as_deref(), Some("working"));

        // Claude states no window: remaining is missing, tokens are not.
        let claude = member(json!({"resume": {"driver": "claude", "usage": {"tokens": 5}}}));
        assert_eq!(
            bind("session.usage.remaining").value(&claude, Format::Tokens, 0),
            None
        );
        assert_eq!(
            bind("session.usage.tokens")
                .value(&claude, Format::Tokens, 0)
                .as_deref(),
            Some("5")
        );
        // No remembered session, no pane, no such metadata or field.
        let bare = member(Value::Null);
        for path in [
            "cwd",
            "target",
            "session.driver",
            "session.model",
            "session.usage.tokens",
            "meta.team.other",
            "meta.squad.pr_link",
        ] {
            assert_eq!(bind(path).value(&bare, Format::Text, 0), None, "{path}");
        }
    }

    #[test]
    fn formats_shorten_numbers_and_leave_other_text_alone() {
        let at = |value: Value, format: Format| render_value(&value, format, 10_000_000).unwrap();
        assert_eq!(at(json!(950), Format::Tokens), "950");
        assert_eq!(at(json!(487_123), Format::Tokens), "487k");
        assert_eq!(at(json!(999_499), Format::Tokens), "999k");
        assert_eq!(at(json!(999_500), Format::Tokens), "1.0M");
        assert_eq!(at(json!(1_234_567), Format::Tokens), "1.2M");
        assert_eq!(at(json!("487123"), Format::Tokens), "487k", "numeric text");
        assert_eq!(at(json!(10_000_000 - 42_000), Format::Age), "42s");
        assert_eq!(at(json!(10_000_000 - 300_000), Format::Age), "5m");
        assert_eq!(at(json!(10_000_000 - 3 * 3_600_000), Format::Age), "3h");
        assert_eq!(at(json!(0), Format::Age), "2h");
        assert_eq!(at(json!(20_000_000), Format::Age), "0s", "a clock ahead");
        assert_eq!(at(json!(12_345), Format::Count), "12,345");
        assert_eq!(at(json!(1_000_000), Format::Count), "1,000,000");
        assert_eq!(at(json!(-4_321), Format::Count), "-4,321");
        assert_eq!(at(json!(12), Format::Count), "12");
        assert_eq!(at(json!("n/a"), Format::Tokens), "n/a");
        assert_eq!(at(json!(1234), Format::Text), "1234");
    }
}
