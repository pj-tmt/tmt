//! Read-only view of extension storage cutover receipts (schema 36).
//!
//! An extension's own migration coordinator writes its receipt inside the
//! transaction that rechecks its retained rows; core only reports it.

use rusqlite::OptionalExtension;

use super::{Storage, StorageError, errors::classify};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageCutover {
    pub extension: String,
    pub destination_device: i64,
    pub destination_inode: i64,
    pub storage_schema_version: i64,
    pub manifest: String,
    pub switched_at_ms: i64,
}

impl Storage {
    pub fn extension_storage_cutover(
        &self,
        extension: &str,
    ) -> Result<Option<StorageCutover>, StorageError> {
        self.connection()?
            .query_row(
                "SELECT extension, destination_device, destination_inode, storage_schema_version, manifest, switched_at_ms \
                 FROM extension_storage_cutovers WHERE extension = ?",
                [extension],
                |row| {
                    Ok(StorageCutover {
                        extension: row.get(0)?,
                        destination_device: row.get(1)?,
                        destination_inode: row.get(2)?,
                        storage_schema_version: row.get(3)?,
                        manifest: row.get(4)?,
                        switched_at_ms: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(|error| classify(error, "Read extension storage cutover"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;

    #[test]
    fn absent_until_an_extension_records_its_receipt() {
        let directory = TestDirectory::new();
        let storage = Storage::open(directory.path.join("state.db")).unwrap();
        assert_eq!(storage.extension_storage_cutover("office").unwrap(), None);
        storage
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO extension_storage_cutovers VALUES ('office', 1, 2, 2, ?, 3)",
                ["a".repeat(64)],
            )
            .unwrap();
        assert_eq!(
            storage.extension_storage_cutover("office").unwrap(),
            Some(StorageCutover {
                extension: "office".into(),
                destination_device: 1,
                destination_inode: 2,
                storage_schema_version: 2,
                manifest: "a".repeat(64),
                switched_at_ms: 3,
            })
        );
        assert_eq!(storage.extension_storage_cutover("other").unwrap(), None);
        for invalid in ["", "Office", "x y", &"a".repeat(33)] {
            assert!(
                storage
                    .connection()
                    .unwrap()
                    .execute(
                        "INSERT INTO extension_storage_cutovers VALUES (?, 1, 2, 2, ?, 3)",
                        [invalid, &"a".repeat(64)],
                    )
                    .is_err(),
                "{invalid:?}"
            );
        }
    }
}
