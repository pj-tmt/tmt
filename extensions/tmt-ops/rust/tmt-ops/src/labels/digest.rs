//! Digest's status v1 projection, acquired by the existing labels reader.
//! Labels/actions retain their supplier-owned parser and are keyed by the same
//! UUIDs in the enclosing snapshot. No scheduling or delivery decisions live here.
use super::Rows;
use crate::config::DigestWindow;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const SAFE: u64 = 9_007_199_254_740_991;
const MAX_TEXT: usize = 512;
const MAX_PRICED_MODELS: usize = 64;

/// Only this supplier owns its window grammar; other label sources are unchanged.
pub(super) fn status_arguments(source: &str, window: DigestWindow) -> Vec<&str> {
    let mut args = vec![source, "status"];
    if source == "digest" && window != DigestWindow::Day {
        args.extend(["--window", window.argument()]);
    }
    args
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Interval,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingSource {
    Default,
    Member,
}

/// Supplier-owned token estimate and coverage. Ops never recomputes its inputs.
/// Cost and percent are independent valuations of that figure: each is absent
/// when the supplier omitted it or it failed validation, never the token figure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Savings {
    pub method: String,
    pub window: crate::config::DigestWindow,
    pub from_ms: u64,
    pub through_ms: u64,
    pub tokens: Option<u64>,
    pub complete: bool,
    pub gap: Option<String>,
    pub cost: Option<Cost>,
    pub percent: Option<Percent>,
}

/// A list-price quote of the token figure at one evidenced model's standard
/// uncached input rate; an estimate, never a bill or money saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cost {
    pub usd_micros: u64,
    pub model_id: String,
    pub model_observed_at_ms: u64,
    pub pricing_version: String,
    pub usd_micros_per_million_input_tokens: u64,
}

/// The share of observed input that the estimate would have avoided, over whole
/// closed minutes of the 24h or 7d window; never available for lifetime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Percent {
    pub basis_points: u16,
    pub avoided: u64,
    pub observed: u64,
    pub from_ms: u64,
    pub through_ms: u64,
}

/// The supplier's price table behind the quotes of one status document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pricing {
    pub version: String,
    pub as_of: String,
    pub basis: String,
    pub models: Vec<PricedModel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PricedModel {
    pub model_id: String,
    pub usd_micros_per_million_input_tokens: u64,
    pub source: String,
}

/// Numeric evidence is nullable: unavailable counts and times are never zero.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub mode: Mode,
    pub interval_ms: Option<u64>,
    pub setting_source: SettingSource,
    pub setting_value: String,
    pub set_by_identity_id: Option<String>,
    pub set_at_ms: Option<u64>,
    pub held_count: Option<u64>,
    pub stats_observed_at_ms: Option<u64>,
    pub next_send_at_ms: Option<u64>,
    pub time_to_send_ms: Option<u64>,
    pub savings: Option<Savings>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub observed_at_ms: u64,
    pub members: BTreeMap<String, Member>,
    /// Absent for older producers and for any shape this version does not know.
    pub pricing: Option<Pricing>,
}

impl Status {
    /// The namespace comes from the invoked command, never the response.
    /// Malformed typed evidence is unavailable; valid generic labels can remain.
    pub(super) fn from_source(source: &str, document: &Value, labels: &Rows) -> Option<Self> {
        if source != "digest" || document["version"] != 1 {
            return None;
        }
        let observed_at_ms = number(document.get("observedAtMs")?)?;
        let mut members = BTreeMap::new();
        for row in document["members"].as_array()? {
            let id = identity(row.get("identityId")?)?;
            // Typed and label consumers must refer to the same admitted member.
            if !labels.contains_key(id) || members.contains_key(id) {
                return None;
            }
            members.insert(id.to_owned(), Member::parse(row)?);
        }
        Some(Self {
            observed_at_ms,
            members,
            pricing: document.get("pricing").and_then(Pricing::parse),
        })
    }
}

