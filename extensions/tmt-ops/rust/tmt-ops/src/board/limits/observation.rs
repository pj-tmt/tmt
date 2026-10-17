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

#[derive(Clone, Debug, PartialEq)]
pub(in crate::board) struct Report {
    pub observed_at_ms: u64,
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
        Some(Self {
            observed_at_ms,
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
