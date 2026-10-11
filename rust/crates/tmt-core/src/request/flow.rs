//! Content-free exchange counts, independent of body and attention retention.

use crate::limits::MAX_JS_SAFE_INTEGER;

pub const BUCKET_MS: u64 = 1_800_000;
pub const RETENTION_MS: u64 = 30 * 86_400_000;
pub const DEFAULT_WINDOW_MS: u64 = 86_400_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FlowKind {
    Talk,
    Reply,
    Result,
    Ack,
}
impl FlowKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Talk => "talk",
            Self::Reply => "reply",
            Self::Result => "result",
            Self::Ack => "ack",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "talk" => Some(Self::Talk),
            "reply" => Some(Self::Reply),
            "result" => Some(Self::Result),
            "ack" => Some(Self::Ack),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowRow {
    pub sender_identity_id: Option<String>,
    pub recipient_identity_id: Option<String>,
    pub kind: FlowKind,
    pub bucket_start_ms: u64,
    pub count: u64,
    pub bytes: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowReport {
    pub observed_at_ms: u64,
    pub since_ms: u64,
    pub rows: Vec<FlowRow>,
}
pub fn bucket(timestamp: u64) -> u64 {
    timestamp / BUCKET_MS * BUCKET_MS
}

/// Deliberately separate from observer timeouts: this window also accepts hours/days.
pub fn parse_window(value: &str) -> Option<u64> {
    let (number, scale) = if let Some(number) = value.strip_suffix("ms") {
        (number, 1)
    } else if let Some(number) = value.strip_suffix('s') {
        (number, 1_000)
    } else if let Some(number) = value.strip_suffix('m') {
        (number, 60_000)
    } else if let Some(number) = value.strip_suffix('h') {
        (number, 3_600_000)
    } else if let Some(number) = value.strip_suffix('d') {
        (number, 86_400_000)
    } else {
        (value, 1_000)
    };
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    number
        .parse::<u64>()
        .ok()?
        .checked_mul(scale)
        .filter(|ms| (1..=RETENTION_MS).contains(ms))
}
pub fn since(now: u64, window: u64) -> Option<u64> {
    (now > 0 && now <= MAX_JS_SAFE_INTEGER && (1..=RETENTION_MS).contains(&window))
        .then(|| bucket(now.saturating_sub(window)))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utc_bucket_and_window_bounds_are_exact() {
        assert_eq!(bucket(BUCKET_MS - 1), 0);
        assert_eq!(bucket(BUCKET_MS), BUCKET_MS);
        assert_eq!(parse_window("30d"), Some(RETENTION_MS));
        assert_eq!(parse_window("24h"), Some(DEFAULT_WINDOW_MS));
        assert_eq!(parse_window("1ms"), Some(1));
        for v in [
            "0",
            "31d",
            "NaN",
            "-1",
            "1.5h",
            " 1s",
            "18446744073709551615d",
        ] {
            assert_eq!(parse_window(v), None);
        }
        assert_eq!(since(BUCKET_MS + 100, 101), Some(0));
        assert_eq!(since(BUCKET_MS + 100, 100), Some(BUCKET_MS));
    }
}
