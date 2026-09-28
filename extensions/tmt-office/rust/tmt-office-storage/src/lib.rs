//! Office-owned storage: `<global>/office/office.db`, its schema and the
//! resumable migration of Office rows out of the shared core database.
//!
//! This crate depends on core only through the public configuration and file
//! lock owners. It never reuses core storage internals; the migration reads the
//! core database directly, read-only, inside one snapshot.

mod cells;
pub mod migration;
mod schema;

use std::path::{Path, PathBuf};
use tmt_adapters::config::ConfigPaths;

/// Office storage paths derived from existing configuration discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageLayout {
    pub source: PathBuf,
    pub directory: PathBuf,
    pub database: PathBuf,
    pub staging: PathBuf,
    pub lock: PathBuf,
}

impl StorageLayout {
    pub fn new(paths: &ConfigPaths) -> Self {
        Self::within(&paths.database, &paths.office_directory())
    }

    fn within(source: &Path, directory: &Path) -> Self {
        Self {
            source: source.to_path_buf(),
            directory: directory.to_path_buf(),
            database: directory.join("office.db"),
            staging: directory.join("office.db.staging"),
            lock: directory.join("migration.lock"),
        }
    }
}

#[cfg(test)]
mod tests;
