//! Content-free exchange counts, independent of body and attention retention.

use crate::limits::MAX_JS_SAFE_INTEGER;
use std::collections::BTreeMap;

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
/// A `--bucket` duration: a whole number of the stored 30-minute buckets, within
/// the same bound as a window.
pub fn parse_bucket(value: &str) -> Option<u64> {
    parse_window(value).filter(|ms| ms.is_multiple_of(BUCKET_MS))
}
/// Sums stored rows into `bucket_ms` buckets anchored at `since_ms`; the last one
/// may be shorter. A bucket at least as long as the window holds every row, so it
/// gives one row per pair and kind. Rows keep the stored order (bucket, sender,
/// recipient, kind name); none when a total would pass the JavaScript-safe bound.
pub fn rebucket(
    rows: Vec<FlowRow>,
    since_ms: u64,
    window_ms: u64,
    bucket_ms: u64,
) -> Option<Vec<FlowRow>> {
    if bucket_ms == BUCKET_MS {
        return Some(rows);
    }
    let mut sums = BTreeMap::<_, (FlowKind, u64, u64)>::new();
    for row in rows {
        let index = if bucket_ms >= window_ms {
            0
        } else {
            row.bucket_start_ms.saturating_sub(since_ms) / bucket_ms
        };
        let start = since_ms.checked_add(index.checked_mul(bucket_ms)?)?;
        let total = sums
            .entry((
                start,
                row.sender_identity_id,
                row.recipient_identity_id,
                row.kind.as_str(),
            ))
            .or_insert((row.kind, 0, 0));
        total.1 = total.1.checked_add(row.count)?;
        total.2 = total.2.checked_add(row.bytes)?;
        if total.1 > MAX_JS_SAFE_INTEGER || total.2 > MAX_JS_SAFE_INTEGER {
            return None;
        }
    }
    Some(
        sums.into_iter()
            .map(
                |(
                    (bucket_start_ms, sender_identity_id, recipient_identity_id, _),
                    (kind, count, bytes),
                )| FlowRow {
                    sender_identity_id,
                    recipient_identity_id,
                    kind,
                    bucket_start_ms,
                    count,
                    bytes,
                },
            )
            .collect(),
    )
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

    #[test]
    fn a_bucket_is_a_whole_number_of_stored_buckets_within_the_window_bound() {
        assert_eq!(parse_bucket("30m"), Some(BUCKET_MS));
        assert_eq!(parse_bucket("1h"), Some(2 * BUCKET_MS));
        assert_eq!(parse_bucket("1800"), Some(BUCKET_MS));
        assert_eq!(parse_bucket("30d"), Some(RETENTION_MS));
        for v in ["0", "1m", "45m", "31d", "1.5h", "abc", ""] {
            assert_eq!(parse_bucket(v), None, "{v}");
        }
    }

    fn row(sender: Option<&str>, kind: FlowKind, start: u64, count: u64, bytes: u64) -> FlowRow {
        FlowRow {
            sender_identity_id: sender.map(Into::into),
            recipient_identity_id: Some("r".into()),
            kind,
            bucket_start_ms: start,
            count,
            bytes,
        }
    }

    #[test]
    fn rebucketing_sums_whole_buckets_anchored_at_since_in_stored_order() {
        let since = 100 * BUCKET_MS;
        let rows = vec![
            row(None, FlowKind::Talk, since, 1, 10),
            row(Some("a"), FlowKind::Talk, since, 1, 5),
            row(Some("a"), FlowKind::Talk, since + BUCKET_MS, 2, 7),
            row(Some("a"), FlowKind::Ack, since + BUCKET_MS, 4, 0),
            row(Some("a"), FlowKind::Talk, since + 2 * BUCKET_MS, 8, 1),
            // The current partial bucket, one stored bucket past a 1 h window.
            row(Some("a"), FlowKind::Talk, since + 4 * BUCKET_MS, 16, 2),
        ];
        let window = 4 * BUCKET_MS;
        assert_eq!(
            rebucket(rows.clone(), since, window, BUCKET_MS),
            Some(rows.clone())
        );
        // One-hour buckets: kinds stay apart, an ack sorts before a talk by name.
        let hourly = rebucket(rows.clone(), since, window, 2 * BUCKET_MS).unwrap();
        let shape = |rows: &[FlowRow]| -> Vec<(u64, Option<String>, &'static str, u64, u64)> {
            rows.iter()
                .map(|row| {
                    (
                        row.bucket_start_ms,
                        row.sender_identity_id.clone(),
                        row.kind.as_str(),
                        row.count,
                        row.bytes,
                    )
                })
                .collect()
        };
        assert_eq!(
            shape(&hourly),
            vec![
                (since, None, "talk", 1, 10),
                (since, Some("a".into()), "ack", 4, 0),
                (since, Some("a".into()), "talk", 3, 12),
                (since + 2 * BUCKET_MS, Some("a".into()), "talk", 8, 1),
                // The partial tail joins the last bucket of its own hour.
                (since + 4 * BUCKET_MS, Some("a".into()), "talk", 16, 2),
            ]
        );
        // A bucket as long as the window (or longer) is one row per pair and kind,
        // the partial tail included.
        for bucket in [window, 30 * 86_400_000] {
            let whole = rebucket(rows.clone(), since, window, bucket).unwrap();
            assert_eq!(
                shape(&whole),
                vec![
                    (since, None, "talk", 1, 10),
                    (since, Some("a".into()), "ack", 4, 0),
                    (since, Some("a".into()), "talk", 27, 15),
                ]
            );
        }
    }

    #[test]
    fn rebucketing_refuses_a_total_past_the_javascript_safe_bound() {
        let rows = vec![
            row(Some("a"), FlowKind::Reply, 0, MAX_JS_SAFE_INTEGER, 0),
            row(Some("a"), FlowKind::Reply, BUCKET_MS, 1, 0),
        ];
        assert_eq!(rebucket(rows, 0, 2 * BUCKET_MS, 2 * BUCKET_MS), None);
    }
}
