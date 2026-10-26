//! Typed public quota evidence, shared by listing and retained-history readers.
use super::{QuotaWindow, Window};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::board) struct Point {
    pub observed_at_ms: u64,
    pub window_minutes: u64,
    pub used_percent: f64,
    pub resets_at_ms: u64,
}
impl Point {
    pub fn balance(self) -> Window {
        Window {
            left: 100.0 - self.used_percent,
            resets_at_ms: self.resets_at_ms,
        }
    }
}

/// Which of a provider's accounts a report belongs to, as Core names it in `rateLimits.account`
/// (#5274): an opaque grouping key and a display label. The key is never shown; the label is
/// untrusted display text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct Account {
    pub key: String,
    pub label: String,
}

impl Account {
    /// The key is 12 lowercase hex digits and the label 1 to 32 of `[A-Za-z0-9._-]`, as Core
    /// writes them. Any other shape is no account.
    pub fn read(value: &Value) -> Option<Self> {
        let (key, label) = (value["key"].as_str()?, value["label"].as_str()?);
        (key.len() == 12
            && key
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            && (1..=32).contains(&label.len())
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)))
        .then(|| Self {
            key: key.into(),
            label: label.into(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(in crate::board) struct Report {
    pub observed_at_ms: u64,
    /// Absent for a provider with one implicit account and for an older Core.
    pub account: Option<Account>,
    windows: Vec<Point>,
}
impl Report {
    /// One bad window rejects this optional object, never the surrounding tokens.
    pub fn read(value: &Value) -> Option<Self> {
        const SAFE: u64 = 9_007_199_254_740_991;
        let observed_at_ms = value["observedAtMs"]
            .as_u64()
            .filter(|at| (1..=SAFE).contains(at))?;
        let raw = value["windows"].as_array()?;
        if !(1..=2).contains(&raw.len()) {
            return None;
        }
        let windows: Vec<_> = raw
            .iter()
            .map(|window| {
                Some(Point {
                    observed_at_ms,
                    window_minutes: window["windowMinutes"]
                        .as_u64()
                        .filter(|n| (1..=SAFE).contains(n))?,
                    used_percent: window["usedPercent"]
                        .as_f64()
                        .filter(|p| (0.0..=100.0).contains(p))?,
                    resets_at_ms: window["resetsAtMs"]
                        .as_u64()
                        .filter(|at| (1..=SAFE).contains(at))?,
                })
            })
            .collect::<Option<_>>()?;
        if windows
            .windows(2)
            .any(|pair| pair[0].window_minutes >= pair[1].window_minutes)
        {
            return None;
        }
        // A present but malformed account makes the report unusable: it must not fold into
        // another account's numbers.
        let account = match value.get("account") {
            None | Some(Value::Null) => None,
            Some(account) => Some(Account::read(account)?),
        };
        Some(Self {
            observed_at_ms,
            account,
            windows,
        })
    }

    pub fn window(&self, kind: QuotaWindow) -> Option<Point> {
        let mut matching = self
            .windows
            .iter()
            .copied()
            .filter(|p| kind.accepts(p.window_minutes));
        let first = matching.next()?;
        matching.next().is_none().then_some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn an_account_is_read_whole_and_a_malformed_one_makes_the_report_unusable() {
        let mut value = json!({"observedAtMs":1000,"windows":[{"windowMinutes":300,"usedPercent":12.5,"resetsAtMs":5000}]});
        assert_eq!(Report::read(&value).unwrap().account, None);
        value["account"] = json!({"key":"0123456789ab","label":"claude-work"});
        let account = Report::read(&value).unwrap().account.unwrap();
        assert_eq!(
            (account.key.as_str(), account.label.as_str()),
            ("0123456789ab", "claude-work")
        );
        for bad in [
            json!({"key":"0123456789AB","label":"claude"}),
            json!({"key":"0123456789ab","label":""}),
            json!({"key":"0123456789ab","label":"a b"}),
            json!({"key":"0123456789ab"}),
            json!("claude"),
        ] {
            value["account"] = bad.clone();
            assert_eq!(Report::read(&value), None, "{bad}");
        }
    }

    #[test]
    fn short_only_is_evidence_and_invalid_optional_objects_are_not_partial() {
        let value = json!({"observedAtMs":1000,"windows":[{"windowMinutes":300,"usedPercent":12.5,"resetsAtMs":5000}]});
        let report = Report::read(&value).unwrap();
        assert_eq!(
            report.window(QuotaWindow::Short).unwrap().balance().left,
            87.5
        );
        assert_eq!(report.window(QuotaWindow::Weekly), None);
        for invalid in [
            json!([]),
            json!([value["windows"][0].clone(), value["windows"][0].clone()]),
            json!([{"windowMinutes":300,"usedPercent":101,"resetsAtMs":5000}]),
        ] {
            let mut bad = value.clone();
            bad["windows"] = invalid;
            assert_eq!(Report::read(&bad), None);
        }
    }
}
