//! Atomic deployment persistence and all existing FILES consumers.
#[path = "support/deploy_fixture.rs"]
mod deploy_fixture;
use deploy_fixture::{ID, Root};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
use tmt_remote::{
    deploy_record::{self, DeployRecordStore},
    deploy_run::DeployRecord,
    settings,
    state::{Layout, MachineKey},
    store::Store,
};
#[test]
fn writer_excludes_a_successor_but_readers_and_existing_layout_never_take_its_lock() {
    let root = Root::new();
    let layout = root.layout();
    let mut writer = DeployRecordStore::open(&layout).unwrap();
    assert!(matches!(DeployRecordStore::open(&layout),Err(e) if e.code=="REMOTE_DEPLOY_BUSY"));
    let original = DeployRecord::new(ID);
    writer.persist(&original).unwrap();
    let snapshot = fs::read(root.remote().join("deploy.json")).unwrap();
    assert_eq!(deploy_record::read(&layout).unwrap().unwrap(), original);
    let reopened = Layout::open(&root.0).unwrap();
    let existing = Layout::existing(&root.0).unwrap().unwrap();
    assert_eq!(deploy_record::read(&reopened).unwrap().unwrap(), original);
    assert_eq!(deploy_record::read(&existing).unwrap().unwrap(), original);
    assert_eq!(
        fs::read(root.remote().join("deploy.json")).unwrap(),
        snapshot
    );
    assert_eq!(
        fs::metadata(root.remote().join("deploy.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    drop(writer);
    drop(DeployRecordStore::open(&layout).unwrap());
}
#[test]
fn machine_publication_settings_store_and_stopped_lease_preserve_deployment_identity() {
    let root = Root::new();
    let layout = root.layout();
    let mut writer = DeployRecordStore::open(&layout).unwrap();
    writer.persist(&DeployRecord::new(ID)).unwrap();
    let before = fs::read(root.remote().join("deploy.json")).unwrap();
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
    assert_eq!(fs::read(root.remote().join("deploy.json")).unwrap(), before);
    assert_eq!(
        deploy_record::read(&layout).unwrap().unwrap().deployment_id,
        ID
    );
}
#[test]
fn only_admitted_deploy_staging_is_cleaned_and_foreign_names_are_preserved() {
    let root = Root::new();
    let layout = root.layout();
    let stale = root.remote().join(format!(".deploy-{ID}"));
    fs::write(&stale, b"incomplete").unwrap();
    fs::set_permissions(&stale, fs::Permissions::from_mode(0o600)).unwrap();
    let unrelated = root.remote().join(".deploy-not-a-uuid");
    fs::write(&unrelated, b"retain").unwrap();
    drop(DeployRecordStore::open(&layout).unwrap());
    assert!(!stale.exists());
    assert_eq!(fs::read(&unrelated).unwrap(), b"retain");
    assert!(deploy_record::read(&layout).unwrap().is_none());
    symlink(&unrelated, &stale).unwrap();
    assert!(DeployRecordStore::open(&layout).is_err());
    assert!(stale.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(fs::read(&unrelated).unwrap(), b"retain");
}
#[test]
fn unsafe_oversized_malformed_or_future_record_is_never_reset_or_replaced() {
    for bytes in [
        b"{".to_vec(),
        b"{\"version\":2,\"record\":{}}".to_vec(),
        vec![b' '; tmt_remote::limits::DEPLOY_RECORD_BYTES + 1],
        b"{\"version\":1,\"version\":1,\"record\":{}}".to_vec(),
    ] {
        let root = Root::new();
        let layout = root.layout();
        let path = root.remote().join("deploy.json");
        fs::write(&path, &bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(deploy_record::read(&layout).is_err());
        assert!(DeployRecordStore::open(&layout).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    let root = Root::new();
    let layout = root.layout();
    let path = root.remote().join("deploy.json");
    let mut writer = DeployRecordStore::open(&layout).unwrap();
    writer.persist(&DeployRecord::new(ID)).unwrap();
    drop(writer);
    let bytes = fs::read(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(deploy_record::read(&layout).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let foreign = root.remote().join("foreign-record");
    fs::rename(&path, &foreign).unwrap();
    symlink(&foreign, &path).unwrap();
    assert!(deploy_record::read(&layout).is_err());
    assert!(DeployRecordStore::open(&layout).is_err());
    assert_eq!(fs::read(&foreign).unwrap(), bytes);
}

#[test]
fn an_existing_original_cannot_be_replaced_by_a_new_deployment_identity() {
    let root = Root::new();
    let layout = root.layout();
    let mut writer = DeployRecordStore::open(&layout).unwrap();
    writer.persist(&DeployRecord::new(ID)).unwrap();
    let before = fs::read(root.remote().join("deploy.json")).unwrap();
    assert!(
        writer
            .persist(&DeployRecord::new("ad2fb77d-b5e8-4a32-bf35-413f2220d909"))
            .is_err()
    );
    assert_eq!(fs::read(root.remote().join("deploy.json")).unwrap(), before);
}
