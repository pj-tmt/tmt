//! Free-plan Firestore budget guard (#2180). Vectors come from an independent Python script
//! (`fixtures/firestore_budget/reference.py`, Pacific days from the system tz database); the
//! guard's arithmetic is what is tested, not Google's counter, which no test can read.
use serde_json::Value;
use std::fs;
use tmt_remote::firestore_budget::{
    BUDGET_EXHAUSTED, Decision, ExhaustedEvidence, ExhaustedOutcome, Model, ModelError, PacificDay,
    ProviderExhausted, READS_PER_DAY, REFUSE_PERCENT, Refusal, Usage, Verdict, WARN_PERCENT,
    classify_provider_exhausted, decide, pacific_day,
};

fn vectors() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/firestore_budget/vectors.json");
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn model(row: &Value) -> Model {
    Model::new(
        row["members"].as_u64().unwrap(),
        row["writers"].as_u64().unwrap(),
    )
    .unwrap()
}

#[test]
fn the_arithmetic_matches_the_independent_vectors() {
    let vectors = vectors();
    for row in vectors["append"].as_array().unwrap() {
        let m = model(row);
        let got = [
            m.reads_per_append(),
            m.daily_append_limit(),
            m.share(),
            m.warn_at(),
            m.refuse_at(),
            m.min_flush_interval_ms(),
        ];
        let want = [
            "readsPerAppend",
            "dailyAppendLimit",
            "share",
            "warnAt",
            "refuseAt",
            "minFlushIntervalMs",
        ]
        .map(|k| row[k].as_u64().unwrap());
        assert_eq!(got, want, "{row}");
        // The optimistic figure in the vectors is documentation: the guard is pessimistic.
        assert!(row["optimisticReadsPerAppend"].as_u64().unwrap() <= m.reads_per_append());
    }
    let assessments = vectors["assess"].as_array().unwrap();
    assert!(assessments.len() > 50);
    for row in assessments {
        let verdict = match model(row).assess(row["used"].as_u64().unwrap()) {
            Verdict::Ok => "ok",
            Verdict::Warn => "warn",
            Verdict::Refuse => "refuse",
        };
        assert_eq!(verdict, row["verdict"], "{row}");
    }
    for row in vectors["pacific"].as_array().unwrap() {
        let now = row["nowMs"].as_u64().unwrap();
        let (day, reset) = pacific_day(now);
        assert_eq!(
            (day.0, reset),
            (
                row["day"].as_i64().unwrap(),
                row["resetAtMs"].as_u64().unwrap()
            ),
            "{row}"
        );
    }
}

#[test]
fn a_hand_worked_row_and_the_named_thresholds() {
    // Five members, three writers: 3 + 4 x (1 + 2) = 15 reads per append; 50,000 / 15 = 3,333
    // appends; a writer's share is 1,111, warned at 777 (70%) and refused at 999 (90%).
    let m = Model::new(5, 3).unwrap();
    assert_eq!(
        (
            m.reads_per_append(),
            m.daily_append_limit(),
            m.share(),
            m.warn_at(),
            m.refuse_at()
        ),
        (15, 3333, 1111, 777, 999)
    );
    assert_eq!(
        (m.assess(776), m.assess(777), m.assess(998), m.assess(999)),
        (Verdict::Ok, Verdict::Warn, Verdict::Warn, Verdict::Refuse)
    );
    assert_eq!((WARN_PERCENT, REFUSE_PERCENT), (70, 90));
    // One member alone: the writer's own lookups only.
    assert_eq!(Model::new(1, 1).unwrap().reads_per_append(), 3);
    // The writes pool (20,000) never binds before the reads pool for any accepted model.
    for members in [1, 2, 10, 100] {
        let m = Model::new(members, 1).unwrap();
        assert!(m.daily_append_limit() * m.reads_per_append() <= READS_PER_DAY);
    }
}

#[test]
fn impossible_models_are_refused() {
    for (members, writers) in [(0, 0), (1, 0), (0, 1), (2, 3), (10_001, 1)] {
        assert_eq!(
            Model::new(members, writers),
            Err(ModelError::OutOfRange),
            "{members} {writers}"
        );
    }
}

