//! Versioned content-free event inputs and the explicit separate-delivery estimate.

use super::{MAX, invalid};
use crate::core::Error;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug)]
pub enum Window {
    Day,
    Week,
    Lifetime,
}
impl Window {
    /// Every window, in the order the CLI lists them. The check below keeps it complete.
    pub const ALL: [Self; 3] = [Self::Day, Self::Week, Self::Lifetime];
    /// The window after this one in `ALL`. Its match has no wildcard: a new variant fails to
    /// compile here until it is placed, and then the check below until `ALL` lists it.
    const fn next(self) -> Option<Self> {
        match self {
            Self::Day => Some(Self::Week),
            Self::Week => Some(Self::Lifetime),
            Self::Lifetime => None,
        }
    }
    pub fn parse(value: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|window| window.text() == value)
            .ok_or_else(|| {
                let texts = Self::ALL.map(Self::text);
                let (last, rest) = texts.split_last().expect("at least one window");
                format!("Use {} or {last}", rest.join(", "))
            })
    }
    pub fn text(self) -> &'static str {
        match self {
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Lifetime => "lifetime",
        }
    }
    pub fn duration(self) -> Option<u64> {
        match self {
            Self::Day => Some(super::DAY),
            Self::Week => Some(7 * super::DAY),
            Self::Lifetime => None,
        }
    }
}

// `ALL` lists exactly the windows `next` chains from `Day`, in that order.
const _: () = {
    let mut walked = Some(Window::Day);
    let mut listed = 0;
    while let Some(window) = walked {
        assert!(
            listed < Window::ALL.len() && Window::ALL[listed] as usize == window as usize,
            "Window::ALL must list every Window, in order"
        );
        walked = window.next();
        listed += 1;
    }
    assert!(
        listed == Window::ALL.len(),
        "Window::ALL lists a window `next` never reaches"
    );
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    pub sequence: u64,
    pub created_at_ms: u64,
    pub context_tokens_at_arrival: Option<u64>,
    pub context_observed_at_ms: Option<u64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChecklistSettlement {
    pub checklist_id: String,
    pub settlement_sequence: u64,
    pub claimed_at_ms: u64,
    pub settled_at_ms: Option<u64>,
    pub outcome: String,
    pub opportunity: Option<String>,
    pub sealed_count: Option<u64>,
    pub retained_count: u64,
    pub membership_complete: bool,
    pub items: Vec<Input>,
    pub next_item_after: Option<u64>,
}
impl ChecklistSettlement {
    pub fn validate(&self) -> Result<(), Error> {
        if !crate::settings::canonical_uuid(&self.checklist_id)
            || self.settlement_sequence == 0
            || [
                Some(self.settlement_sequence),
                Some(self.claimed_at_ms),
                self.settled_at_ms,
                self.sealed_count,
                Some(self.retained_count),
                self.next_item_after,
            ]
            .into_iter()
            .flatten()
            .any(|n| n > MAX)
            || !matches!(
                self.outcome.as_str(),
                "delivered" | "definitely_unsent" | "uncertain"
            )
            || self
                .opportunity
                .as_deref()
                .is_some_and(|v| !matches!(v, "turn_boundary" | "idle"))
            || (self.outcome == "delivered" && self.sealed_count == Some(0))
            || self.items.iter().any(|item| {
                item.sequence == 0
                    || [
                        Some(item.sequence),
                        Some(item.created_at_ms),
                        item.context_tokens_at_arrival,
                        item.context_observed_at_ms,
                    ]
                    .into_iter()
                    .flatten()
                    .any(|n| n > MAX)
            })
            || self
                .items
                .windows(2)
                .any(|pair| pair[0].sequence >= pair[1].sequence)
            || self.items.len() as u64 > self.retained_count
            || (self.membership_complete && self.sealed_count != Some(self.retained_count))
        {
            return Err(invalid("Invalid checklist history event"));
        }
        Ok(())
    }
    pub fn estimate(&self) -> Estimate {
        if self.outcome != "delivered" {
            return Estimate {
                tokens: Some(0),
                avoided: Some(0),
                gap: None,
            };
        }
        let avoided = self.sealed_count.map(|n| n.saturating_sub(1));
        if self.sealed_count == Some(1) {
            return Estimate {
                tokens: Some(0),
                avoided,
                gap: None,
            };
        }
        if !self.membership_complete
            || self.next_item_after.is_some()
            || self.sealed_count != Some(self.items.len() as u64)
        {
            return Estimate {
                tokens: None,
                avoided,
                gap: Some("incomplete_membership"),
            };
        }
        let mut tokens = 0_u64;
        for item in self.items.iter().skip(1) {
            let Some(value) = item.context_tokens_at_arrival else {
                return Estimate {
                    tokens: None,
                    avoided,
                    gap: Some("missing_context"),
                };
            };
            let Some(value) = tokens.checked_add(value).filter(|n| *n <= MAX) else {
                return Estimate {
                    tokens: None,
                    avoided,
                    gap: Some("unsafe_arithmetic"),
                };
            };
            tokens = value;
        }
        Estimate {
            tokens: Some(tokens),
            avoided,
            gap: None,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Estimate {
    pub tokens: Option<u64>,
    pub avoided: Option<u64>,
    pub gap: Option<&'static str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub identity_id: String,
    pub observed_at_ms: u64,
    pub through_sequence: u64,
    pub delivered_digests: u64,
    pub checklists: Vec<ChecklistSettlement>,
    pub next_after: Option<u64>,
}
impl Page {
    pub fn parse(value: serde_json::Value, identity: &str) -> Result<Self, Error> {
        let page: Self = serde_json::from_value(value).map_err(|e| invalid(e.to_string()))?;
        if page.identity_id != identity
            || [
                page.observed_at_ms,
                page.through_sequence,
                page.delivered_digests,
            ]
            .into_iter()
            .any(|n| n > MAX)
            || page.next_after.is_some_and(|n| n > MAX)
            || page.checklists.len() > 128
            || page.checklists.iter().map(|e| e.items.len()).sum::<usize>() > 128
            || page
                .checklists
                .windows(2)
                .any(|p| p[0].settlement_sequence >= p[1].settlement_sequence)
            || page
                .checklists
                .iter()
                .any(|e| e.settlement_sequence > page.through_sequence)
        {
            return Err(invalid("Invalid checklist history page"));
        }
        for event in &page.checklists {
            event.validate()?;
        }
        Ok(page)
    }
}
