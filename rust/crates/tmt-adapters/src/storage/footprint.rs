//! What each local store occupies on disk (#5036), for `tmt storage`. Only
//! bounded `stat` calls and the Core database's read-only page counts: nothing
//! is created, migrated, locked for writing or repaired. A store that does not
//! exist is reported as absent.

use super::compaction::{DatabasePages, observe_pages};
use crate::config::ConfigPaths;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// Entries the notes walk visits at most; past it the total is a lower bound.
const NOTES_ENTRY_LIMIT: usize = 10_000;
/// Directory levels below `notes/` the walk descends (`<identity>/notes.md`).
const NOTES_DEPTH: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreFile {
    pub path: PathBuf,
    pub bytes: u64,
}

#[derive(Debug)]
pub struct DatabaseFootprint {
    pub path: PathBuf,
    pub bytes: u64,
    pub wal_bytes: u64,
    /// The page counts, or why SQLite could not read them.
    pub pages: Result<DatabasePages, String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct NotesFootprint {
    pub path: PathBuf,
    pub bytes: u64,
    pub files: u64,
    /// False when the walk stopped at its entry limit.
    pub complete: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ColabFootprint {
    pub path: PathBuf,
    pub bytes: u64,
    pub wal_bytes: u64,
    /// `space.db.pre-v<N>.bak` copies taken before Colab migrations, by name.
    pub backups: Vec<StoreFile>,
}

/// Each store's path, and its footprint when it exists.
#[derive(Debug)]
pub struct Footprint {
    pub data_root: PathBuf,
    pub database_path: PathBuf,
    pub database: Option<DatabaseFootprint>,
    pub notes_path: PathBuf,
    pub notes: Option<NotesFootprint>,
    pub colab_path: PathBuf,
    pub colab: Option<ColabFootprint>,
}

fn size(path: &Path) -> io::Result<Option<u64>> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(metadata.len())),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn wal_size(path: &Path) -> io::Result<u64> {
    let mut wal = path.as_os_str().to_os_string();
    wal.push("-wal");
    Ok(size(Path::new(&wal))?.unwrap_or(0))
}

pub fn footprint(paths: &ConfigPaths) -> io::Result<Footprint> {
    let data_root = crate::config::normalize(&std::path::absolute(&paths.global_dir)?);
    let database_path = crate::config::normalize(&std::path::absolute(&paths.database)?);
    let database = match size(&database_path)? {
        Some(bytes) => Some(DatabaseFootprint {
            bytes,
            wal_bytes: wal_size(&database_path)?,
            pages: observe_pages(&database_path).map_err(|error| error.to_string()),
            path: database_path.clone(),
        }),
        None => None,
    };
    let notes_path = data_root.join("notes");
    let notes = notes(&notes_path)?;
    let colab_directory = data_root.join("colab");
    let colab_path = colab_directory.join("space.db");
    let colab = match size(&colab_path)? {
        Some(bytes) => Some(ColabFootprint {
            bytes,
            wal_bytes: wal_size(&colab_path)?,
            backups: colab_backups(&colab_directory)?,
            path: colab_path.clone(),
        }),
        None => None,
    };
    Ok(Footprint {
        data_root,
        database_path,
        database,
        notes_path,
        notes,
        colab_path,
        colab,
    })
}

fn notes(root: &Path) -> io::Result<Option<NotesFootprint>> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    let mut footprint = NotesFootprint {
        path: root.to_path_buf(),
        bytes: 0,
        files: 0,
        complete: true,
    };
    let mut visited = 0;
    let mut pending = vec![(root.to_path_buf(), 0)];
    while let Some((directory, depth)) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            visited += 1;
            if visited > NOTES_ENTRY_LIMIT {
                footprint.complete = false;
                return Ok(Some(footprint));
            }
            let entry = entry?;
            // Links are not followed: only what lives under notes/ counts.
            let metadata = entry.metadata()?;
            if metadata.is_file() {
                footprint.bytes += metadata.len();
                footprint.files += 1;
            } else if metadata.is_dir() && depth < NOTES_DEPTH {
                pending.push((entry.path(), depth + 1));
            }
        }
    }
    Ok(Some(footprint))
}

