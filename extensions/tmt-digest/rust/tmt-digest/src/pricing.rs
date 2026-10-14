//! Offline standard base-input list prices, verified on 2026-10-14.
//! These quote an explicit uncached-input assumption, never historical bills.

use crate::history::MAX;
use serde_json::{Value, json};

pub const VERSION: &str = "2026-10-14";

struct Rate {
    model: &'static str,
    usd_micros_per_million: u64,
    source: &'static str,
}

// Only canonical models evidenced by current seats. No aliases or fallback rate.
// Standard base rates exclude cache, fast/regional/long-context modifiers and
// negotiated discounts. The model pages and Claude table are the source owners.
const TABLE: [Rate; 4] = [
    Rate {
        model: "claude-opus-5-5",
        usd_micros_per_million: 4_000_000,
        source: "https://platform.claude.com/docs/en/about-claude/pricing",
    },
    Rate {
        model: "claude-sonnet-5-5",
        usd_micros_per_million: 2_000_000,
        source: "https://platform.claude.com/docs/en/about-claude/pricing",
    },
    Rate {
        model: "gpt-6-sol",
        usd_micros_per_million: 2_000_000,
        source: "https://developers.openai.com/api/docs/models/gpt-6-sol",
    },
    Rate {
        model: "gpt-6.1-sol",
        usd_micros_per_million: 2_000_000,
        source: "https://developers.openai.com/api/docs/models/gpt-6.1-sol",
    },
];

pub fn quote(model: &str, observed_at: u64, tokens: u64) -> Option<Value> {
    if observed_at > MAX || tokens > MAX {
        return None;
    }
    let rate = TABLE.iter().find(|rate| rate.model == model)?;
    let micros =
        (u128::from(tokens) * u128::from(rate.usd_micros_per_million) + 500_000) / 1_000_000;
    let micros = u64::try_from(micros).ok().filter(|n| *n <= MAX)?;
    Some(json!({
        "usdMicros":micros,"modelId":model,"modelObservedAtMs":observed_at,
        "pricingVersion":VERSION,"usdMicrosPerMillionInputTokens":rate.usd_micros_per_million,
        "assumption":"uncached-input"
    }))
}

pub fn table() -> Value {
    json!({"version":VERSION,"asOf":"2026-10-14","basis":"standard-base-input",
        "models":TABLE.iter().map(|rate| json!({"modelId":rate.model,
            "usdMicrosPerMillionInputTokens":rate.usd_micros_per_million,
            "source":rate.source})).collect::<Vec<_>>()})
}
