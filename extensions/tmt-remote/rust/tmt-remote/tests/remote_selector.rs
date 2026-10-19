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
    assert_eq!(selection.kind, selector::ClientKind::Imported);
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
    let mut writer =
        DeployRecordStore::open(&layout, "demo-local-project", "retry deploy fixture").unwrap();
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
        let mut writer = DeployRecordStore::open(&layout, project, "retry deploy fixture").unwrap();
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
    let mut writer =
        DeployRecordStore::open(&layout, "demo-project-a", "retry deploy fixture").unwrap();
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
    let mut writer = DeployRecordStore::open_for_target(
        &layout,
        "demo-project-a",
        "asia-east1",
        "retry deploy fixture",
    )
    .unwrap();
    writer.bind_target("demo-project-a", "asia-east1").unwrap();
    writer.persist(&DeployRecord::new(ID)).unwrap();
    drop(writer);
    drop(DeployRecordStore::open(&layout, "demo-project-b", "retry deploy fixture").unwrap());
    let source = root.remote().join("deployments/demo-project-a/deploy.json");
    let destination = root.remote().join("deployments/demo-project-b/deploy.json");
    use std::os::unix::fs::PermissionsExt;
    std::fs::copy(&source, &destination).unwrap();
    std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o600)).unwrap();
    let error = DeployRecordStore::open_for_target(
        &layout,
        "demo-project-b",
        "asia-east1",
        "retry deploy fixture",
    )
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

#[test]
fn catalog_updates_join_the_document_writer_and_read_its_current_bytes_once() {
    use std::{sync::mpsc, time::Duration};
    use tmt_extension_state::Document;
    let root = Root::new();
    let layout = root.layout();
    let path = root.remote().join("remotes.json");
    tmt_remote::remote_catalog::add(&layout, "seed", "https://example.test/r/seed/").unwrap();
    tmt_remote::remote_catalog::remove(&layout, "seed").unwrap();
    let document = Document::new(
        &layout.directory,
        &path,
        tmt_remote::limits::REMOTE_CATALOG_BYTES as u64,
        "retry catalog test",
    )
    .unwrap();
    let (entered, held) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let (finished, completion) = mpsc::channel();
    let (early, first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(move || document.update::<std::io::Error>(|current| {
            let mut value: serde_json::Value = serde_json::from_slice(current.unwrap()).unwrap();
            value["entries"].as_array_mut().unwrap().push(serde_json::json!({
                "name":"first", "kind":"local", "url":"https://example.test/r/first/", "project":null
            }));
            entered.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(serde_json::to_vec(&value).unwrap())
        }));
        held.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(
            tmt_remote::remote_catalog::read(&layout)
                .unwrap()
                .is_empty()
        );
        let second = scope.spawn(|| {
            let result = tmt_remote::remote_catalog::add(
                &layout,
                "second",
                "https://example.test/r/second/",
            );
            finished.send(()).unwrap();
            result
        });
        // A real held writer is the readiness signal. Completion before release is a defect.
        let early = completion.recv_timeout(Duration::from_millis(100));
        release.send(()).unwrap();
        (early, first.join().unwrap(), second.join().unwrap())
    });
    first.unwrap();
    second.unwrap();
    assert!(
        matches!(early, Err(mpsc::RecvTimeoutError::Timeout)),
        "catalog bypassed the document writer"
    );
    let entries = tmt_remote::remote_catalog::read(&layout).unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}

#[test]
fn catalog_document_recovery_preserves_private_mode_and_refuses_loose_input() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let root = Root::new();
    let layout = root.layout();
    let path = root.remote().join("remotes.json");
    let staging = root.remote().join(".remotes.json.tmt-staging");
    fs::write(&staging, b"abandoned publication").unwrap();
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o600)).unwrap();
    tmt_remote::remote_catalog::add(&layout, "first", "https://example.test/r/first/").unwrap();
    assert!(!staging.exists());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let bytes = fs::read(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        tmt_remote::remote_catalog::add(&layout, "second", "https://example.test/r/second/")
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );
}
