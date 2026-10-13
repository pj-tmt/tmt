//! Native request charging shares the shipped model; expected values are frozen
//! independently of the implementation and include worst-case empty polling.
use tmt_remote::firestore_budget::{
    Decision, Model, NativeUsage, charge_native, modeled_discovery_reads_per_hour,
    modeled_poll_reads_per_hour, native_budget_state,
};

#[test]
fn reads_charge_before_send_without_refund_and_share_the_append_allowance() {
    let model = Model::new(1, 1).unwrap();
    let now = 1_800_000_000_000;
    let mut usage = NativeUsage::new(now);
    assert_eq!(
        charge_native(&model, &mut usage, 258, now).unwrap(),
        Decision::Allow
    );
    assert_eq!(
        usage.reads, 258,
        "an empty response cannot refund the bounded maximum"
    );
    usage.reads = 34_995;
    assert_eq!(
        charge_native(&model, &mut usage, 3, now).unwrap(),
        Decision::Warn
    );
    assert_eq!(usage.reads, 34_998);
    let snapshot = usage;
    assert_eq!(
        native_budget_state(&model, usage, now).unwrap(),
        Decision::Warn
    );
    assert_eq!(usage, snapshot);
    usage.reads = 44_995;
    assert!(matches!(
        charge_native(&model, &mut usage, 3, now).unwrap(),
        Decision::Refuse(_)
    ));
    assert_eq!(
        usage.reads, 44_995,
        "refusal must not charge an unsent request"
    );
}

#[test]
fn writer_share_and_existing_write_ceiling_are_not_a_second_quota_pool() {
    let model = Model::new(10, 10).unwrap();
    let now = 1_800_000_000_000;
    let mut usage = NativeUsage::new(now);
    usage.reads = 3_480;
    assert_eq!(
        charge_native(&model, &mut usage, 6, now).unwrap(),
        Decision::Warn
    );
    usage.reads = 4_480;
    let Decision::Refuse(refusal) = charge_native(&model, &mut usage, 3, now).unwrap() else {
        panic!("must refuse at 90% of 4,980 reads")
    };
    assert!(refusal.reset_at_ms > now);
    assert_eq!(refusal.retry_after_ms, refusal.reset_at_ms - now);
    let next_day = refusal.reset_at_ms;
    assert_eq!(
        charge_native(&model, &mut usage, 3, next_day).unwrap(),
        Decision::Allow
    );
    assert_eq!(usage.reads, 3);
}

#[test]
fn an_unrepresentable_charge_never_wraps_into_permission() {
    let model = Model::new(1, 1).unwrap();
    let now = 1_800_000_000_000;
    let mut usage = NativeUsage::new(now);
    usage.reads = u64::MAX;
    assert!(charge_native(&model, &mut usage, 1, now).is_err());
    assert_eq!(usage.reads, u64::MAX);
    assert!(charge_native(&model, &mut NativeUsage::new(now), 0, now).is_err());
}

#[test]
fn guard_table_states_full_charge_at_active_and_idle_intervals() {
    // Two shared Rules dependencies are charged on every query, including empty.
    for (limit, active, idle) in [(1, 2_160, 180), (8, 7_200, 600), (256, 185_760, 15_480)] {
        assert_eq!(modeled_poll_reads_per_hour(limit, 5_000).unwrap(), active);
        assert_eq!(modeled_poll_reads_per_hour(limit, 60_000).unwrap(), idle);
    }
    assert!(modeled_poll_reads_per_hour(0, 5_000).is_err());
    assert!(modeled_poll_reads_per_hour(1, 0).is_err());
}

#[test]
fn discovery_guard_table_charges_each_single_document_page_and_completion_probe() {
    for (documents, hourly) in [(0, 180), (1, 360), (2, 540), (8, 1_620), (256, 46_260)] {
        assert_eq!(
            modeled_discovery_reads_per_hour(documents, 60_000).unwrap(),
            hourly
        );
    }
    assert!(modeled_discovery_reads_per_hour(257, 60_000).is_err());
    assert!(modeled_discovery_reads_per_hour(1, 5_000).is_err());
}
