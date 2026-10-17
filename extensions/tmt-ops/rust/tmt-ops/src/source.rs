//! Column sources: a column whose value comes from the member's public
//! projection (`from`) rather than a squad field, shown in a `format`. The
//! paths are a public contract: they name TMT's normalized member data (the
//! `ls --json` row and identity metadata), never a driver's raw state, so a
//! member that switches drivers keeps its columns. `usage.w1`–`usage.w3` instead
//! name runtime board observation windows; one-shot projections have no values.

use crate::squad::Member;
use serde_json::Value;

/// Every path `from` accepts, for the validation message and the guide.
pub const PATHS: &str = "member, presence, cwd, target, session.driver, session.model, \
     session.usage.tokens, session.usage.remaining, meta.<key>, meta.squad.<field>, \
     fields.<provided field>, usage.w1, usage.w2, usage.w3 (board only)";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Origin {
    Member,
    Presence,
    Cwd,
    Target,
    SessionDriver,
    SessionModel,
    UsageTokens,
    ObservedWindow(usize),
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
            "usage.w1" => Origin::ObservedWindow(0),
            "usage.w2" => Origin::ObservedWindow(1),
            "usage.w3" => Origin::ObservedWindow(2),
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

    /// Runtime board observations have no value in a one-shot member projection.
    pub fn window(&self) -> Option<usize> {
        match self.origin {
            Origin::ObservedWindow(index) => Some(index),
            _ => None,
        }
    }

    pub fn board_only(&self) -> bool {
        self.window().is_some()
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
            Origin::SessionModel => seen["resume"]["model"]
                .as_str()
                .map(|name| model_name(name).into()),
            Origin::ObservedWindow(_) => None,
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

/// Best-effort family labels shared by member columns and the home dashboard.
/// Match complete name components so an unrelated name such as `solar` survives.
pub(crate) fn model_name(name: &str) -> &str {
    const FAMILIES: &[&str] = &["opus", "sonnet", "haiku", "fable", "sol", "astra", "luna"];
    name.split(|character: char| !character.is_ascii_alphanumeric())
        .find_map(|component| {
            FAMILIES
                .iter()
                .copied()
                .find(|family| component.eq_ignore_ascii_case(family))
        })
        .unwrap_or(name)
}

/// A number, whether core sent one or the value is numeric text.
fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
        .filter(|number: &f64| number.is_finite())
}

/// A value in `format`. A value the format cannot read shows as it is.
pub(crate) fn render_value(value: &Value, format: Format, now_ms: u64) -> Option<String> {
    let text = || match value {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    };
    let shown: fn(f64, u64) -> String = match format {
        Format::Text => return text(),
        Format::Tokens => |number, _| tokens(number.round() as u128),
        Format::Age => |number, now_ms| age(now_ms as f64 - number),
        Format::Count => |number, _| count(number),
    };
    number(value)
        .map(|number| shown(number, now_ms))
        .or_else(text)
}

/// One token-count projection for meters, columns, charts and supplier estimates.
pub(crate) fn tokens(value: u128) -> String {
    if value < 1_000 {
        return value.to_string();
    }
    let (mut scale, mut suffix) = if value >= 1_000_000_000 {
        (1_000_000_000, "B")
    } else if value >= 1_000_000 {
        (1_000_000, "M")
    } else {
        (1_000, "k")
    };
    // Whole scaled values round half up; carry 1,000 into the next unit.
    if scale < 1_000_000_000 && value >= scale * 1_000 - scale / 2 {
        scale *= 1_000;
        suffix = if scale == 1_000_000 { "M" } else { "B" };
    }
    let whole = value / scale;
    let remainder = value % scale;
    if whole < 100 {
        // Integer arithmetic keeps large source observations independent of f64.
        let tenths = whole * 10 + (remainder * 10 + scale / 2) / scale;
        if tenths < 1_000 {
            return format!("{}.{:01}{suffix}", tenths / 10, tenths % 10);
        }
    }
    format!("{}{suffix}", whole + u128::from(remainder >= scale / 2))
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

    #[test]
    fn model_families_match_complete_components_and_unknown_names_pass_through() {
        for (name, expected) in [
            ("claude-opus-4-7", "opus"),
            ("claude-sonnet-4-5", "sonnet"),
            ("claude-haiku-4", "haiku"),
            ("CLAUDE-FABLE-5", "fable"),
            ("gpt-6.1-sol", "sol"),
            ("gpt-6-astra", "astra"),
            ("gpt-5.6-luna", "luna"),
            ("opus", "opus"),
            ("solar-model", "solar-model"),
            ("unknown-long-model", "unknown-long-model"),
            ("", ""),
        ] {
            assert_eq!(model_name(name), expected, "{name}");
            let member = member(serde_json::json!({"resume": {"model": name}}));
            assert_eq!(
                bind("session.model").value(&member, Format::Text, 0),
                (!name.is_empty()).then(|| expected.to_owned())
            );
        }
    }

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
            colors: Default::default(),
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
            "usage.w1",
            "usage.w2",
            "usage.w3",
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
            "usage.w0",
            "usage.w4",
            "usage.model",
        ] {
            assert!(
                ColumnSource::parse(path, field, provided).is_none(),
                "{path}"
            );
        }
        assert!(bind("meta.team.role").reads_metadata());
        assert!(!bind("meta.squad.state").reads_metadata());
        assert!(!bind("session.model").reads_metadata());
        assert!(bind("usage.w1").board_only());
        assert_eq!(bind("usage.w3").window(), Some(2));
        assert_eq!(
            bind("usage.w1").value(&member(Value::Null), Format::Tokens, 0),
            None
        );
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
    fn token_counts_share_precision_rounding_and_large_integer_boundaries() {
        for (value, expected) in [
            (0, "0"),
            (999, "999"),
            (1_000, "1.0k"),
            (18_200, "18.2k"),
            (99_949, "99.9k"),
            (99_950, "100k"),
            (100_000, "100k"),
            (340_000, "340k"),
            (999_499, "999k"),
            (999_500, "1.0M"),
            (1_400_000, "1.4M"),
            (120_000_000, "120M"),
            (999_500_000, "1.0B"),
            (u128::MAX, "340282366920938463463374607432B"),
        ] {
            assert_eq!(tokens(value), expected, "{value}");
            if value < (1u128 << 53) {
                assert_eq!(
                    render_value(&json!(value as u64), Format::Tokens, 0),
                    Some(expected.into()),
                    "numeric bindings share the formatter"
                );
            }
        }
        assert_eq!(
            render_value(&json!(-1), Format::Tokens, 0),
            Some("0".into())
        );
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
