use super::{ConfigFiles, ConfigPaths};
use crate::test_support::TestDirectory;
use std::{
    fs,
    path::Path,
    sync::{Arc, Barrier},
};

fn files(directory: &Path) -> ConfigFiles {
    ConfigFiles {
        paths: ConfigPaths::resolve(directory, directory, Some(&directory.join("global")), None),
    }
}

#[test]
fn initialize_local_writes_exact_bytes_without_other_state() {
    let directory = TestDirectory::new();
    let config = files(&directory.path);

    config.initialize_local().unwrap();

    assert_eq!(fs::read(&config.paths.local_config).unwrap(), b"{}\n");
    assert!(!config.paths.global_config.exists());
    assert!(!config.paths.database.exists());
    let entries = fs::read_dir(&directory.path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(entries, vec![config.paths.local_config.clone()]);
}

#[test]
fn initialize_local_refuses_existing_file_and_directory_without_mutation() {
    let directory = TestDirectory::new();
    let config = files(&directory.path);
    let path = &config.paths.local_config;

    fs::write(path, b"{malformed\n").unwrap();
    let error = config.initialize_local().unwrap_err();
    assert_eq!(error.code, "ERROR");
    assert!(error.message.contains(&path.display().to_string()));
    assert_eq!(fs::read(path).unwrap(), b"{malformed\n");

    fs::remove_file(path).unwrap();
    fs::create_dir(path).unwrap();
    let error = config.initialize_local().unwrap_err();
    assert_eq!(error.code, "ERROR");
    assert!(path.is_dir());
}

#[cfg(unix)]
#[test]
fn initialize_local_refuses_existing_symlink_without_mutation() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new();
    let config = files(&directory.path);
    let path = &config.paths.local_config;
    let destination = directory.path.join("existing-settings.json");
    fs::write(&destination, b"user-owned\n").unwrap();
    symlink(&destination, path).unwrap();

    let error = config.initialize_local().unwrap_err();
    assert_eq!(error.code, "ERROR");
    assert_eq!(fs::read(path).unwrap(), b"user-owned\n");
    assert_eq!(fs::read_link(path).unwrap(), destination);
    assert_eq!(fs::read(&destination).unwrap(), b"user-owned\n");
}

#[test]
fn concurrent_initialization_has_one_winner_and_no_partial_state() {
    let directory = TestDirectory::new();
    let config = Arc::new(files(&directory.path));
    let barrier = Arc::new(Barrier::new(8));

    let results = std::thread::scope(|scope| {
        let handles = (0..8)
            .map(|_| {
                let config = Arc::clone(&config);
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    barrier.wait();
                    config.initialize_local()
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("initialization worker must not panic"))
            .collect::<Vec<_>>()
    });

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 7);
    for result in &results {
        if let Err(error) = result {
            assert_eq!(error.code, "ERROR");
        }
    }
    assert_eq!(fs::read(&config.paths.local_config).unwrap(), b"{}\n");
    assert!(!config.paths.global_config.exists());
    assert!(!config.paths.database.exists());
}

#[test]
fn failure_cleanup_removes_only_the_owned_partial_file_and_preserves_replacements() {
    let directory = TestDirectory::new();
    let path = directory.path.join("tmt.json");
    let owned = fs::File::create_new(&path).unwrap();
    let error = super::finish_initialization_failure(
        &path,
        &owned,
        std::io::Error::other("injected write failure"),
    );
    assert!(error.message.contains("injected write failure"));
    assert!(!path.exists());
    drop(owned);

    let owned = fs::File::create_new(&path).unwrap();
    let detached = directory.path.join("owned-partial");
    fs::rename(&path, &detached).unwrap();
    fs::write(&path, b"replacement owned by another writer").unwrap();
    let error = super::finish_initialization_failure(
        &path,
        &owned,
        std::io::Error::other("injected sync failure"),
    );
    assert!(error.message.contains("injected sync failure"));
    assert_eq!(
        fs::read(&path).unwrap(),
        b"replacement owned by another writer"
    );
    assert!(detached.exists());
}
