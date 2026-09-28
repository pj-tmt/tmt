use super::*;
use crate::test_support::TestDirectory;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    sync::{Arc, Barrier},
    thread,
};
use tmt_core::identity::{Identity, Lifetime, NotesIdentityId};

fn identity_id() -> NotesIdentityId {
    NotesIdentityId::try_from(&Identity {
        id: "e27f6cd2-2ce7-4c40-8ef7-f156492e983b".into(),
        name: "Researcher".into(),
        canonical_name: "researcher".into(),
        lifetime: Lifetime::Saved,
        created_at: String::new(),
        updated_at: String::new(),
    })
    .unwrap()
}

fn paths(directory: &TestDirectory) -> ConfigPaths {
    ConfigPaths::resolve(
        &directory.path,
        &directory.path,
        Some(&directory.path.join("config")),
        None,
    )
}

fn notebook_fixture(lifetime: Lifetime) -> (TestDirectory, ConfigPaths, Identity) {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    let mut storage = crate::storage::Storage::open(&paths.database).unwrap();
    let identity = tmt_core::identity::create_or_resolve(&mut storage, "Researcher", lifetime)
        .unwrap()
        .identity;
    storage.close().unwrap();
    (directory, paths, identity)
}

#[test]
fn context_path_discovery_never_opens_storage_or_reads_notebook_contents() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    let identity = identity_id();
    assert_eq!(existing_path(&paths, &identity).unwrap(), None);
    assert!(!paths.global_dir.exists());
    // Notebook initialization requires the configuration owner to create its
    // root; the read-only discovery above must not do that work implicitly.
    fs::create_dir_all(&paths.global_dir).unwrap();
    let path = initialize(&paths, &identity).unwrap().path;
    // An invalid database proves this projection cannot accidentally open it.
    fs::write(&paths.database, b"not a SQLite database").unwrap();
    let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len((NOTEBOOK_READ_LIMIT * 2) as u64).unwrap();
    drop(file);
    let before = fs::metadata(&path).unwrap();
    assert_eq!(
        existing_path(&paths, &identity).unwrap(),
        Some(path.clone())
    );
    let after = fs::metadata(&path).unwrap();
    assert_eq!(after.len(), before.len());
    assert_eq!(after.ino(), before.ino());
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    assert_eq!(fs::read(&paths.database).unwrap(), b"not a SQLite database");
    assert!(!paths.database.with_extension("db-wal").exists());
    fs::remove_file(&path).unwrap();
    assert_eq!(existing_path(&paths, &identity).unwrap(), None);
}

#[test]
fn notebook_read_is_exact_bounded_and_never_initializes_or_rewrites() {
    let (_directory, paths, identity) = notebook_fixture(Lifetime::Saved);
    assert_eq!(read(&paths, &identity.id), Err(NotebookError::Missing));
    assert!(!paths.global_dir.join("notes").exists());
    let notes_id = NotesIdentityId::try_from(&identity).unwrap();
    let file = initialize(&paths, &notes_id).unwrap().path;
    let content = "# Notebook\r\n<script>inert</script>\0\u{feff} exact 🦀";
    fs::write(&file, content).unwrap();
    let before = fs::metadata(&file).unwrap();
    assert_eq!(
        read(&paths, &identity.id).unwrap(),
        Notebook {
            identity_id: identity.id.clone(),
            name: identity.name,
            content: content.into()
        }
    );
    assert_eq!(fs::metadata(&file).unwrap().ino(), before.ino());
    assert_eq!(
        fs::metadata(&file).unwrap().modified().unwrap(),
        before.modified().unwrap()
    );
    assert_eq!(fs::read(&file).unwrap(), content.as_bytes());
    fs::write(&file, "Updated by the agent").unwrap();
    assert_eq!(
        read(&paths, &identity.id).unwrap().content,
        "Updated by the agent"
    );
    fs::write(&file, vec![b'a'; NOTEBOOK_READ_LIMIT]).unwrap();
    assert_eq!(
        read(&paths, &identity.id).unwrap().content.len(),
        NOTEBOOK_READ_LIMIT
    );
    fs::write(&file, vec![b'a'; NOTEBOOK_READ_LIMIT + 1]).unwrap();
    assert_eq!(read(&paths, &identity.id), Err(NotebookError::TooLarge));
    assert_eq!(
        fs::metadata(&file).unwrap().len(),
        (NOTEBOOK_READ_LIMIT + 1) as u64
    );
    fs::write(&file, [0xff]).unwrap();
    assert_eq!(read(&paths, &identity.id), Err(NotebookError::InvalidText));
    assert_eq!(fs::read(&file).unwrap(), [0xff]);
}

