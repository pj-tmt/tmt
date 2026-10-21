//! Validated, content-free Core observations for the shared TOK range.
pub use crate::config::TokRange as Since;

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

#[derive(Clone, Debug, PartialEq)]
pub struct Traffic {
    pub maximum: u64,
    pub since_ms: u64,
    pub observed_at_ms: u64,
    pub messages:
        std::collections::BTreeMap<Option<String>, std::collections::BTreeMap<Option<String>, u64>>,
}

impl Traffic {
    /// Normalize once at acquisition. Paint and each visible row only look up
    /// an admitted sender's recipients, rather than rescanning the full flow.
    pub fn from_pairs(
        since_ms: u64,
        observed_at_ms: u64,
        pairs: impl IntoIterator<Item = Pair>,
    ) -> Result<Self, String> {
        let mut messages = std::collections::BTreeMap::<
            Option<String>,
            std::collections::BTreeMap<Option<String>, u64>,
        >::new();
        for pair in pairs.into_iter().filter(Pair::is_message) {
            let count = messages
                .entry(pair.sender.id)
                .or_default()
                .entry(pair.recipient.id)
                .or_default();
            *count = count
                .checked_add(pair.count)
                .ok_or("Exchange message count overflow.")?;
        }
        let maximum = messages
            .values()
            .flat_map(|recipients| recipients.values())
            .copied()
            .max()
            .unwrap_or(0);
        Ok(Self {
            maximum,
            since_ms,
            observed_at_ms,
            messages,
        })
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
