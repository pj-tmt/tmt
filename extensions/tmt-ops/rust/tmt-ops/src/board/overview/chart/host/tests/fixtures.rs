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
    let seed = seed(values, resume);
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
    app.shell.tab = OpsTab::Agents;
    app.workspace.filter = crate::board::workspace::AgentsFilter::Agents;
    app.focus = 0;
}

fn seed(values: [u64; 12], resume: serde_json::Value) -> Seed {
    Seed {
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
                    by_driver: Vec::new(),
                }
            })
            .collect(),
    }
}

/// Three actual rate-ring inputs, including a missing watermark, supply review
/// captures through the production projection. No display-only provider totals.
pub(in crate::board) fn mixed(app: &mut App) {
    use crate::board::workspace::OpsTarget;
    use serde_json::json;
    populate(app);
    let view = app.view.as_mut().unwrap();
    let home = view.home.as_mut().unwrap();
    let first = home.workspace.roster.members.keys().next().unwrap().clone();
    let member = home.workspace.roster.members[&first].clone();
    let room = home.workspace.roster.squads[0].room_id.clone();
    let row = home.workspace.rows[&OpsTarget::Member {
        room_id: Some(room.clone()),
        identity_id: first.clone(),
    }]
        .clone();
    let rate = view.home_rate.get_mut("chart-fixture").unwrap();
    let seed = seed(VALUES, rate.input.resumes[&first].clone());
    rate.history = Some([(first.clone(), Some(seed.clone()))].into());
    for (id, name, driver) in [
        (
            "44444444-4444-4444-8444-444444444444",
            "codex-worker",
            "codex",
        ),
        (
            "55555555-5555-4555-8555-555555555555",
            "unproven-worker",
            "unknown",
        ),
    ] {
        let mut next = member.clone();
        next.id = id.into();
        next.name = name.into();
        home.workspace.roster.members.insert(id.into(), next);
        home.workspace.roster.squads[0].members.push(id.into());
        let mut row = row.clone();
        row["id"] = json!(id);
        row["name"] = json!(name);
        home.workspace.rows.insert(
            OpsTarget::Member {
                room_id: Some(room.clone()),
                identity_id: id.into(),
            },
            row,
        );
        let mut seed = seed.clone();
        if driver == "unknown" {
            seed.latest = serde_json::Value::Null;
        } else {
            seed.latest["driver"] = json!(driver);
        }
        rate.input.resumes.insert(id.into(), seed.latest.clone());
        rate.input.names.insert(id.into(), name.into());
        rate.history.as_mut().unwrap().insert(id.into(), Some(seed));
    }
    home.workspace.shell_counts = home.workspace.roster.shell_counts();
    home.summary.members = 3;
    home.summary.working = 3;
    let mut snapshot = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
    snapshot.tabs = app.tabs.clone();
    snapshot.view = Ok(app.view.take().unwrap());
    app.apply(snapshot);
    app.references = crate::board::inventory::Inventory::from_listed(
        &[],
        &Ok(json!({"identities":[
            {"id":first,"name":"worker","canonicalName":"worker"},
            {"id":"44444444-4444-4444-8444-444444444444","name":"codex-worker","canonicalName":"codex-worker"},
            {"id":"33333333-3333-4333-8333-333333333333","name":"Owner","canonicalName":"owner"}
        ]})),
    );
    use crate::board::limits::{Provider, Reading, Standing, Window};
    app.limits = Some(vec![
        Standing {
            rates: Default::default(),
            provider: Provider::named("claude"),
            latest: Some(Reading {
                observed_at_ms: NOW,
                weekly: Window {
                    left: 82.0,
                    resets_at_ms: NOW + 86_400_000,
                },
                short: Some(Window {
                    left: 98.0,
                    resets_at_ms: NOW + 3_600_000,
                }),
            }),
            burn: None,
            trend: None,
        },
        Standing {
            rates: Default::default(),
            provider: Provider::named("codex"),
            latest: Some(Reading {
                observed_at_ms: NOW,
                weekly: Window {
                    left: 88.0,
                    resets_at_ms: NOW + 576_000_000,
                },
                short: None,
            }),
            burn: None,
            trend: None,
        },
    ]);
    app.overview_rows.borrow_mut().clear();
}