#[test]
fn notebook_reader_rejects_nonregular_files_and_symlinks_at_each_notes_component() {
    let (directory, paths, identity) = notebook_fixture(Lifetime::Saved);
    let notes_id = NotesIdentityId::try_from(&identity).unwrap();
    let file = initialize(&paths, &notes_id).unwrap().path;
    let outside = directory.path.join("private-sentinel");
    fs::write(&outside, b"not notebook content").unwrap();
    fs::remove_file(&file).unwrap();
    symlink(&outside, &file).unwrap();
    assert_eq!(read(&paths, &identity.id), Err(NotebookError::Unavailable));
    assert!(existing_path(&paths, &notes_id).is_err());
    fs::remove_file(&file).unwrap();
    fs::create_dir(&file).unwrap();
    assert_eq!(read(&paths, &identity.id), Err(NotebookError::Unavailable));
    assert!(existing_path(&paths, &notes_id).is_err());
    fs::remove_dir(&file).unwrap();
    nix::unistd::mkfifo(&file, nix::sys::stat::Mode::S_IRUSR).unwrap();
    assert_eq!(read(&paths, &identity.id), Err(NotebookError::Unavailable));
    assert!(existing_path(&paths, &notes_id).is_err());
    fs::remove_file(&file).unwrap();
    fs::write(&file, b"retained notebook").unwrap();
    for (index, parent) in [
        file.parent().unwrap().to_path_buf(),
        paths.global_dir.join("notes"),
    ]
    .iter()
    .enumerate()
    {
        let retained = directory.path.join(format!("retained-{index}"));
        fs::rename(parent, &retained).unwrap();
        symlink(&retained, parent).unwrap();
        assert_eq!(read(&paths, &identity.id), Err(NotebookError::Unavailable));
        assert!(existing_path(&paths, &notes_id).is_err());
        fs::remove_file(parent).unwrap();
        fs::rename(retained, parent).unwrap();
    }
    assert_eq!(fs::read(outside).unwrap(), b"not notebook content");
    assert_eq!(fs::read(file).unwrap(), b"retained notebook");
}

#[test]
fn notebook_reader_requires_the_original_active_saved_identity() {
    let (_directory, paths, temporary) = notebook_fixture(Lifetime::Temporary);
    assert_eq!(
        read(&paths, "../notes"),
        Err(NotebookError::InvalidIdentity)
    );
    assert_eq!(
        read(&paths, &temporary.id),
        Err(NotebookError::SavedIdentityRequired)
    );
    assert!(!paths.global_dir.join("notes").exists());
    let mut storage = crate::storage::Storage::open(&paths.database).unwrap();
    let saved = tmt_core::identity::create_or_resolve(&mut storage, "Researcher", Lifetime::Saved)
        .unwrap()
        .identity;
    assert_eq!(saved.id, temporary.id);
    storage.close().unwrap();
    let file = initialize(&paths, &NotesIdentityId::try_from(&saved).unwrap())
        .unwrap()
        .path;
    fs::write(&file, b"Retained after retirement").unwrap();
    let database = rusqlite::Connection::open(&paths.database).unwrap();
    database
        .execute(
            "UPDATE identities SET retired_at_ms = 100 WHERE id = ?",
            [&saved.id],
        )
        .unwrap();
    drop(database);
    assert_eq!(
        read(&paths, &saved.id),
        Err(NotebookError::IdentityNotFound)
    );
    let mut storage = crate::storage::Storage::open(&paths.database).unwrap();
    let replacement =
        tmt_core::identity::create_or_resolve(&mut storage, "Researcher", Lifetime::Saved)
            .unwrap()
            .identity;
    storage.close().unwrap();
    assert_ne!(replacement.id, saved.id);
    assert_eq!(read(&paths, &replacement.id), Err(NotebookError::Missing));
    assert_eq!(fs::read(file).unwrap(), b"Retained after retirement");
}

