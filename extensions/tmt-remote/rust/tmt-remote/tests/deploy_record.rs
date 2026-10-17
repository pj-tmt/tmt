//! Atomic deployment persistence and all existing FILES consumers.
#[path = "support/deploy_fixture.rs"]
mod deploy_fixture;
use deploy_fixture::{ID, Root};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
use tmt_remote::{
    deploy_record::{self, DeployRecordEvidence, DeployRecordStore},
    deploy_run::DeployRecord,
    readiness::{FirestoreEvidence, FirestoreEvidenceSource},
    settings,
    state::{Layout, MachineKey},
    store::Store,
};
#[test]
fn writer_excludes_a_successor_but_readers_and_existing_layout_never_take_its_lock() {
    let root = Root::new();
    let layout = root.layout();
    let source = DeployRecordEvidence::new(&layout);
    assert_eq!(source.evidence(), None);
    let mut writer = DeployRecordStore::open(&layout, "demo-remote-1").unwrap();
    assert!(
        matches!(DeployRecordStore::open(&layout, "demo-remote-1"),Err(e) if e.code=="REMOTE_DEPLOY_BUSY")
    );
    let original = DeployRecord::new(ID);
    writer.persist(&original).unwrap();
    assert_eq!(source.evidence(), None);
    let snapshot = fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
    assert_eq!(deploy_record::read(&layout).unwrap().unwrap(), original);
    let reopened = Layout::open(&root.0).unwrap();
    let existing = Layout::existing(&root.0).unwrap().unwrap();
    assert_eq!(deploy_record::read(&reopened).unwrap().unwrap(), original);
    assert_eq!(deploy_record::read(&existing).unwrap().unwrap(), original);
    assert_eq!(
        fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        snapshot
    );
    assert_eq!(
        fs::metadata(root.remote().join("deployments/demo-remote-1/deploy.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(writer);
    drop(DeployRecordStore::open(&layout, "demo-remote-1").unwrap());
}

#[test]
fn index_readiness_reads_the_snapshot_under_the_writer_lock_without_changing_bytes() {
    use tmt_remote::deploy_run::{Run, RunState, StepRecord, StepState};
    use tmt_remote::readiness::{LAYERS, project};
    let root = Root::new();
    let layout = root.layout();
    let mut writer = DeployRecordStore::open(&layout, "demo-remote-1").unwrap();
    let mut record = DeployRecord::new(ID);
    record.run = Some(Run {
        plan_digest: "a".repeat(64),
        account: "owner@example.test".into(),
        authorized_at_ms: 1,
        state: RunState::Partial,
        rules_attempted: false,
        hosting: None,
        steps: [
            "database".to_owned(),
            "sign-in:anonymous".to_owned(),
            format!("composite-index:log:{}", "b".repeat(64)),
            "rules".to_owned(),
            "verify".to_owned(),
        ]
        .into_iter()
        .map(|id| StepRecord {
            cause: None,
            state: if id.starts_with("composite-index:") {
                StepState::Building
            } else {
                StepState::Pending
            },
            id,
        })
        .collect(),
    });
    writer.persist(&record).unwrap();
    let path = root.remote().join("deployments/demo-remote-1/deploy.json");
    let before = fs::read(&path).unwrap();
    let evidence = DeployRecordEvidence::new(&layout).evidence().unwrap();
    let layers = project(Some(&evidence), &LAYERS);
    let indexes = layers[0]["prerequisites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["item"] == "indexes")
        .expect("index evidence is present while writer is held");
    assert_eq!(indexes["state"], "not-enabled");
    assert_eq!(indexes["reason"], "building");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        deploy_record::read_project(&layout, "demo-remote-1").unwrap(),
        Some(record.clone())
    );
    // A complete snapshot meets guard requirements without altering observed billing/usage.
    record.run.as_mut().unwrap().state = RunState::Complete;
    for step in &mut record.run.as_mut().unwrap().steps {
        step.state = StepState::Done;
    }
    record.binding = Some(tmt_remote::deploy_run::DeployBinding {
        plan_digest: "a".repeat(64),
        project: "demo-remote-1".into(),
        completed_at_ms: 2,
    });
    writer.persist(&record).unwrap();
    let complete = fs::read(&path).unwrap();
    let evidence = DeployRecordEvidence::new(&layout).evidence().unwrap();
    assert_eq!(evidence.quota, tmt_remote::readiness::Observed::Unknown);
    assert_eq!(evidence.tier, tmt_remote::readiness::FirestoreTier::Unknown);
    assert_eq!(project(Some(&evidence), &LAYERS)[0]["state"], "enabled");
    assert_eq!(fs::read(&path).unwrap(), complete);
    assert!(!root.remote().join("remote.db").exists());
    assert!(
        matches!(DeployRecordStore::open(&layout, "demo-remote-1"), Err(e) if e.code == "REMOTE_DEPLOY_BUSY")
    );
}
#[test]
fn machine_publication_settings_store_and_stopped_lease_preserve_deployment_identity() {
    let root = Root::new();
    let layout = root.layout();
    let mut writer = DeployRecordStore::open(&layout, "demo-remote-1").unwrap();
    writer.persist(&DeployRecord::new(ID)).unwrap();
    let before = fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
    let stale = root
        .remote()
        .join(".machine-0123456789abcdef0123456789abcdef");
    fs::write(&stale, [1; 32]).unwrap();
    fs::set_permissions(&stale, fs::Permissions::from_mode(0o600)).unwrap();
    let key = MachineKey::open(&layout).unwrap();
    assert!(!stale.exists());
    drop(key);
    let serving = layout.serve_lock().unwrap();
    let store = Store::open(&serving).unwrap();
    drop(store);
    drop(serving);
    let stopped = layout.existing_serve_lock().unwrap().unwrap();
    assert_eq!(Store::stopped_port(&stopped).unwrap(), None);
    drop(stopped);
    settings::set_open(&root.0, false).unwrap();
    assert_eq!(
        fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        before
    );
    assert_eq!(
        deploy_record::read(&layout).unwrap().unwrap().deployment_id,
        ID
    );
}
#[test]
fn only_admitted_deploy_staging_is_cleaned_and_foreign_names_are_preserved() {
    let root = Root::new();
    let layout = root.layout();
    drop(DeployRecordStore::open(&layout, "demo-remote-1").unwrap());
    let stale = root
        .remote()
        .join("deployments/demo-remote-1")
        .join(format!(".deploy-{ID}"));
    fs::write(&stale, b"incomplete").unwrap();
    fs::set_permissions(&stale, fs::Permissions::from_mode(0o600)).unwrap();
    let unrelated = root
        .remote()
        .join("deployments/demo-remote-1/.deploy-not-a-uuid");
    fs::write(&unrelated, b"retain").unwrap();
    drop(DeployRecordStore::open(&layout, "demo-remote-1").unwrap());
    assert!(!stale.exists());
    assert_eq!(fs::read(&unrelated).unwrap(), b"retain");
    assert!(deploy_record::read(&layout).unwrap().is_none());
    symlink(&unrelated, &stale).unwrap();
    assert!(DeployRecordStore::open(&layout, "demo-remote-1").is_err());
    assert!(stale.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(fs::read(&unrelated).unwrap(), b"retain");
}
#[test]
fn unsafe_oversized_malformed_or_future_record_is_never_reset_or_replaced() {
    for bytes in [
        b"{".to_vec(),
        b"{\"version\":2,\"record\":{}}".to_vec(),
        b"{\"version\":3,\"record\":{}}".to_vec(),
        vec![b' '; tmt_remote::limits::DEPLOY_RECORD_BYTES + 1],
        b"{\"version\":1,\"version\":1,\"record\":{}}".to_vec(),
    ] {
        let root = Root::new();
        let layout = root.layout();
        drop(DeployRecordStore::open(&layout, "demo-remote-1").unwrap());
        let path = root.remote().join("deployments/demo-remote-1/deploy.json");
        fs::write(&path, &bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(deploy_record::read(&layout).is_err());
        assert_eq!(
            DeployRecordEvidence::new(&layout).evidence(),
            Some(FirestoreEvidence::unknown())
        );
        assert!(DeployRecordStore::open(&layout, "demo-remote-1").is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    let root = Root::new();
    let layout = root.layout();
    let path = root.remote().join("deployments/demo-remote-1/deploy.json");
    let mut writer = DeployRecordStore::open(&layout, "demo-remote-1").unwrap();
    writer.persist(&DeployRecord::new(ID)).unwrap();
    drop(writer);
    let bytes = fs::read(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(deploy_record::read(&layout).is_err());
    assert_eq!(
        DeployRecordEvidence::new(&layout).evidence(),
        Some(FirestoreEvidence::unknown())
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let foreign = root.remote().join("foreign-record");
    fs::rename(&path, &foreign).unwrap();
    symlink(&foreign, &path).unwrap();
    assert!(deploy_record::read(&layout).is_err());
    assert_eq!(
        DeployRecordEvidence::new(&layout).evidence(),
        Some(FirestoreEvidence::unknown())
    );
    assert!(DeployRecordStore::open(&layout, "demo-remote-1").is_err());
    assert_eq!(fs::read(&foreign).unwrap(), bytes);
}

#[test]
fn an_existing_original_cannot_be_replaced_by_a_new_deployment_identity() {
    let root = Root::new();
    let layout = root.layout();
    let mut writer = DeployRecordStore::open(&layout, "demo-remote-1").unwrap();
    writer.persist(&DeployRecord::new(ID)).unwrap();
    let before = fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap();
    assert!(
        writer
            .persist(&DeployRecord::new("ad2fb77d-b5e8-4a32-bf35-413f2220d909"))
            .is_err()
    );
    assert_eq!(
        fs::read(root.remote().join("deployments/demo-remote-1/deploy.json")).unwrap(),
        before
    );
}

#[test]
fn target_binds_atomically_and_conflict_preserves_record_and_staging() {
    let root = Root::new();
    let layout = root.layout();
    let original = DeployRecord::new(ID);
    let mut writer =
        DeployRecordStore::open_for_target(&layout, "demo-remote-1", "asia-east1").unwrap();
    writer.bind_target("demo-remote-1", "asia-east1").unwrap();
    assert!(deploy_record::read(&layout).unwrap().is_none());
    writer.persist(&original).unwrap();
    let path = root.remote().join("deployments/demo-remote-1/deploy.json");
    let before = fs::read(&path).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(value["version"], 2);
    assert_eq!(
        value["target"],
        serde_json::json!({"project":"demo-remote-1","region":"asia-east1"})
    );
    assert_eq!(
        deploy_record::read(&layout).unwrap(),
        Some(original.clone())
    );
    drop(writer);
    let staged = root
        .remote()
        .join("deployments/demo-remote-1")
        .join(format!(".deploy-{ID}"));
    fs::write(&staged, b"keep on conflict").unwrap();
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o600)).unwrap();
    {
        let (project, region) = ("demo-remote-1", "us-central1");
        assert!(
            matches!(DeployRecordStore::open_for_target(&layout, project, region), Err(e) if e.code == "REMOTE_DEPLOY_PROJECT_CONFLICT")
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read(&staged).unwrap(), b"keep on conflict");
    }
    let mut reopened =
        DeployRecordStore::open_for_target(&layout, "demo-remote-1", "asia-east1").unwrap();
    reopened.persist(&original).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(!staged.exists());
}

#[test]
fn legacy_single_path_is_refused_without_conversion_or_any_reset() {
    let root = Root::new();
    let layout = root.layout();
    let path = root.remote().join("deploy.json");
    let bytes =
        serde_json::to_vec(&serde_json::json!({"version":1,"record":DeployRecord::new(ID)}))
            .unwrap();
    fs::write(&path, &bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        matches!(DeployRecordStore::open_for_target(&layout, "demo-remote-1", "asia-east1"), Err(e) if e.code == "REMOTE_SELECTOR_LEGACY")
    );
    assert!(matches!(deploy_record::read(&layout), Err(e) if e.code == "REMOTE_SELECTOR_LEGACY"));
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(!root.remote().join("deployments").exists());
}

#[test]
fn previews_for_two_projects_retain_independent_identities_without_binding() {
    let root = Root::new();
    let layout = root.layout();
    for (project, identity) in [
        ("demo-project-a", ID),
        ("demo-project-b", "ad2fb77d-b5e8-4a32-bf35-413f2220d909"),
    ] {
        let mut writer =
            DeployRecordStore::open_for_target(&layout, project, "asia-east1").unwrap();
        writer.persist(&DeployRecord::new(identity)).unwrap();
        let path = root
            .remote()
            .join("deployments")
            .join(project)
            .join("deploy.json");
        let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(value["version"], 1);
        assert!(value.get("target").is_none());
        assert_eq!(value["record"]["deploymentId"], identity);
    }
    assert!(!root.remote().join("deploy.json").exists());
    let mut writer =
        DeployRecordStore::open_for_target(&layout, "demo-project-a", "asia-east1").unwrap();
    assert_eq!(writer.load_or_draft().unwrap().deployment_id, ID);
    writer.bind_target("demo-project-a", "asia-east1").unwrap();
    writer.persist(&DeployRecord::new(ID)).unwrap();
    drop(writer);
    let writer =
        DeployRecordStore::open_for_target(&layout, "demo-project-b", "asia-east1").unwrap();
    assert_eq!(
        writer.load_or_draft().unwrap().deployment_id,
        "ad2fb77d-b5e8-4a32-bf35-413f2220d909"
    );
}
