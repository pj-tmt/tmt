//! The consented path: a side-effect-free plan and consent-bound migration.
use super::{Root, seed, whole_source};
use crate::migration::{self, MigrationError, PlanState, Quiesce, Quiesced};
use std::{collections::BTreeMap, fs, path::Path, time::SystemTime};

struct Stopped;

impl Quiesce for Stopped {
    fn quiesce(&self) -> Result<Quiesced<'_>, String> {
        Ok(Quiesced {
            was_running: false,
            guard: Box::new(()),
        })
    }
}

/// Every entry under the root with its size and modification time, except
/// the core database's `-wal` and `-shm` files, which any SQLite reader of a
/// WAL database creates; the core rows are compared separately.
fn tree(root: &Path) -> BTreeMap<String, (u64, SystemTime)> {
    let mut entries = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy();
            if name == "tmux-team.db-wal" || name == "tmux-team.db-shm" {
                continue;
            }
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                pending.push(path.clone());
            }
            entries.insert(
                path.display().to_string(),
                (metadata.len(), metadata.modified().unwrap()),
            );
        }
    }
    entries
}

#[test]
fn the_plan_reports_without_creating_or_changing_anything() {
    let root = Root::new();
    seed(&root.source());
    let files = tree(&root.path);
    let source = whole_source(&root.source());
    let plan = migration::plan(&root.layout, Some(false)).unwrap();
    assert_eq!(plan.state, PlanState::Pending);
    assert!(plan.office_rows > 0 && plan.office_bytes > 0);
    assert!(plan.user_rows > 0 && plan.user_rows < plan.office_rows);
    assert!(plan.backup_bytes >= fs::metadata(&root.layout.source).unwrap().len());
    assert_eq!(plan.destination, root.layout.database);
    assert_eq!(
        migration::plan(&root.layout, Some(false)).unwrap().digest,
        plan.digest
    );
    assert_eq!(tree(&root.path), files);
    assert_eq!(whole_source(&root.source()), source);
    assert!(!root.layout.directory.exists());
}

#[test]
fn a_fresh_install_has_only_seeded_rows_and_no_user_data() {
    let root = Root::new();
    let plan = migration::plan(&root.layout, None).unwrap();
    assert_eq!((plan.office_rows, plan.user_rows), (3, 0));
}

#[test]
fn migration_runs_only_for_the_plan_the_user_saw() {
    let root = Root::new();
    seed(&root.source());
    let seen = migration::plan(&root.layout, Some(false)).unwrap();
    root.source()
        .execute("UPDATE office_board_state SET revision = revision + 1", [])
        .unwrap();
    let files = tree(&root.path);
    let result = migration::migrate(&root.layout, &seen.digest, Some(false), &Stopped);
    assert!(
        matches!(result, Err(MigrationError::PlanChanged)),
        "{result:?}"
    );
    assert_eq!(result.unwrap_err().code(), "OFFICE_STORAGE_PLAN_CHANGED");
    assert_eq!(tree(&root.path), files);

    let current = migration::plan(&root.layout, Some(false)).unwrap();
    let switched =
        migration::migrate(&root.layout, &current.digest, Some(false), &Stopped).unwrap();
    assert_eq!(switched.database, root.layout.database);
    let after = migration::plan(&root.layout, Some(false)).unwrap();
    assert_eq!(after.state, PlanState::Switched);
    assert!(after.switched_at_ms.is_some());
    assert_eq!(after.backups, vec![switched.backup]);
    assert!(matches!(
        migration::migrate(&root.layout, &after.digest, Some(false), &Stopped),
        Err(MigrationError::State(_))
    ));
}

#[test]
fn a_core_database_without_the_fence_reports_unavailable() {
    let root = Root::new();
    root.source()
        .execute_batch("DROP TRIGGER office_board_state_after_office_cutover_update")
        .unwrap();
    let plan = migration::plan(&root.layout, None).unwrap();
    assert_eq!(plan.state, PlanState::Unavailable);
    assert!(matches!(
        migration::migrate(&root.layout, &plan.digest, None, &Stopped),
        Err(MigrationError::Source(_))
    ));
    assert!(!root.layout.directory.exists());
}