/// Authoritative empty history after populated evidence must revoke every old
/// count/hit while retaining acquired driver names and separately held quotas.
pub(super) fn unreported(app: &mut App) {
    mixed(app);
    let mut view = app.view.take().unwrap();
    for rate in view.home_rate.values_mut() {
        let mut seeds = BTreeMap::new();
        for (id, resume) in &mut rate.input.resumes {
            if resume.is_object() {
                resume["consumption"] = serde_json::Value::Null;
            }
            seeds.insert(
                id.clone(),
                Some(Seed {
                    from: NOW - 3_600_000,
                    through: NOW,
                    available: None,
                    sampled: None,
                    latest: serde_json::Value::Null,
                    reporting: false,
                    buckets: Vec::new(),
                }),
            );
        }
        rate.history = Some(seeds);
    }
    let mut snapshot = crate::board::app::tests::snapshot(crate::board::ALL, serde_json::json!([]));
    snapshot.tabs = app.tabs.clone();
    snapshot.view = Ok(view);
    app.apply(snapshot);
}

/// One worker-shaped response feeds both resolutions through normal admission.
pub(super) fn retained(app: &App, selection: Selection) -> crate::board::refresh::HistoryRead {
    use crate::board::rate::history::rollup;
    let request = rollup::Request::new(
        u64::from(selection.each_minutes) * 60_000,
        u64::from(selection.range_minutes) * 60_000,
    )
    .unwrap();
    let rates = &app.view.as_ref().unwrap().home_rate;
    let mut seeds = BTreeMap::new();
    let mut retained = BTreeMap::new();
    let mut observed = BTreeMap::new();
    for rate in rates.values() {
        for (id, resume) in &rate.input.resumes {
            seeds.insert(id.clone(), Some(seed(VALUES, resume.clone())));
            observed.insert(id.clone(), resume.clone());
            let from = NOW - request.range_ms();
            retained.insert(
                id.clone(),
                Some(rollup::Seed {
                    limits: Vec::new(),
                    request,
                    through: NOW,
                    fine_through: NOW,
                    sampled: Some(NOW - 5_000),
                    latest: resume.clone(),
                    buckets: (0..request.buckets())
                        .map(|index| {
                            let missing = index == 0;
                            let partial = index == 1;
                            Slice {
                                from_ms: from + index as u64 * request.each_ms(),
                                to_ms: from + (index as u64 + 1) * request.each_ms(),
                                input_tokens: if missing { 0 } else { (index as u64 + 1) * 17 },
                                output_tokens: 0,
                                cached_input_tokens: 0,
                                covered_ms: if missing {
                                    0
                                } else if partial {
                                    300_000
                                } else {
                                    request.each_ms()
                                },
                                complete: !missing && !partial,
                                gap: missing || partial,
                                discontinuous: false,
                                by_driver: Vec::new(),
                            }
                        })
                        .collect(),
                }),
            );
        }
    }
    crate::board::refresh::HistoryRead {
        owner: crate::board::ALL.into(),
        targets: rates
            .iter()
            .map(|(name, rate)| (name.clone(), (rate.settings, rate.input.clone())))
            .collect(),
        seeds,
        retained: Some((request, retained)),
        quota: app.quota_history_visible().then(Default::default),
        observed: Ok(observed),
    }
}