fn is_colab_backup(name: &str) -> bool {
    name.strip_prefix("space.db.pre-v")
        .and_then(|rest| rest.strip_suffix(".bak"))
        .is_some_and(|version| {
            !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn colab_backups(directory: &Path) -> io::Result<Vec<StoreFile>> {
    let mut backups = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let metadata = entry.metadata()?;
        if is_colab_backup(&name) && metadata.is_file() {
            backups.push(StoreFile {
                path: entry.path(),
                bytes: metadata.len(),
            });
        }
    }
    backups.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(backups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{storage::Storage, test_support::TestDirectory};

    fn paths(directory: &Path) -> ConfigPaths {
        ConfigPaths::resolve(directory, directory, Some(directory), None)
    }

    fn names(directory: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn missing_stores_are_absent_and_nothing_is_created() {
        let directory = TestDirectory::new();
        let footprint = footprint(&paths(&directory.path)).unwrap();
        assert!(footprint.database.is_none());
        assert!(footprint.notes.is_none());
        assert!(footprint.colab.is_none());
        assert_eq!(footprint.database_path, directory.path.join("tmt.db"));
        assert_eq!(footprint.colab_path, directory.path.join("colab/space.db"));
        assert!(names(&directory.path).is_empty());
    }

    #[test]
    fn every_present_store_is_measured_without_changing_it() {
        let directory = TestDirectory::new();
        let root = &directory.path;
        let mut storage = Storage::open(root.join("tmt.db")).unwrap();
        storage.close().unwrap();
        fs::create_dir_all(root.join("notes/id-1")).unwrap();
        fs::write(root.join("notes/id-1/notes.md"), vec![b'n'; 300]).unwrap();
        fs::create_dir_all(root.join("notes/id-2")).unwrap();
        fs::write(root.join("notes/id-2/notes.md"), vec![b'n'; 200]).unwrap();
        fs::create_dir_all(root.join("colab")).unwrap();
        fs::write(root.join("colab/space.db"), vec![0; 4096]).unwrap();
        fs::write(root.join("colab/space.db.pre-v12.bak"), vec![0; 100]).unwrap();
        fs::write(root.join("colab/space.db.pre-v13.bak"), vec![0; 200]).unwrap();
        fs::write(root.join("colab/space.db.pre-vx.bak"), b"not a backup").unwrap();
        let before = (names(root), fs::read(root.join("tmt.db")).unwrap());

        let footprint = footprint(&paths(root)).unwrap();

        let database = footprint.database.unwrap();
        assert_eq!(
            database.bytes,
            fs::metadata(root.join("tmt.db")).unwrap().len()
        );
        let pages = database.pages.unwrap();
        assert_eq!(pages.page_count * pages.page_size, database.bytes);
        assert_eq!(
            footprint.notes,
            Some(NotesFootprint {
                path: root.join("notes"),
                bytes: 500,
                files: 2,
                complete: true,
            })
        );
        let colab = footprint.colab.unwrap();
        assert_eq!((colab.bytes, colab.wal_bytes), (4096, 0));
        assert_eq!(
            colab.backups,
            [
                StoreFile {
                    path: root.join("colab/space.db.pre-v12.bak"),
                    bytes: 100
                },
                StoreFile {
                    path: root.join("colab/space.db.pre-v13.bak"),
                    bytes: 200
                },
            ]
        );
        assert_eq!(
            (names(root), fs::read(root.join("tmt.db")).unwrap()),
            before
        );
    }

    #[test]
    fn an_unreadable_database_reports_why_instead_of_failing() {
        let directory = TestDirectory::new();
        fs::write(
            directory.path.join("tmt.db"),
            b"this is not a database at all",
        )
        .unwrap();
        let database = footprint(&paths(&directory.path))
            .unwrap()
            .database
            .unwrap();
        assert_eq!(database.bytes, 29);
        assert!(database.pages.is_err());
        assert_eq!(
            fs::read(directory.path.join("tmt.db")).unwrap(),
            b"this is not a database at all"
        );
    }
}
