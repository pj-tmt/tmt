//! The existing scoped Digest evidence adapted to reusable metric rows.
use crate::{
    board::{app::App, key_value, savings},
    config::SavingsUnit,
};
use ratatui::{Frame, layout::Rect};
use unicode_width::UnicodeWidthStr;

pub(super) struct Metric {
    pub key: String,
    amount: Option<String>,
    coverage: String,
    note: Option<&'static str>,
}
impl Metric {
    fn scalar(key: &str, amount: Option<String>) -> Self {
        Self {
            key: key.into(),
            amount,
            coverage: String::new(),
            note: None,
        }
    }
    /// The amount alone, or a dash while it is unreported.
    pub fn amount(&self) -> &str {
        self.amount.as_deref().unwrap_or("—")
    }
    pub fn paint(&self, frame: &mut Frame, app: &App, area: Rect) {
        let amount = self.amount.as_deref().unwrap_or("—");
        let value = format!(
            "{}{}{}",
            amount,
            self.coverage,
            self.note
                .map_or_else(String::new, |note| format!(" ({note})"))
        );
        // Keep the estimate and amount before the redundant price-basis note.
        // The label's window is never traded for a longer value.
        let value = if self.key.width() + value.width() + 3 > usize::from(area.width) {
            if let Some(amount) = amount.strip_suffix(" at list price") {
                format!("{amount}{}", self.coverage)
            } else {
                value
            }
        } else {
            value
        };
        key_value::paint(
            frame,
            app,
            area,
            &key_value::Row {
                key: &self.key,
                value: if self.amount.is_some() {
                    key_value::Value::Observed(&value)
                } else {
                    key_value::Value::Unreported(&value)
                },
            },
        );
    }
}
fn count(value: u64, exact: bool) -> Option<String> {
    if exact {
        Some(value.to_string())
    } else if value > 0 {
        Some(format!("~{value}"))
    } else {
        None
    }
}

pub(super) fn rows(app: &App, summary: &super::summary::Summary) -> [Metric; 5] {
    let held = if summary.digest_available {
        count(summary.held, summary.complete && summary.unknown_held == 0)
    } else {
        None
    };
    let exact_modes = summary.digest_available && summary.complete && summary.unknown_mode == 0;
    let modes =
        [summary.auto, summary.timed, summary.off].map(|value| count(value as u64, exact_modes));
    let modes = if summary.digest_available && modes.iter().any(Option::is_some) {
        Some(
            modes
                .iter()
                .map(|value| value.as_deref().unwrap_or("—"))
                .collect::<Vec<_>>()
                .join(" · "),
        )
    } else {
        None
    };
    let estimate = |unit| {
        let (mut amount, mut coverage, note) =
            savings::savings_value(&summary.savings, summary.window, unit);
        // The token row already names missing member coverage. An unreported
        // cost stays a dash; reported quotes keep their own distinct coverage.
        if unit == SavingsUnit::Cost && amount.is_none() {
            coverage.clear();
        }
        if !summary.complete
            && let Some(value) = amount.as_mut()
        {
            value.insert(0, '~');
        }
        Metric {
            key: format!(
                "{} {}",
                if unit == SavingsUnit::Cost {
                    "cost avoided"
                } else {
                    "ctx avoided"
                },
                summary.window.label()
            ),
            amount,
            coverage,
            note,
        }
    };
    let prices = app
        .labels
        .digest()
        .filter(|(_, stale)| !stale)
        .and_then(|(status, _)| status.pricing.as_ref())
        .map(|pricing| pricing.as_of.clone());
    [
        Metric::scalar("held", held),
        Metric::scalar("Auto · timed · Off", modes),
        estimate(SavingsUnit::Tokens),
        estimate(SavingsUnit::Cost),
        Metric::scalar("prices as of", prices),
    ]
}

#[cfg(test)]
mod tests;