/// Real public retained bucket documents drive quota capture arithmetic.
pub(super) fn quota(app: &mut App, scenario: &str) {
    use crate::board::{
        rate::history::{self, rollup},
        refresh::HistoryRead,
    };
    use serde_json::json;
    mixed(app);
    for standing in app
        .limits
        .iter_mut()
        .flatten()
        .filter(|standing| standing.provider.id() == "claude")
    {
        standing.latest = None;
        standing.burn = None;
        standing.trend = None;
    }
    let mut view = app.view.take().unwrap();
    for rate in view.home_rate.values_mut() {
        let mut seeds = BTreeMap::new();
        for (id, resume) in &mut rate.input.resumes {
            if resume["consumption"].is_object() {
                resume["consumption"]["observedAtMs"] = json!(NOW - 240_000);
            }
            seeds.insert(id.clone(), Some(seed(VALUES, resume.clone())));
        }
        rate.history = Some(seeds);
    }
    let mut snapshot = crate::board::app::tests::snapshot(crate::board::ALL, json!([]));
    snapshot.tabs = app.tabs.clone();
    snapshot.view = Ok(view);
    app.apply(snapshot);
    let targets: BTreeMap<_, _> = app
        .view
        .as_ref()
        .unwrap()
        .home_rate
        .iter()
        .map(|(name, rate)| (name.clone(), (rate.settings, rate.input.clone())))
        .collect();
    let mut fine = BTreeMap::new();
    let mut quota = BTreeMap::new();
    let mut observed = BTreeMap::new();
    let request = history::quota_request();
    for (_, input) in targets.values() {
        for (id, resume) in &input.resumes {
            fine.insert(id.clone(), Some(seed(VALUES, resume.clone())));
            let reported = resume["driver"] == "codex" && scenario != "unknown";
            let limits = |at: u64, i: usize| {
                let (previous, current) = match scenario {
                    "falling" => (0.05, 0.025),
                    "early" => (0.25, 0.5),
                    "steady" => (0.05, 0.05),
                    _ => (0.025, 0.05),
                };
                let delta = if i < 24 {
                    24.0 * current + (23 - i) as f64 * previous
                } else {
                    (47 - i) as f64 * current
                };
                json!({"observedAtMs":at,"windows":[
                    {"windowMinutes":300,"usedPercent":40.0-delta,"resetsAtMs":NOW+3*3_600_000},
                    {"windowMinutes":10080,"usedPercent":70.0-delta,"resetsAtMs":NOW+5*86_400_000+3*3_600_000}]})
            };
            let buckets:Vec<_>=(0..48).map(|i| {
                let from=NOW-request.range_ms()+i as u64*request.each_ms();
                let mut bucket=json!({"fromMs":from,"toMs":from+request.each_ms(),"inputTokens":1,"outputTokens":0,"cachedInputTokens":0,"coveredMs":request.each_ms(),"complete":true,"gap":false,"discontinuous":false});
                if reported { bucket["rateLimits"]=limits(from+60_000,i); } bucket
            }).collect();
            let row = json!({"found":true,"lastSampleAtMs":NOW-5_000,"latest":resume,"windows":[{
                "windowMs":request.range_ms(),"fromMs":NOW-request.range_ms(),"toMs":NOW,"bucketMs":request.each_ms(),"resolutionMs":rollup::RESOLUTION_MS,
                "availableFromMs":NOW-request.range_ms(),"retainedFromMs":NOW-request.range_ms(),"buckets":buckets}]});
            let response = json!({"asOfMs":NOW+2_500,"throughMs":NOW,"retainedFromMs":NOW-7_200_000,"resolutionMs":5_000});
            quota.insert(
                id.clone(),
                Some(
                    rollup::Seed::read(&response, &row, request)
                        .expect("valid Core-shaped quota history"),
                ),
            );
            let mut current = resume.clone();
            if reported {
                current["consumption"]["rateLimits"] = limits(NOW - 240_000, 47);
            } else if let Some(consumption) = current["consumption"].as_object_mut() {
                consumption.remove("rateLimits");
            }
            observed.insert(id.clone(), current);
        }
    }
    assert!(app.apply_history(
        HistoryRead {
            owner: crate::board::ALL.into(),
            targets,
            seeds: fine,
            retained: None,
            quota: Some(quota),
            observed: Ok(observed)
        },
        std::time::Instant::now()
    ));
}
