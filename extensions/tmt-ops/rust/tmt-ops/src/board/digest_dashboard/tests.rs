use super::model;
use crate::board::workspace::{
    OpsMember, OpsObservation, OpsTarget, observe::OpsWorkspaceSnapshot,
};

fn supplied(held: serde_json::Value, stale: bool) -> crate::labels::Supplied {
    crate::labels::Supplied::from_document(
        "digest",
        &serde_json::json!({
            "version": 1, "observedAtMs": 1000,
            "members": [{"identityId":"10000000-0000-4000-8000-000000000001", "name":"Agent",
                "mode":"auto", "intervalMs":null, "settingSource":"default", "settingValue":"default",
                "heldCount":held, "statsObservedAtMs":null, "nextSendAtMs":null, "timeToSendMs":null,
            "labels":[{"text":"Auto","colorClass":"muted"},{"text":"When idle","colorClass":"muted"}]
            }]
        }),
        stale,
    )
}

#[test]
fn null_stats_keep_the_inherited_idle_setting_without_a_zero_count() {
    let supplied = supplied(serde_json::Value::Null, false);
    assert!(model::summary(&supplied).contains("held unavailable"));
    assert!(!model::summary(&supplied).contains("0 held"));
    let details = model::details(&supplied, "10000000-0000-4000-8000-000000000001");
    assert!(details.iter().any(|s| s.contains("inherited default")));
    assert!(details.iter().any(|s| s == "Auto: when idle"));
    assert!(!details.iter().any(|s| s.starts_with("Next send")));
}

#[test]
fn known_zero_and_retained_stale_statistics_stay_distinct() {
    let zero = supplied(serde_json::json!(0), false);
    assert!(model::summary(&zero).contains("0 held"));
    let stale = supplied(serde_json::json!(3), true);
    assert!(model::summary(&stale).contains("3 held · stale observation"));
    assert!(
        model::details(&stale, "10000000-0000-4000-8000-000000000001")
            .iter()
            .any(|s| s.contains("Stale observation"))
    );
}

#[test]
fn unavailable_statistics_never_become_zero() {
    assert_eq!(
        model::summary(&Default::default()),
        "Digest status unavailable"
    );
    assert!(
        model::details(&Default::default(), "missing")
            .iter()
            .any(|line| line.contains("unavailable"))
    );
}

#[test]
fn duplicate_memberships_have_one_row_and_keep_a_management_context() {
    let mut snapshot = OpsWorkspaceSnapshot::default();
    snapshot.roster.observation = OpsObservation::Complete;
    snapshot.roster.members.insert(
        "id".into(),
        OpsMember {
            id: "id".into(),
            name: "Agent".into(),
            state: None,
            waiting: false,
            presence: None,
            usage: None,
        },
    );
    for room in [None, Some("a"), Some("b")] {
        snapshot.rows.insert(
            OpsTarget::Member {
                room_id: room.map(str::to_owned),
                identity_id: "id".into(),
            },
            serde_json::json!({"id":"id","name":"Agent"}),
        );
    }
    let rows = model::rows(&snapshot, "agent");
    assert_eq!(rows.len(), 1);
    assert!(
        matches!(&rows[0].target, OpsTarget::Member { room_id: Some(room), .. } if room == "a")
    );
    assert!(model::rows(&snapshot, "absent").is_empty());
}