#[test]
fn creates_private_empty_file_and_preserves_existing_bytes_and_inode() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    fs::create_dir(&paths.global_dir).unwrap();

    let first = initialize(&paths, &identity_id()).unwrap();
    assert!(first.created);
    assert!(first.path.is_absolute());
    assert_eq!(fs::read(&first.path).unwrap(), b"");
    let identity_dir = first.path.parent().unwrap();
    let notes_dir = identity_dir.parent().unwrap();
    assert_eq!(
        fs::metadata(identity_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(notes_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&first.path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    fs::write(&first.path, b"# Notes\nexact bytes\0\xff").unwrap();
    let before = fs::metadata(&first.path).unwrap().ino();
    let second = initialize(&paths, &identity_id()).unwrap();
    assert!(!second.created);
    assert_eq!(second.path, first.path);
    assert_eq!(fs::metadata(&second.path).unwrap().ino(), before);
    assert_eq!(
        fs::read(&second.path).unwrap(),
        b"# Notes\nexact bytes\0\xff"
    );
}

#[test]
fn concurrent_first_access_has_one_creator() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    fs::create_dir(&paths.global_dir).unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let handles = (0..8)
        .map(|_| {
            let paths = paths.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                initialize(&paths, &identity_id()).unwrap()
            })
        })
        .collect::<Vec<_>>();
    let reports = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(reports.iter().filter(|report| report.created).count(), 1);
    assert!(reports.windows(2).all(|pair| pair[0].path == pair[1].path));

    let notes_file = &reports[0].path;
    assert!(fs::symlink_metadata(notes_file).unwrap().is_file());
    assert_eq!(fs::read(notes_file).unwrap(), b"");
    fs::write(notes_file, b"# concurrent sentinel\n").unwrap();
    let inode = fs::metadata(notes_file).unwrap().ino();
    let repeat_barrier = Arc::new(Barrier::new(8));
    let repeats = (0..8)
        .map(|_| {
            let paths = paths.clone();
            let barrier = repeat_barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                initialize(&paths, &identity_id()).unwrap()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert!(repeats.iter().all(|report| !report.created));
    assert!(repeats.iter().all(|report| report.path == *notes_file));
    assert_eq!(fs::metadata(notes_file).unwrap().ino(), inode);
    assert_eq!(fs::read(notes_file).unwrap(), b"# concurrent sentinel\n");
}

#[test]
fn rejects_symlink_and_nonregular_targets_without_touching_them() {
    let directory = TestDirectory::new();
    let paths = paths(&directory);
    fs::create_dir(&paths.global_dir).unwrap();
    let notes = paths.global_dir.join("notes");
    let outside = directory.path.join("outside");
    fs::create_dir(&outside).unwrap();
    let sentinel = outside.join("sentinel");
    fs::write(&sentinel, b"untouched").unwrap();
    symlink(&outside, &notes).unwrap();
    assert!(initialize(&paths, &identity_id()).is_err());
    assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");

    fs::remove_file(&notes).unwrap();
    fs::create_dir(&notes).unwrap();
    let identity_dir = notes.join(identity_id().as_str());
    symlink(&outside, &identity_dir).unwrap();
    assert!(initialize(&paths, &identity_id()).is_err());
    assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");
    assert!(!outside.join("notes.md").exists());

    fs::remove_file(&identity_dir).unwrap();
    fs::create_dir(&identity_dir).unwrap();
    let notes_file = identity_dir.join("notes.md");
    symlink(&sentinel, &notes_file).unwrap();
    assert!(initialize(&paths, &identity_id()).is_err());
    assert_eq!(fs::read(&sentinel).unwrap(), b"untouched");

    fs::remove_file(&notes_file).unwrap();
    let missing = outside.join("missing.md");
    symlink(&missing, &notes_file).unwrap();
    assert!(initialize(&paths, &identity_id()).is_err());
    assert!(!missing.exists());

    fs::remove_file(&notes_file).unwrap();
    fs::create_dir(&notes_file).unwrap();
    assert!(initialize(&paths, &identity_id()).is_err());
}
