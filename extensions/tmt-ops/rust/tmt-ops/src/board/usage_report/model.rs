//! Validated, content-free Core observations for the shared TOK range.
pub use crate::config::TokRange as Since;
use std::sync::Arc;

/// A name the report could not resolve is shown by the start of its UUID.
#[derive(Clone, Debug, PartialEq)]
pub struct Who {
    pub id: Option<String>,
    pub name: Option<String>,
    pub retired: bool,
}

/// One admitted hourly observation. Its clock is owned once by HourlySeries;
/// cached/input/output are validated at acquisition, then reduced to visible TOK.
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub tokens: u64,
    pub complete: bool,
}
impl Point {
    pub fn total(&self) -> u64 {
        self.tokens
    }
}
/// A validated 24-hour window, never a collection of independent clocks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HourlySeries {
    pub from_ms: u64,
    points: Option<Box<HourlyPoints>>,
}
/// Separate arrays avoid padding every token count with its coverage boolean.
#[derive(Clone, Debug, PartialEq)]
struct HourlyPoints {
    tokens: [u64; 24],
    complete: [bool; 24],
}
impl HourlySeries {
    pub fn new(from_ms: u64, tokens: [u64; 24], complete: [bool; 24]) -> Self {
        Self {
            from_ms,
            points: Some(Box::new(HourlyPoints { tokens, complete })),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.points.is_none()
    }
    pub fn iter(&self) -> impl Iterator<Item = Point> + '_ {
        self.points.iter().flat_map(|points| {
            points
                .tokens
                .iter()
                .zip(&points.complete)
                .map(|(&tokens, &complete)| Point { tokens, complete })
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Seat {
    /// Exact per-model input attribution, absent when the source cannot provide it.
    pub model_inputs: Option<Vec<(String, u64)>>,
    pub who: Who,
    pub input: u64,
    pub output: u64,
    pub cached: u64,
    /// Input plus output per bucket, oldest first.
    pub series: HourlySeries,
    pub covered_ms: u64,
    pub window_ms: u64,
    pub from_ms: u64,
    pub to_ms: u64,
    pub observed_at_ms: u64,
    /// Where the seat's evidence begins inside the window; None from a supplier
    /// that does not report it.
    pub starts_at_ms: Option<u64>,
    pub complete: bool,
}

impl Seat {
    /// Incomplete zero is missing evidence, not a measured zero.
    pub fn observed_total(&self) -> Option<u64> {
        (self.complete || self.total() > 0).then(|| self.total())
    }
    pub fn total(&self) -> u64 {
        self.input.saturating_add(self.output)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Unavailable {
    pub who: Who,
    pub reason: String,
}

/// Content-free volume. Only talk/reply are messages; retrieval and acknowledgement
/// counts remain available without silently becoming messages in the matrix.
#[derive(Clone, Debug, PartialEq)]
pub struct Pair {
    pub sender: Who,
    pub recipient: Who,
    pub kind: String,
    pub count: u64,
}

impl Pair {
    pub fn is_message(&self) -> bool {
        matches!(self.kind.as_str(), "talk" | "reply")
    }
}

/// The messages one participant sent another in the window. An endpoint the report could not
/// identify is `None`, which sorts before every id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flow {
    pub sender: Option<Arc<str>>,
    pub recipient: Option<Arc<str>>,
    pub count: u64,
}

/// Message volume of the window, flat and sorted by `(sender, recipient)`. Ids are shared
/// between the flows that name them, so a clone copies the vector and bumps refcounts instead
/// of allocating every id again.
#[derive(Clone, Debug, PartialEq)]
pub struct Traffic {
    pub maximum: u64,
    pub since_ms: u64,
    pub observed_at_ms: u64,
    flows: Vec<Flow>,
}

impl Traffic {
    /// Normalize once at acquisition. Paint and each visible row only look up
    /// an admitted sender's recipients, rather than rescanning the full flow.
    pub fn from_pairs(
        since_ms: u64,
        observed_at_ms: u64,
        pairs: impl IntoIterator<Item = Pair>,
    ) -> Result<Self, String> {
        let mut ids = std::collections::HashMap::<String, Arc<str>>::new();
        let mut intern = |id: Option<String>| {
            id.map(|id| {
                ids.entry(id)
                    .or_insert_with_key(|id| Arc::from(id.as_str()))
                    .clone()
            })
        };
        let mut flows: Vec<Flow> = pairs
            .into_iter()
            .filter(Pair::is_message)
            .map(|pair| Flow {
                sender: intern(pair.sender.id),
                recipient: intern(pair.recipient.id),
                count: pair.count,
            })
            .collect();
        flows.sort_by(|a, b| (&a.sender, &a.recipient).cmp(&(&b.sender, &b.recipient)));
        let mut summed: Vec<Flow> = Vec::with_capacity(flows.len());
        for flow in flows {
            match summed.last_mut() {
                Some(last) if last.sender == flow.sender && last.recipient == flow.recipient => {
                    last.count = last
                        .count
                        .checked_add(flow.count)
                        .ok_or("Exchange message count overflow.")?;
                }
                _ => summed.push(flow),
            }
        }
        let maximum = summed.iter().map(|flow| flow.count).max().unwrap_or(0);
        Ok(Self {
            maximum,
            since_ms,
            observed_at_ms,
            flows: summed,
        })
    }

    /// Every flow, ordered by sender and then recipient.
    pub fn flows(&self) -> &[Flow] {
        &self.flows
    }

    /// What one sender sent, ordered by recipient; empty for a sender with no flow.
    pub fn recipients_of(&self, sender: Option<&str>) -> &[Flow] {
        let start = self
            .flows
            .partition_point(|flow| flow.sender.as_deref() < sender);
        let end = self
            .flows
            .partition_point(|flow| flow.sender.as_deref() <= sender);
        &self.flows[start..end]
    }

    /// The count one named sender sent one named recipient, when there is such a flow.
    pub fn count(&self, sender: &str, recipient: &str) -> Option<u64> {
        let key = (Some(sender), Some(recipient));
        self.flows
            .binary_search_by(|flow| (flow.sender.as_deref(), flow.recipient.as_deref()).cmp(&key))
            .ok()
            .map(|index| self.flows[index].count)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    pub section: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub since: Since,
    pub observed_at_ms: u64,
    /// The start of the window, None when no identity could be read.
    pub window_from_ms: Option<u64>,
    pub seats: Vec<Seat>,
    pub unavailable: Vec<Unavailable>,
    /// None when the supplier could not read the flow.
    pub exchange: Option<Traffic>,
    pub warnings: Vec<Warning>,
}

impl Report {
    /// Acquisition keeps UUIDs sorted once; visible lookups remain logarithmic.
    pub fn seat(&self, id: &str) -> Option<&Seat> {
        self.seats
            .binary_search_by(|seat| seat.who.id.as_deref().cmp(&Some(id)))
            .ok()
            .map(|index| &self.seats[index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn pair(sender: Option<&str>, recipient: Option<&str>, kind: &str, count: u64) -> Pair {
        let who = |id: Option<&str>| Who {
            id: id.map(str::to_owned),
            name: None,
            retired: false,
        };
        Pair {
            sender: who(sender),
            recipient: who(recipient),
            kind: kind.into(),
            count,
        }
    }

    /// Input in no useful order: duplicates, unknown endpoints, ids that sort in different
    /// ways by length and case, and kinds that are not messages.
    fn mixed() -> Vec<Pair> {
        vec![
            pair(Some("b"), Some("a"), "talk", 2),
            pair(Some("a"), Some("b"), "reply", 5),
            pair(None, Some("a"), "talk", 1),
            pair(Some("aa"), Some("a"), "talk", 7),
            pair(Some("b"), Some("a"), "reply", 3),
            pair(Some("B"), None, "talk", 4),
            pair(Some("a"), Some("b"), "ack", 100),
            pair(Some("a"), None, "talk", 6),
            pair(None, None, "talk", 9),
            pair(Some("a"), Some("aa"), "talk", 0),
        ]
    }

    /// The nested map this representation replaces, built the way it was.
    fn reference(pairs: &[Pair]) -> BTreeMap<Option<String>, BTreeMap<Option<String>, u64>> {
        let mut messages = BTreeMap::<Option<String>, BTreeMap<Option<String>, u64>>::new();
        for pair in pairs.iter().filter(|pair| pair.is_message()) {
            *messages
                .entry(pair.sender.id.clone())
                .or_default()
                .entry(pair.recipient.id.clone())
                .or_default() += pair.count;
        }
        messages
    }

    #[test]
    fn flows_iterate_in_the_order_the_nested_map_did_and_sum_duplicates() {
        let pairs = mixed();
        let traffic = Traffic::from_pairs(0, 1, pairs.clone()).unwrap();
        let expected: Vec<_> = reference(&pairs)
            .into_iter()
            .flat_map(|(sender, recipients)| {
                recipients
                    .into_iter()
                    .map(move |(recipient, count)| (sender.clone(), recipient, count))
            })
            .collect();
        let actual: Vec<_> = traffic
            .flows()
            .iter()
            .map(|flow| {
                (
                    flow.sender.as_deref().map(str::to_owned),
                    flow.recipient.as_deref().map(str::to_owned),
                    flow.count,
                )
            })
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(traffic.maximum, 9);
        assert!(actual[0].0.is_none(), "unknown endpoints sort first");
    }

    #[test]
    fn lookups_follow_the_sorted_flows() {
        let traffic = Traffic::from_pairs(0, 1, mixed()).unwrap();
        let of = |sender: Option<&str>| -> Vec<(Option<String>, u64)> {
            traffic
                .recipients_of(sender)
                .iter()
                .map(|flow| (flow.recipient.as_deref().map(str::to_owned), flow.count))
                .collect()
        };
        assert_eq!(
            of(Some("a")),
            [(None, 6), (Some("aa".into()), 0), (Some("b".into()), 5)]
        );
        assert_eq!(of(Some("b")), [(Some("a".into()), 5)]);
        assert_eq!(of(None), [(None, 9), (Some("a".into()), 1)]);
        assert!(of(Some("zz")).is_empty(), "a sender with no flow");
        assert!(of(Some("")).is_empty());
        assert_eq!(traffic.count("a", "b"), Some(5));
        assert_eq!(
            traffic.count("a", "aa"),
            Some(0),
            "a recorded zero is not missing"
        );
        assert_eq!(traffic.count("b", "b"), None);
        assert_eq!(traffic.count("zz", "a"), None);
    }

    #[test]
    fn summing_overflows_as_it_did_in_any_arrival_order() {
        let pairs = [
            pair(Some("a"), Some("b"), "talk", u64::MAX),
            pair(Some("c"), Some("d"), "talk", 1),
            pair(Some("a"), Some("b"), "reply", 1),
        ];
        assert!(Traffic::from_pairs(0, 1, pairs.clone()).is_err());
        assert!(Traffic::from_pairs(0, 1, pairs.into_iter().rev()).is_err());
        let fits = [
            pair(Some("a"), Some("b"), "talk", u64::MAX - 1),
            pair(Some("a"), Some("b"), "reply", 1),
        ];
        assert_eq!(Traffic::from_pairs(0, 1, fits).unwrap().maximum, u64::MAX);
    }

    #[test]
    fn one_recipient_per_sender_shares_ids_between_flows_and_clones() {
        let ids: Vec<String> = (0..1_000).map(|n| format!("id-{n:04}")).collect();
        let traffic = Traffic::from_pairs(
            0,
            1,
            ids.windows(2)
                .map(|w| pair(Some(w[0].as_str()), Some(w[1].as_str()), "talk", 1)),
        )
        .unwrap();
        assert_eq!(traffic.flows().len(), 999);
        // Each middle id is both a recipient and the next sender: one allocation.
        let flows = traffic.flows();
        assert!(Arc::ptr_eq(
            flows[0].recipient.as_ref().unwrap(),
            flows[1].sender.as_ref().unwrap()
        ));
        let copy = traffic.clone();
        assert!(Arc::ptr_eq(
            flows[5].sender.as_ref().unwrap(),
            copy.flows()[5].sender.as_ref().unwrap()
        ));
        assert_eq!(traffic.recipients_of(Some("id-0500")).len(), 1);
    }
}