#[test]
fn a_page_the_free_plan_cannot_host_is_refused_when_the_model_is_built() {
    let vectors = vectors();
    let rows = vectors["hosting"].as_array().unwrap();
    assert!(rows.len() >= 5);
    for row in rows {
        let built = Model::new(
            row["members"].as_u64().unwrap(),
            row["writers"].as_u64().unwrap(),
        );
        if row["accepted"] == true {
            let m = built.unwrap_or_else(|e| panic!("{row}: {e:?}"));
            // Accepted means the guard can allow at least one append before it refuses.
            assert!(m.refuse_at() >= 1 && m.assess(0) == Verdict::Ok, "{row}");
        } else {
            assert_eq!(built, Err(ModelError::FreePlanCannotHost), "{row}");
        }
    }
    // Hand-checked for one writer: 3N reads per append, so 50,000 / 3N is 2 appends at N = 8,333.
    assert!(
        Model::new(8333, 1).is_ok() && Model::new(8334, 1) == Err(ModelError::FreePlanCannotHost)
    );
}

const T: u64 = 1_800_000_000_000; // an instant in Pacific winter time
#[test]
fn the_guard_warns_then_refuses_before_the_limit_and_the_refusal_is_recoverable() {
    let m = Model::new(5, 3).unwrap();
    let (_, reset) = pacific_day(T);
    let usage = |appends| Usage {
        appends,
        ..Usage::new(T)
    };
    assert_eq!(decide(&m, usage(0), T), Decision::Allow);
    assert_eq!(decide(&m, usage(776), T), Decision::Allow);
    assert_eq!(decide(&m, usage(777), T), Decision::Warn);
    assert_eq!(decide(&m, usage(998), T), Decision::Warn);
    let refused = decide(&m, usage(999), T);
    assert_eq!(
        refused,
        Decision::Refuse(Refusal {
            reset_at_ms: reset,
            retry_after_ms: reset - T
        })
    );
    // Refusal comes at 90% of the share, strictly before the share and the page allowance.
    assert!(m.refuse_at() < m.share() && m.share() * 3 <= m.daily_append_limit());
    // Not silent and not permanent: after the reset the same usage is a fresh day.
    assert_eq!(decide(&m, usage(999), reset), Decision::Allow);
    assert_eq!(
        decide(&m, usage(999), reset - 1),
        refused_at(reset - 1, reset)
    );
    // Counting rolls over by itself.
    let sent = usage(5).recorded(T);
    assert_eq!(sent.appends, 6);
    assert_eq!(sent.recorded(reset).appends, 1);
    assert_eq!(sent.at(reset).day, PacificDay(sent.day.0 + 1));
}
fn refused_at(now: u64, reset: u64) -> Decision {
    Decision::Refuse(Refusal {
        reset_at_ms: reset,
        retry_after_ms: reset - now,
    })
}

#[test]
fn a_provider_refusal_after_a_possible_write_is_unknown_never_the_budget_error() {
    let (_, reset) = pacific_day(T);
    assert_eq!(BUDGET_EXHAUSTED, "REMOTE_BUDGET_EXHAUSTED");
    // A read cannot have changed anything: a refusal with the reset time.
    let (read, evidence) = classify_provider_exhausted(ProviderExhausted { mutation: false }, T);
    assert_eq!(
        read,
        ExhaustedOutcome::Refused(Refusal {
            reset_at_ms: reset,
            retry_after_ms: reset - T
        })
    );
    // A write may have been applied: unknown, with no reset promise and no budget code.
    let (write, same) = classify_provider_exhausted(ProviderExhausted { mutation: true }, T);
    assert_eq!(write, ExhaustedOutcome::Unknown);
    assert_eq!(evidence, same);
    assert!(
        matches!(read, ExhaustedOutcome::Refused(_))
            && !matches!(write, ExhaustedOutcome::Refused(_))
    );
    // Both record `exhausted` evidence, which says nothing once the quota has reset.
    assert_eq!(evidence, ExhaustedEvidence::recorded(T));
    assert!(evidence.is_current(T) && evidence.is_current(reset - 1));
    assert!(!evidence.is_current(reset));
}
