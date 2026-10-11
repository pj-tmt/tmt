use super::*;
use crate::test_support::TestDirectory;

fn paths(directory: &TestDirectory) -> ConfigPaths {
    ConfigPaths::resolve(
        &directory.path,
        &directory.path,
        Some(&directory.path.join("state")),
        None,
    )
}

#[test]
fn publication_is_private_repeatable_and_versioned_without_settings_writes() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    let first = publish(&paths, "1.2.3").unwrap();
    assert_eq!(publish(&paths, "1.2.3").unwrap(), first);
    let next = publish(&paths, "1.2.4").unwrap();
    assert_ne!(first, next);
    validate(&first).unwrap();
    validate(&next).unwrap();
    assert!(!paths.global_config.exists());
    assert!(!paths.database.exists());
    assert!(!directory.path.join(".claude").exists());
    assert_eq!(fs::read_dir(next.parent().unwrap()).unwrap().count(), 2);
}

#[test]
fn concurrent_launchers_receive_only_complete_assets() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    let barrier = std::sync::Barrier::new(8);
    std::thread::scope(|scope| {
        let handles = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    let path = publish(&paths, "1.2.3").unwrap();
                    validate(&path).unwrap();
                    path
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            assert_eq!(
                handle.join().unwrap(),
                paths.provider_plugin_directory(super::super::NAME, "1.2.3")
            );
        }
    });
    let parent = paths.provider_plugin_directory(super::super::NAME, "1.2.3");
    assert_eq!(fs::read_dir(parent.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn changed_or_symlinked_assets_are_not_loaded() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    let path = publish(&paths, "1.2.3").unwrap();
    let script = path.join("hooks/register.mjs");
    fs::write(&script, "changed").unwrap();
    assert!(publish(&paths, "1.2.3").is_err());
    assert_eq!(fs::read_to_string(&script).unwrap(), "changed");
    fs::remove_file(&script).unwrap();
    let foreign = directory.path.join("foreign");
    fs::write(&foreign, FILES[2].1).unwrap();
    std::os::unix::fs::symlink(&foreign, &script).unwrap();
    assert!(publish(&paths, "1.2.3").is_err());
    assert!(
        fs::symlink_metadata(script)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn unexpected_provider_readable_files_refuse_the_cached_plugin() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    let path = publish(&paths, "1.2.3").unwrap();
    fs::create_dir(path.join("commands")).unwrap();
    assert!(publish(&paths, "1.2.3").is_err());
    assert!(path.join("commands").is_dir());
}

#[test]
fn generated_editor_entries_require_their_owned_regular_shape() {
    for (relative, directory) in [("tsconfig.json", false), (".claude-plugin/types", true)] {
        for symlink in [false, true] {
            let root = TestDirectory::new();
            let paths = paths(&root);
            let plugin = publish(&paths, "1.2.3").unwrap();
            let entry = plugin.join(relative);
            let outside = root.path.join("outside");
            if directory {
                fs::create_dir(&outside).unwrap();
            } else {
                fs::write(&outside, b"foreign bytes").unwrap();
            }
            if symlink {
                std::os::unix::fs::symlink(&outside, &entry).unwrap();
            } else if directory {
                fs::write(&entry, b"wrong shape").unwrap();
            } else {
                fs::create_dir(&entry).unwrap();
            }
            assert!(
                publish(&paths, "1.2.3").is_err(),
                "{relative}, symlink={symlink}"
            );
            assert!(fs::symlink_metadata(&entry).is_ok());
            if !directory {
                assert_eq!(fs::read(&outside).unwrap(), b"foreign bytes");
            }
        }
    }
}

#[test]
fn generated_editor_entries_cannot_be_group_writable() {
    use std::os::unix::fs::PermissionsExt;
    for relative in ["tsconfig.json", ".claude-plugin/types"] {
        let root = TestDirectory::new();
        let paths = paths(&root);
        let plugin = publish(&paths, "1.2.3").unwrap();
        let entry = plugin.join(relative);
        if relative == "tsconfig.json" {
            fs::write(&entry, "{}").unwrap();
            fs::set_permissions(&entry, fs::Permissions::from_mode(0o660)).unwrap();
        } else {
            fs::create_dir(&entry).unwrap();
            fs::set_permissions(&entry, fs::Permissions::from_mode(0o770)).unwrap();
        }
        assert!(publish(&paths, "1.2.3").is_err());
        assert!(entry.exists());
    }
}

#[test]
fn retention_keeps_active_previous_and_assets_for_retained_cli_versions() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    let oldest = publish(&paths, "1.2.1").unwrap();
    let pinned = publish(&paths, "1.2.2").unwrap();
    let previous = publish(&paths, "1.2.3").unwrap();
    let active = publish(&paths, "1.2.4").unwrap();
    let protected = [semver::Version::parse("1.2.2").unwrap()]
        .into_iter()
        .collect();
    prune(&active, "1.2.4", &protected).unwrap();
    assert!(!oldest.exists());
    for path in [&pinned, &previous, &active] {
        validate(path).unwrap();
    }
}

#[test]
fn retention_preserves_foreign_damaged_generated_and_symlinked_plugin_content() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    let damaged = publish(&paths, "1.2.0").unwrap();
    fs::write(damaged.join("hooks/register.mjs"), b"user edit").unwrap();
    let generated = publish(&paths, "1.2.1").unwrap();
    fs::create_dir(generated.join(".claude-plugin/types")).unwrap();
    fs::write(generated.join(".claude-plugin/types/user"), b"generated").unwrap();
    let previous = publish(&paths, "1.2.2").unwrap();
    let active = publish(&paths, "1.2.3").unwrap();
    let parent = active.parent().unwrap();
    let foreign = parent.join("user-backup");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("notes"), b"user bytes").unwrap();
    let alias = parent.join("1.1.0");
    std::os::unix::fs::symlink(&foreign, &alias).unwrap();
    prune(&active, "1.2.3", &Default::default()).unwrap();
    assert_eq!(
        fs::read(damaged.join("hooks/register.mjs")).unwrap(),
        b"user edit"
    );
    assert_eq!(
        fs::read(generated.join(".claude-plugin/types/user")).unwrap(),
        b"generated"
    );
    assert_eq!(fs::read(foreign.join("notes")).unwrap(), b"user bytes");
    assert!(
        fs::symlink_metadata(alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    validate(&previous).unwrap();
    validate(&active).unwrap();
}

#[test]
fn retention_refuses_a_replaced_plugin_inventory() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    let oldest = publish(&paths, "1.2.1").unwrap();
    publish(&paths, "1.2.2").unwrap();
    let active = publish(&paths, "1.2.3").unwrap();
    let parent = active.parent().unwrap();
    let outside = directory.path.join("outside-plugins");
    fs::rename(parent, &outside).unwrap();
    std::os::unix::fs::symlink(&outside, parent).unwrap();
    assert!(prune(&active, "1.2.3", &Default::default()).is_err());
    assert!(oldest.exists());
    for version in ["1.2.1", "1.2.2", "1.2.3"] {
        validate(&outside.join(version)).unwrap();
    }
}