/// Included identities retain their individual source intervals; no common
/// watermark or collection-start cohort is invented for a sum.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SavingsSummary {
    pub tokens: Option<u64>,
    pub included: usize,
    pub total: usize,
    pub earliest_from_ms: Option<u64>,
    pub latest_from_ms: Option<u64>,
    /// Total list-price quote in micro-dollars.
    pub cost: Valuation<u64>,
    /// Share of observed input in basis points, from summed numerators.
    pub percent: Valuation<u16>,
}

/// One unit's sum with its own coverage: members lacking that valuation are
/// unavailable, never zero. `conflict` means included members could not be
/// combined (different price tables, or different percent intervals).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Valuation<T> {
    pub value: Option<T>,
    pub included: usize,
    pub conflict: bool,
}

pub fn summarize_savings<'a>(
    evidence: Option<(&Status, bool)>,
    identities: impl IntoIterator<Item = &'a str>,
    window: DigestWindow,
) -> SavingsSummary {
    let identities: BTreeSet<_> = identities.into_iter().collect();
    let mut summary = SavingsSummary {
        total: identities.len(),
        ..Default::default()
    };
    let Some((status, false)) = evidence else {
        return summary;
    };
    let mut sum = Some(0u64);
    let mut costs = Vec::new();
    let mut percents = Vec::new();
    for identity in identities {
        let Some(savings) = status
            .members
            .get(identity)
            .and_then(|member| member.savings.as_ref())
            .filter(|savings| {
                savings.method == "context-input-avoided-v1" && savings.window == window
            })
        else {
            continue;
        };
        costs.extend(savings.cost.as_ref());
        percents.extend(savings.percent.as_ref());
        let Some(tokens) = savings.tokens else {
            continue;
        };
        summary.included += 1;
        sum = sum
            .and_then(|sum| sum.checked_add(tokens))
            .filter(|sum| *sum <= SAFE);
        summary.earliest_from_ms = Some(
            summary
                .earliest_from_ms
                .map_or(savings.from_ms, |value| value.min(savings.from_ms)),
        );
        summary.latest_from_ms = Some(
            summary
                .latest_from_ms
                .map_or(savings.from_ms, |value| value.max(savings.from_ms)),
        );
    }
    if summary.included > 0 {
        summary.tokens = sum;
    }
    summary.cost = sum_costs(&costs);
    summary.percent = sum_percents(&percents);
    summary
}

/// Quotes combine only under one price-table version: each already used its own
/// member's evidenced model, so the sum is of list-price quotes.
fn sum_costs(costs: &[&Cost]) -> Valuation<u64> {
    let mut valuation = Valuation {
        included: costs.len(),
        ..Default::default()
    };
    if costs
        .windows(2)
        .any(|pair| pair[0].pricing_version != pair[1].pricing_version)
    {
        valuation.conflict = true;
    } else if !costs.is_empty() {
        valuation.value = costs
            .iter()
            .try_fold(0u64, |sum, cost| sum.checked_add(cost.usd_micros))
            .filter(|sum| *sum <= SAFE);
    }
    valuation
}

/// Ratios never add or average: only members measured over one exact interval
/// combine, by summing numerators and observed inputs and dividing once.
fn sum_percents(percents: &[&Percent]) -> Valuation<u16> {
    let mut valuation = Valuation {
        included: percents.len(),
        ..Default::default()
    };
    if percents
        .windows(2)
        .any(|pair| (pair[0].from_ms, pair[0].through_ms) != (pair[1].from_ms, pair[1].through_ms))
    {
        valuation.conflict = true;
    } else if !percents.is_empty() {
        let total = |field: fn(&Percent) -> u64| {
            percents
                .iter()
                .try_fold(0u64, |sum, percent| sum.checked_add(field(percent)))
        };
        valuation.value = total(|percent| percent.avoided)
            .zip(total(|percent| percent.observed))
            .and_then(|(avoided, observed)| basis_points_of(avoided, observed));
    }
    valuation
}

