//! Office-owned storage: `<global>/office/office.db`, its schema and the
//! resumable migration of Office rows out of the shared core database.
//!
//! This crate depends on core only through public owners: configuration, file
//! locks, the core storage entry point and its error type. It never reuses core
//! storage internals; the migration reads the core database directly, read-only,
//! inside one snapshot.

pub mod access;
mod catalog_replay;
mod cells;
pub mod core_references;
pub mod migration;
mod office_avatar;
mod office_board;
mod office_local;
mod office_profile;
mod office_prop;
mod office_whiteboard;
mod office_world;
pub mod reconciliation;
pub mod retirement;
mod schema;
mod store;
#[cfg(test)]
mod test_support;

pub use office_avatar::{
    LocalAvatarCatalogError, LocalAvatarCatalogList, LocalAvatarExcluded,
    LocalAvatarExcludedReason, LocalAvatarMutation, LocalAvatarSnapshot,
};
pub use office_board::local_owner_actor;
pub use office_local::{LocalBlockSnapshot, LocalOfficeError, LocalPropResolution};
pub use office_profile::{LocalProfileError, LocalProfileMutation, LocalProfileSnapshot};
pub use office_prop::{
    LocalPropCatalogError, LocalPropCatalogList, LocalPropMutation, LocalPropSnapshot,
};
pub use office_whiteboard::WhiteboardStoreError;
pub use office_world::{LocalWorldSnapshot, WorldStoreError};
pub use store::OfficeStore;

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
    /// Global configuration copied into each switch backup.
    pub config: PathBuf,
    /// Parent of the per-switch backup directories.
    pub backups: PathBuf,
}

impl StorageLayout {
    pub fn new(paths: &ConfigPaths) -> Self {
        Self::within(&paths.database, &paths.global_dir, &paths.global_config)
    }

    pub(crate) fn within(source: &Path, global: &Path, config: &Path) -> Self {
        let directory = global.join("office");
        Self {
            source: source.to_path_buf(),
            database: directory.join("office.db"),
            staging: directory.join("office.db.staging"),
            lock: directory.join("migration.lock"),
            directory,
            config: config.to_path_buf(),
            backups: global.join("backups"),
        }
    }
}

#[cfg(test)]
mod tests;
