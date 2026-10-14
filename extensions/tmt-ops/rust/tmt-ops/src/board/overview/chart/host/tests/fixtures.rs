//! Shared acquired-data fixture: one reporting UUID, 120 source buckets, no I/O.
use super::*;
use crate::board::{
    app::RateView,
    rate::{
        Input,
        history::{Seed, Slice},
    },
};
use std::collections::BTreeMap;
pub(super) const NOW: u64 = 1_791_600_000_000;
pub(super) const VALUES: [u64; 12] = [0, 1, 0, 51, 12, 72, 49, 110, 85, 24, 180, 105];

pub(super) fn populate(app: &mut App) {
    populate_values(app, VALUES);
}

pub(super) fn populate_values(app: &mut App, values: [u64; 12]) {
    let roster = &app.workspace_snapshot().unwrap().roster;
    let id = roster.members.keys().next().unwrap().clone();
    let name = roster.members[&id].name.clone();
    let room = roster
        .squads
        .iter()
        .find(|squad| squad.members.contains(&id))
        .map(|squad| squad.room_id.clone())
        .unwrap_or_default();
    let resume = crate::board::rate::tests::input(100).resumes["a"].clone();
    let input = Input {
        room,
        resumes: BTreeMap::from([(id.clone(), resume.clone())]),
        names: BTreeMap::from([(id.clone(), name)]),
    };
    let seed = Seed {
        from: NOW - 3_600_000,
        through: NOW,
        available: Some(NOW - 3_600_000),
        sampled: Some(NOW - 5_000),
        latest: resume,
        reporting: true,
        buckets: (0..120)
            .map(|index| {
                let bar = index / 10;
                let unknown = bar == 2;
                let partial = bar == 3;
                Slice {
                    from_ms: NOW - 3_600_000 + index as u64 * 30_000,
                    to_ms: NOW - 3_600_000 + (index as u64 + 1) * 30_000,
                    input_tokens: if index % 10 == 0 { values[bar] } else { 0 },
                    output_tokens: 0,
                    cached_input_tokens: 0,
                    covered_ms: if unknown {
                        0
                    } else if partial {
                        10_000
                    } else {
                        30_000
                    },
                    complete: !unknown && !partial,
                    gap: unknown || partial,
                    discontinuous: false,
                }
            })
            .collect(),
    };
    let mut view = app.view.take().unwrap();
    view.home_rate.insert(
        "chart-fixture".into(),
        RateView {
            settings: crate::config::TokenRate {
                enabled: true,
                reduced_motion: true,
                ..Default::default()
            },
            input,
            history: Some([(id, Some(seed))].into()),
        },
    );
    let mut snapshot = crate::board::app::tests::snapshot(crate::board::ALL, serde_json::json!([]));
    snapshot.tabs = app.tabs.clone();
    snapshot.view = Ok(view);
    app.apply(snapshot);
    app.shell.tab = OpsTab::Overview;
    app.focus = 0;
}