/// `10000 * avoided / (observed + avoided)`, half up, from wide intermediates.
/// A zero or unsafe denominator is unavailable.
fn basis_points_of(avoided: u64, observed: u64) -> Option<u16> {
    let total = avoided
        .checked_add(observed)
        .filter(|total| *total <= SAFE)?;
    if total == 0 {
        return None;
    }
    let total = u128::from(total);
    let ratio = (u128::from(avoided) * 20_000 + total) / (total * 2);
    u16::try_from(ratio).ok().filter(|ratio| *ratio <= 10_000)
}

impl Savings {
    fn parse(value: &Value) -> Option<Self> {
        let method = value["method"].as_str()?.to_owned();
        if method.is_empty() {
            return None;
        }
        let window = DigestWindow::parse(value["window"].as_str()?)?;
        let from_ms = number(value.get("fromMs")?)?;
        let through_ms = number(value.get("throughMs")?)?;
        if from_ms > through_ms {
            return None;
        }
        let tokens = nullable(value, "estimatedInputTokensAvoided")?;
        let complete = value["complete"].as_bool()?;
        let gap = match value.get("gap")? {
            Value::Null => None,
            value => Some(value.as_str().filter(|gap| !gap.is_empty())?.to_owned()),
        };
        if (gap.is_some() && tokens.is_some()) || (complete && (gap.is_some() || tokens.is_none()))
        {
            return None;
        }
        // Valuations quote the token figure, so they exist only beside it; a
        // percent also needs complete parent evidence.
        let cost = value
            .get("cost")
            .filter(|_| tokens.is_some())
            .and_then(Cost::parse);
        let percent = value
            .get("percent")
            .filter(|_| complete)
            .and_then(|percent| Percent::parse(percent, window));
        Some(Self {
            method,
            window,
            from_ms,
            through_ms,
            tokens,
            complete,
            gap,
            cost,
            percent,
        })
    }
}

impl Cost {
    fn parse(value: &Value) -> Option<Self> {
        if value.get("assumption")? != "uncached-input" {
            return None;
        }
        Some(Self {
            usd_micros: number(value.get("usdMicros")?)?,
            model_id: text(value.get("modelId")?)?,
            model_observed_at_ms: number(value.get("modelObservedAtMs")?)?,
            pricing_version: text(value.get("pricingVersion")?)?,
            usd_micros_per_million_input_tokens: number(
                value.get("usdMicrosPerMillionInputTokens")?,
            )?,
        })
    }
}

impl Percent {
    fn parse(value: &Value, window: DigestWindow) -> Option<Self> {
        // Lifetime has no percent; a supplier that sends one is not trusted.
        if window == DigestWindow::AllTime {
            return None;
        }
        let avoided = number(value.get("estimatedInputTokensAvoided")?)?;
        let observed = number(value.get("observedInputTokens")?)?;
        let from_ms = number(value.get("fromMs")?)?;
        let through_ms = number(value.get("throughMs")?)?;
        // The supplier's ratio must agree with its own numerators, since sums
        // are recomputed from them.
        let basis_points = u16::try_from(number(value.get("basisPoints")?)?).ok()?;
        if from_ms >= through_ms || Some(basis_points) != basis_points_of(avoided, observed) {
            return None;
        }
        Some(Self {
            basis_points,
            avoided,
            observed,
            from_ms,
            through_ms,
        })
    }
}

