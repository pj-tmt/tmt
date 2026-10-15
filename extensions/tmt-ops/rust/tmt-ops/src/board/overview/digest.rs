//! The existing scoped Digest evidence adapted to reusable metric rows.
use crate::{
    board::{app::App, digest_dashboard, m10_key_value},
    config::SavingsUnit,
    look::Look,
};
use ratatui::{buffer::Buffer, layout::Rect};

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
    pub fn paint(&self, buffer: &mut Buffer, area: Rect, look: Look) {
        let value = format!(
            "{}{}{}",
            self.amount.as_deref().unwrap_or("—"),
            self.coverage,
            self.note
                .map_or_else(String::new, |note| format!(" ({note})"))
        );
        m10_key_value::paint(
            buffer,
            area,
            &m10_key_value::Row {
                key: &self.key,
                value: if self.amount.is_some() {
                    m10_key_value::Value::Observed(&value)
                } else {
                    m10_key_value::Value::Unreported(&value)
                },
            },
            look,
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
            digest_dashboard::savings_value(&summary.savings, summary.window, unit);
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
