//! Built-in restore sees a member resumed on another host as running.

use super::*;
use std::os::unix::fs::DirBuilderExt;
use tmt_core::workspace::{
    WorkspaceIdentity,
    builtin::{BuiltinMember, BuiltinSnapshot},
};

#[test]
fn a_built_in_member_now_running_on_another_host_is_skipped_untouched() {
    register();
    let fixture = Fixture::new("restore-elsewhere");
    let record = fixture.approve();
    bind(&fixture, record);
    let mut storage = Storage::open(fixture.state().join("tmt.db")).unwrap();
    let identity = storage.resolve_identity("worker").unwrap().unwrap();
    storage.close().unwrap();
    // The built-in store still records worker from before it was resumed on this host.
    let store = fixture.state().join("workspace/pty");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&store)
        .unwrap();
    fs::set_permissions(
        fixture.state().join("workspace"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let snapshot = BuiltinSnapshot {
        captured_at_ms: 1,
        members: vec![BuiltinMember {
            cwd: Some(fixture.root.display().to_string()),
            identity: WorkspaceIdentity {
                id: identity.id.clone(),
                name: "worker".into(),
                lifetime: "saved".into(),
                binding: "binding-before-the-move".into(),
                harness: None,
                session: None,
                mode: None,
                channel: None,
            },
        }],
    };
    fs::write(
        store.join("latest.json"),
        tmt_adapters::workspace::encode_builtin(&snapshot).unwrap(),
    )
    .unwrap();
    let restored = fixture.json(&["workspace", "restore", "--host", "pty", "--json"]);
    assert_eq!(restored["members"][0]["name"], "worker", "{restored:#}");
    assert_eq!(restored["members"][0]["status"], "skipped", "{restored:#}");
    assert_eq!(restored["members"][0]["reason"], "live", "{restored:#}");
    // Untouched: still bound and running on the other host.
    let row = worker(&fixture.json(&["ls", "--json"])).clone();
    assert_eq!(row["presence"], "active", "{row:#}");
    assert_eq!(row["pane"], "fake-1", "{row:#}");
}