impl Pricing {
    fn parse(value: &Value) -> Option<Self> {
        let models = value.get("models")?.as_array()?;
        if models.is_empty() || models.len() > MAX_PRICED_MODELS {
            return None;
        }
        let models: Vec<_> = models
            .iter()
            .map(|model| {
                Some(PricedModel {
                    model_id: text(model.get("modelId")?)?,
                    usd_micros_per_million_input_tokens: number(
                        model.get("usdMicrosPerMillionInputTokens")?,
                    )?,
                    source: text(model.get("source")?)?,
                })
            })
            .collect::<Option<_>>()?;
        let mut ids: Vec<_> = models.iter().map(|model| &model.model_id).collect();
        ids.sort();
        ids.dedup();
        if ids.len() != models.len() {
            return None;
        }
        Some(Self {
            version: text(value.get("version")?)?,
            as_of: text(value.get("asOf")?)?,
            basis: text(value.get("basis")?)?,
            models,
        })
    }
}

/// Supplier text shown beside numbers: bounded, single-line, no control codes.
fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| {
            !text.is_empty()
                && text.chars().count() <= MAX_TEXT
                && !text.chars().any(char::is_control)
        })
        .map(str::to_owned)
}

fn number(value: &Value) -> Option<u64> {
    value.as_u64().filter(|value| *value <= SAFE)
}

fn nullable(row: &Value, field: &str) -> Option<Option<u64>> {
    match row.get(field)? {
        Value::Null => Some(None),
        value => number(value).map(Some),
    }
}

fn identity(value: &Value) -> Option<&str> {
    value.as_str().filter(|id| {
        crate::config::uuid_like(id) && !id.bytes().any(|byte| byte.is_ascii_uppercase())
    })
}

impl Member {
    fn parse(row: &Value) -> Option<Self> {
        let mode = match row["mode"].as_str()? {
            "auto" => Mode::Auto,
            "interval" => Mode::Interval,
            "off" => Mode::Off,
            _ => return None,
        };
        let interval_ms = nullable(row, "intervalMs")?;
        if match mode {
            Mode::Interval => !interval_ms.is_some_and(|ms| ms > 0),
            Mode::Auto | Mode::Off => interval_ms.is_some(),
        } {
            return None;
        }
        let setting_source = match row["settingSource"].as_str()? {
            "default" => SettingSource::Default,
            "member" => SettingSource::Member,
            _ => return None,
        };
        // Duration grammar belongs to Digest. Retain its supplied value instead
        // of interpreting it with Ops' different command-duration parser.
        let setting_value = row["settingValue"].as_str()?;
        if setting_value.is_empty()
            || setting_value.chars().any(char::is_control)
            || (setting_source == SettingSource::Default) != (setting_value == "default")
            || (setting_source == SettingSource::Member
                && match mode {
                    Mode::Auto => setting_value != "auto",
                    Mode::Off => setting_value != "off",
                    Mode::Interval => matches!(setting_value, "auto" | "off"),
                })
        {
            return None;
        }
        let set_by_identity_id = match row.get("setByIdentityId") {
            None => None,
            Some(value) => Some(identity(value)?),
        };
        let set_at_ms = match row.get("setAtMs") {
            None => None,
            Some(value) => Some(number(value)?),
        };
        if setting_source == SettingSource::Default
            && (set_by_identity_id.is_some() || set_at_ms.is_some())
        {
            return None;
        }
        let next_send_at_ms = nullable(row, "nextSendAtMs")?;
        let time_to_send_ms = nullable(row, "timeToSendMs")?;
        if mode != Mode::Interval && (next_send_at_ms.is_some() || time_to_send_ms.is_some()) {
            return None;
        }
        Some(Self {
            name: row["name"].as_str()?.to_owned(),
            mode,
            interval_ms,
            setting_source,
            setting_value: setting_value.to_owned(),
            set_by_identity_id: set_by_identity_id.map(str::to_owned),
            set_at_ms,
            held_count: nullable(row, "heldCount")?,
            stats_observed_at_ms: nullable(row, "statsObservedAtMs")?,
            next_send_at_ms,
            time_to_send_ms,
            savings: row.get("savings").and_then(Savings::parse),
        })
    }
}

#[cfg(test)]
mod tests;
