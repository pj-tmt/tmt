//! Recent exchanges from held HOME pages, not an event log or a new reader.
use crate::board::home_leads::{Kind, State};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) struct RecentRow {
    pub request: String,
    pub kind: Kind,
    pub at_ms: Option<u64>,
    pub agent: String,
    pub preview: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::board) enum Recent {
    Unavailable,
    Observed(Vec<RecentRow>),
}

pub(super) fn project(state: &State) -> Recent {
    if !state.observations_complete() {
        return Recent::Unavailable;
    }
    let leads = state.leads.iter().filter_map(|lead| {
        let exchange = lead.exchange.as_ref()?;
        Some(RecentRow {
            request: exchange.request.clone()?,
            kind: exchange.kind,
            at_ms: exchange.since_ms,
            agent: lead.name().into(),
            preview: exchange.preview.clone(),
        })
    });
    let replies = state.replies.iter().filter_map(|reply| {
        Some(RecentRow {
            request: reply["requestId"].as_str()?.into(),
            kind: Kind::Reply,
            at_ms: reply["submittedAtMs"].as_u64(),
            agent: reply["recipientId"].as_str()?.into(),
            // Results retain metadata only; do not invent a body or response preview.
            preview: "Reply received".into(),
        })
    });
    Recent::Observed(ordered(leads.chain(replies)))
}

fn ordered(rows: impl IntoIterator<Item = RecentRow>) -> Vec<RecentRow> {
    let mut seen = BTreeSet::new();
    let mut rows: Vec<_> = rows
        .into_iter()
        .filter(|row| {
            let kind = match row.kind {
                Kind::Question => 0,
                Kind::Asked => 1,
                Kind::Reply => 2,
            };
            seen.insert((row.request.clone(), kind))
        })
        .collect();
    rows.sort_by(|a, b| {
        b.at_ms
            .cmp(&a.at_ms)
            .then_with(|| a.request.cmp(&b.request))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_kind_deduplicate_occurrences_but_not_distinct_exchanges() {
        let row = |request: &str, kind, at_ms| RecentRow {
            request: request.into(),
            kind,
            at_ms,
            agent: "held source".into(),
            preview: "safe preview".into(),
        };
        let rows = ordered([
            row("a", Kind::Asked, Some(10)),
            row("a", Kind::Asked, Some(10)),
            row("a", Kind::Reply, Some(20)),
            row("untimed", Kind::Question, None),
            row("new", Kind::Question, Some(30)),
        ]);
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows.iter()
                .map(|row| (row.request.as_str(), row.kind))
                .collect::<Vec<_>>(),
            [
                ("new", Kind::Question),
                ("a", Kind::Reply),
                ("a", Kind::Asked),
                ("untimed", Kind::Question),
            ]
        );
    }

    #[test]
    fn never_acquired_is_not_an_empty_observation() {
        assert_eq!(project(&State::default()), Recent::Unavailable);
    }
}
