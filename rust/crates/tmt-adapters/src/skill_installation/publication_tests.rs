use super::{
    ProviderEnvironment, files, install, install_office, install_office_with_publisher,
    install_with_publisher,
};
use crate::test_support::TestDirectory;
use std::{fs, io};

#[test]
fn failed_publication_reports_prior_success_and_recoverable_backup_then_releases_lock() {
    let root = TestDirectory::new();
    let home = root.path.join("home");
    let global = root.path.join("global");
    fs::create_dir(&home).unwrap();
    let env = ProviderEnvironment::from_parts(home.clone(), root.path.clone(), Vec::new(), []);
    let first = home.join(".claude/skills/tmt");
    let second = home.join(".agents/skills/tmt");
    fs::create_dir_all(&second).unwrap();
    fs::write(second.join("user.md"), b"irreplaceable user content").unwrap();
    let mut publications = 0;
    let failure =
        install_with_publisher(&env, &global, Some("all"), None, true, |target, source| {
            publications += 1;
            if target == second {
                // The real backup has already happened. Fail only the file boundary.
                assert!(!target.exists());
                return Err(io::Error::other("injected publication failure"));
            }
            files::link(target, source)
        })
        .unwrap_err();
    assert_eq!(publications, 3);
    assert_eq!(failure.report.installed.len(), 2);
    assert_eq!(failure.report.installed[0].target, first);
    assert!(fs::symlink_metadata(&first).unwrap().is_symlink());
    let backup = failure.pending_backup.as_ref().unwrap();
    assert_eq!(
        fs::read(backup.join("user.md")).unwrap(),
        b"irreplaceable user content"
    );
    assert!(!second.exists());
    let message = failure.to_string();
    assert!(message.contains("injected publication failure"));
    assert!(message.contains(first.to_str().unwrap()));
    assert!(message.contains(backup.to_str().unwrap()));
    // Intent remains discoverable, but failed targets are not reported installed.
    assert!(super::registry::read(&global).unwrap().contains(&second));
    let retry = install(&env, &global, Some("all"), None, false).unwrap();
    assert!(!retry.installed[0].changed);
    assert!(!retry.installed[1].changed);
    assert!(retry.installed[2].changed);
    assert!(fs::symlink_metadata(&second).unwrap().is_symlink());
    assert_eq!(
        fs::read(backup.join("user.md")).unwrap(),
        b"irreplaceable user content"
    );
}

#[test]
fn failed_office_publication_preserves_a_recoverable_backup_and_can_retry() {
    let root = TestDirectory::new();
    let home = root.path.join("home");
    let global = root.path.join("global");
    fs::create_dir(&home).unwrap();
    let env = ProviderEnvironment::from_parts(home.clone(), root.path.clone(), Vec::new(), []);
    let target = home.join(".agents/skills/tmt-office");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("user.md"), b"user-owned office guidance").unwrap();

    let failure = install_office_with_publisher(&env, &global, true, |published, source| {
        if published == target {
            assert!(!published.exists());
            return Err(io::Error::other("injected Office publication failure"));
        }
        files::link(published, source)
    })
    .unwrap_err();
    assert_eq!(failure.report.installed.len(), 1);
    assert_eq!(failure.report.installed[0].name, "tmt-avatar-create");
    let backup = failure.pending_backup.as_ref().unwrap();
    assert_eq!(
        fs::read(backup.join("user.md")).unwrap(),
        b"user-owned office guidance"
    );
    assert!(!target.exists());
    assert!(failure.to_string().contains(backup.to_str().unwrap()));

    let retry = install_office(&env, &global, false).unwrap();
    assert_eq!(retry.installed.len(), 3);
    assert!(
        retry
            .installed
            .iter()
            .find(|item| item.name == "tmt-office")
            .unwrap()
            .changed
    );
    assert!(fs::symlink_metadata(&target).unwrap().is_symlink());
    assert_eq!(
        fs::read(backup.join("user.md")).unwrap(),
        b"user-owned office guidance"
    );
}

#[test]
fn abandoned_stage_is_preserved_without_blocking_a_new_install() {
    let root = TestDirectory::new();
    let global = root.path.join("global");
    let stage = global.join("skill-assets/.stage-abandoned");
    fs::create_dir_all(&stage).unwrap();
    fs::write(stage.join("partial"), b"preserve for inspection").unwrap();
    let env =
        ProviderEnvironment::from_parts(root.path.join("home"), root.path.clone(), Vec::new(), []);
    let result = install(&env, &global, None, None, false).unwrap();
    assert!(result.installed[0].changed);
    assert_eq!(
        fs::read(stage.join("partial")).unwrap(),
        b"preserve for inspection"
    );
}
