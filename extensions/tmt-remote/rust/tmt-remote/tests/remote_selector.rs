//! Selection is local bookkeeping, not provider evidence or administrative authority.
#[path = "support/deploy_fixture.rs"]
mod deploy_fixture;
use deploy_fixture::{ID, Root};
use tmt_remote::{
    deploy_record::DeployRecordStore, deploy_run::DeployRecord, remote_selector as selector,
    settings,
};
fn public_link() -> String {
    use base64::Engine;
    let mut descriptor: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/remote_link/descriptor-v1.json")).unwrap();
    descriptor.sort_all_objects();
    format!(
        "https://demo-remote-1.web.app/pair#{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&descriptor).unwrap())
    )
}

#[test]
fn ordinary_client_imports_public_config_without_deployment_or_owner_authority() {
    let root = Root::new();
    let layout = root.layout();
    tmt_remote::remote_catalog::add(&layout, "tenant", &public_link()).unwrap();
    let selection = selector::client(&layout, Some("tenant"), None).unwrap();
    assert_eq!(selection.project, "demo-remote-1");
    assert_eq!(selection.public_config["projectId"], selection.project);
    assert_eq!(selection.public_config.as_object().unwrap().len(), 4);
    for field in [
        "region",
        "deploymentId",
        "planDigest",
        "ownerHost",
        "uid",
        "token",
    ] {
        assert!(selection.public_config.get(field).is_none());
    }
    assert!(!root.remote().join("deployments").exists());
    assert!(
        matches!(selector::owned(&layout, Some("tenant"), None), Err(e) if e.code == "REMOTE_SELECTOR_NOT_OWNED")
    );
    assert!(
        matches!(selector::existing(&layout, Some("tenant"), None), Err(e) if e.code == "REMOTE_SELECTOR_NOT_OWNED")
    );
    assert_eq!(
        selector::client(&layout, Some("tenant"), Some("another-project"))
            .err()
            .unwrap()
            .code,
        "REMOTE_SELECTOR_INVALID"
    );
    settings::set_default_remote(&root.0, "tenant").unwrap();
    assert_eq!(
        selector::client(&layout, None, None).unwrap().project,
        "demo-remote-1"
    );
    assert_eq!(
        selector::client(&layout, None, Some("another-project"))
            .err()
            .unwrap()
            .code,
        "REMOTE_SELECTOR_INVALID"
    );
}

#[test]
fn ordinary_client_never_falls_back_from_explicit_or_broken_default_selection() {
    let root = Root::new();
    let layout = root.layout();
    let mut writer = DeployRecordStore::open(&layout, "demo-local-project").unwrap();
    writer.persist(&DeployRecord::new(ID)).unwrap();
    drop(writer);
    assert!(
        selector::client_status(&layout, None, None)
            .unwrap()
            .is_none()
    );
    assert!(selector::client_status(&layout, Some("demo-local-project"), None).is_err());
    assert_eq!(
        selector::client(&layout, Some("missing"), None)
            .err()
            .unwrap()
            .code,
        "REMOTE_SELECTOR_NOT_FOUND"
    );
    settings::set_default_remote(&root.0, "missing").unwrap();
    assert_eq!(
        selector::client(&layout, None, None).err().unwrap().code,
        "REMOTE_SELECTOR_STALE_DEFAULT"
    );
    // Even --project cannot turn a broken explicit/default into an implicit host choice.
    assert_eq!(
        selector::client(&layout, None, Some("demo-local-project"))
            .err()
            .unwrap()
            .code,
        "REMOTE_SELECTOR_STALE_DEFAULT"
    );
    tmt_remote::remote_catalog::add(&layout, "tenant", &public_link()).unwrap();
    assert_eq!(
        selector::client(&layout, Some("tenant"), None)
            .unwrap()
            .project,
        "demo-remote-1"
    );
    settings::set_default_remote(&root.0, "local").unwrap();
    // A partial local deployment has no verified public config to enroll with.
    assert_eq!(
        selector::client(&layout, None, None).err().unwrap().code,
        "REMOTE_LINK_UNAVAILABLE"
    );
}

