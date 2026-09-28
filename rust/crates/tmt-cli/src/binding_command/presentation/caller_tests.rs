use super::*;
use tmt_core::{binding::session::RuntimeState, identity::Lifetime};

#[test]
fn ordinary_whoami_preserves_existing_fields_and_human_text() {
    let identity = Identity {
        id: "identity".into(),
        name: "Alice".into(),
        canonical_name: "alice".into(),
        lifetime: Lifetime::Saved,
        created_at: String::new(),
        updated_at: String::new(),
    };
    let original = bound_document(&identity, "%1");
    let report = Report::Caller {
        pane: "%1".into(),
        identity: Some(identity),
        runtime: RuntimeState::Running,
    };
    let mut projected = document(&report);
    assert_eq!(
        projected.as_object_mut().unwrap().remove("interfaceKind"),
        Some(json!("container"))
    );
    assert_eq!(
        projected.as_object_mut().unwrap().remove("sessionState"),
        Some(json!("running"))
    );
    assert_eq!(projected, original);
    let mut human = Vec::new();
    text(&mut human, &report).unwrap();
    assert_eq!(human, b"Bound saved identity 'Alice' on pane %1.\n");
    let report = Report::Caller {
        pane: "%1".into(),
        identity: None,
        runtime: RuntimeState::Unknown,
    };
    assert_eq!(
        document(&report),
        json!({"bound": false, "pane": "%1", "interfaceKind": "container", "sessionState": "unknown"})
    );
}