#[test]
fn ordinary_client_revalidates_corrupt_imports_and_preserves_private_state() {
    let root = Root::new();
    let layout = root.layout();
    tmt_remote::remote_catalog::add(&layout, "tenant", &public_link()).unwrap();
    let path = root.remote().join("remotes.json");
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    // Tamper the persisted link, rather than relying only on import admission.
    let text = catalog
        .to_string()
        .replace("demo-remote-1.web.app", "wrong-project.web.app");
    catalog = serde_json::from_str(&text).unwrap();
    let bytes = serde_json::to_vec(&catalog).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    assert!(selector::client(&layout, Some("tenant"), None).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert!(!root.remote().join("firebase-auth").exists());
}

#[test]
fn explicit_project_default_and_sole_fallback_never_choose_an_arbitrary_project() {
    let root = Root::new();
    let layout = root.layout();
    assert_eq!(selector::owned(&layout, None, None).unwrap(), None);
    for project in ["demo-project-a", "demo-project-b"] {
        let mut writer = DeployRecordStore::open(&layout, project).unwrap();
        writer.persist(&DeployRecord::new(ID)).unwrap();
        drop(writer);
        if project == "demo-project-a" {
            assert_eq!(
                selector::owned(&layout, None, None).unwrap().as_deref(),
                Some(project)
            );
        }
    }
    assert!(
        matches!(selector::owned(&layout, None, None), Err(e) if e.code == "REMOTE_SELECTOR_REQUIRED" && e.message.contains("demo-project-a") && e.message.contains("demo-project-b"))
    );
    settings::set_default_remote(&root.0, "demo-project-b").unwrap();
    assert_eq!(
        selector::owned(&layout, None, None).unwrap().as_deref(),
        Some("demo-project-b")
    );
    assert_eq!(
        selector::owned(&layout, Some("demo-project-a"), None)
            .unwrap()
            .as_deref(),
        Some("demo-project-a")
    );
    assert!(
        matches!(selector::owned(&layout, Some("demo-project-a"), Some("demo-project-b")), Err(e) if e.code == "REMOTE_SELECTOR_INVALID")
    );
    assert_eq!(
        selector::owned(&layout, None, Some("demo-project-c"))
            .unwrap()
            .as_deref(),
        Some("demo-project-c")
    );
}

#[test]
fn local_owner_commands_ignore_default_but_refuse_a_foreign_explicit_selection() {
    let root = Root::new();
    let layout = root.layout();
    settings::set_default_remote(&root.0, "removed").unwrap();
    selector::local(&layout, None, "serve").unwrap();
    assert!(
        matches!(selector::owned(&layout, None, None), Err(e) if e.code == "REMOTE_SELECTOR_STALE_DEFAULT")
    );
    selector::local(&layout, Some("local"), "serve").unwrap();
}

#[test]
fn catalog_removal_preserves_projects_and_leaves_a_stale_default() {
    let root = Root::new();
    let layout = root.layout();
    tmt_remote::remote_catalog::add(&layout, "other-host", "https://remote.example/r/abc/")
        .unwrap();
    assert!(
        matches!(selector::local(&layout, Some("other-host"), "serve"), Err(e) if e.code == "REMOTE_SELECTOR_LOCAL_REQUIRED")
    );
    // Syntax storage permits a public Local reference; it confers no foreign administration.
    settings::set_default_remote(&root.0, "other-host").unwrap();
    assert!(
        matches!(selector::owned(&layout, None, None), Err(e) if e.code == "REMOTE_SELECTOR_LOCAL_REQUIRED")
    );
    tmt_remote::remote_catalog::remove(&layout, "other-host").unwrap();
    assert!(
        matches!(selector::owned(&layout, None, None), Err(e) if e.code == "REMOTE_SELECTOR_STALE_DEFAULT")
    );
    assert!(!root.remote().join("deployments").exists());
    assert_eq!(
        settings::read(&root.0).unwrap().default_remote(),
        Some("other-host")
    );
}

#[test]
fn imported_public_link_is_not_ownership_and_names_cannot_shadow_projects() {
    let root = Root::new();
    let layout = root.layout();
    let descriptor = serde_json::from_str::<serde_json::Value>(include_str!(
        "fixtures/remote_link/descriptor-v1.json"
    ))
    .unwrap();
    use base64::Engine;
    let mut descriptor = descriptor;
    descriptor.sort_all_objects();
    let link = format!(
        "https://demo-remote-1.web.app/pair#{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&descriptor).unwrap())
    );
    tmt_remote::remote_catalog::add(&layout, "tenant", &link).unwrap();
    assert!(
        matches!(selector::owned(&layout, Some("tenant"), None), Err(e) if e.code == "REMOTE_SELECTOR_NOT_OWNED")
    );
    assert!(
        matches!(selector::local(&layout, Some("tenant"), "serve"), Err(e) if e.code == "REMOTE_SELECTOR_KIND_UNSUPPORTED")
    );
    // Explicit --project creates here without mistaking the tenant reference for ownership.
    assert_eq!(
        selector::owned(&layout, None, Some("demo-remote-1"))
            .unwrap()
            .as_deref(),
        Some("demo-remote-1")
    );
    let mut writer = DeployRecordStore::open(&layout, "demo-project-a").unwrap();
    writer.persist(&DeployRecord::new(ID)).unwrap();
    drop(writer);
    assert!(
        matches!(tmt_remote::remote_catalog::add(&layout, "demo-project-a", &link), Err(e) if e.code == "REMOTE_SELECTOR_INVALID")
    );
    tmt_remote::remote_catalog::add(&layout, "demo-project-b", &link).unwrap();
    assert!(
        matches!(selector::owned(&layout, None, Some("demo-project-b")), Err(e) if e.code == "REMOTE_SELECTOR_INVALID")
    );
}

#[test]
fn malformed_default_does_not_change_owner_browser_or_session_preferences() {
    let root = Root::new();
    let layout = root.layout();
    use std::io::Write;
    layout
        .file("settings.json")
        .unwrap()
        .write_all(br#"{"open":false,"sessionsPerDevice":2,"defaultRemote":false}"#)
        .unwrap();
    let settings = settings::read_or_default(&root.0);
    assert!(!settings.open());
    assert_eq!(settings.sessions_per_device(), Some(2));
    assert!(!settings.malformed);
    assert!(!settings.default_valid());
    selector::local(&layout, None, "serve").unwrap();
    assert!(
        matches!(selector::owned(&layout, None, None), Err(e) if e.code == "REMOTE_SELECTOR_UNAVAILABLE")
    );
}

#[test]
fn copied_bound_target_names_both_paths_and_never_opens_an_effect_owner() {
    let root = Root::new();
    let layout = root.layout();
    let mut writer =
        DeployRecordStore::open_for_target(&layout, "demo-project-a", "asia-east1").unwrap();
    writer.bind_target("demo-project-a", "asia-east1").unwrap();
    writer.persist(&DeployRecord::new(ID)).unwrap();
    drop(writer);
    drop(DeployRecordStore::open(&layout, "demo-project-b").unwrap());
    let source = root.remote().join("deployments/demo-project-a/deploy.json");
    let destination = root.remote().join("deployments/demo-project-b/deploy.json");
    use std::os::unix::fs::PermissionsExt;
    std::fs::copy(&source, &destination).unwrap();
    std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o600)).unwrap();
    let error = DeployRecordStore::open_for_target(&layout, "demo-project-b", "asia-east1")
        .err()
        .unwrap();
    assert_eq!(error.code, "REMOTE_SELECTOR_CONFLICT");
    assert!(error.message.contains(&source.display().to_string()));
    assert!(error.message.contains(&destination.display().to_string()));
    assert_eq!(
        std::fs::read(&source).unwrap(),
        std::fs::read(&destination).unwrap()
    );
}

#[test]
fn project_inventory_admits_the_parent_before_listing_an_empty_foreign_directory() {
    let root = Root::new();
    let layout = root.layout();
    let foreign = root.0.join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    std::os::unix::fs::symlink(&foreign, root.remote().join("deployments")).unwrap();
    assert!(
        matches!(tmt_remote::deploy_record::projects(&layout), Err(e) if e.code == "REMOTE_STATE_UNSAFE")
    );
    assert!(std::fs::read_dir(&foreign).unwrap().next().is_none());
}
