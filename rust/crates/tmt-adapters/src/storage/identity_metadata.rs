//! SQLite-backed descriptive identity metadata and exact-match search.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, params, params_from_iter};
use tmt_core::{
    identity::Identity,
    identity_metadata::{
        IdentityMetadataRepository, MAX_METADATA_ENTRIES, MetadataApply, MetadataChanges,
        MetadataCollection, MetadataFilter, MetadataKey, MetadataLookup, MetadataMutation,
        MetadataValue,
    },
};

use super::{
    Storage, StorageError,
    errors::classify,
    identities::{identity_row_at, with_immediate_transaction},
};

impl IdentityMetadataRepository for Storage {
    type Error = StorageError;

    fn get_metadata(
        &self,
        identity_id: &str,
        key: &MetadataKey,
    ) -> Result<MetadataLookup, Self::Error> {
        let (active, value) = self
            .connection()?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM identities WHERE id = ?1 AND retired_at_ms IS NULL),
                        (SELECT m.value FROM identity_metadata m JOIN identities i ON i.id = m.identity_id
                         WHERE m.identity_id = ?1 AND m.key = ?2 AND i.retired_at_ms IS NULL)",
                params![identity_id, key.as_str()],
                |row| Ok((row.get::<_, bool>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .map_err(|error| classify(error, "Get identity metadata"))?;
        Ok(match (active, value) {
            (false, _) => MetadataLookup::IdentityNotFound,
            (true, Some(value)) => MetadataLookup::Found(value),
            (true, None) => MetadataLookup::KeyNotFound,
        })
    }

    fn list_metadata(&self, identity_id: &str) -> Result<MetadataCollection, Self::Error> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT m.key, m.value FROM identities i
                 LEFT JOIN identity_metadata m ON m.identity_id = i.id
                 WHERE i.id = ? AND i.retired_at_ms IS NULL
                 ORDER BY m.key COLLATE BINARY",
            )
            .map_err(|error| classify(error, "Prepare identity metadata list"))?;
        let rows: Vec<(Option<String>, Option<String>)> = statement
            .query_map([identity_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .and_then(|rows| rows.collect())
            .map_err(|error| classify(error, "List identity metadata"))?;
        if rows.is_empty() {
            return Ok(MetadataCollection::IdentityNotFound);
        }
        Ok(MetadataCollection::Found(
            rows.into_iter()
                .filter_map(|(key, value)| key.zip(value))
                .collect(),
        ))
    }

    fn set_metadata(
        &mut self,
        identity_id: &str,
        key: &MetadataKey,
        value: &MetadataValue,
    ) -> Result<MetadataMutation, Self::Error> {
        with_immediate_transaction(self, "identity metadata", |transaction| {
            let active = transaction
                .query_row(
                    "SELECT 1 FROM identities WHERE id = ? AND retired_at_ms IS NULL",
                    [identity_id],
                    |_| Ok(()),
                )
                .optional()
                .map_err(|error| classify(error, "Find identity for metadata set"))?
                .is_some();
            if !active {
                return Ok(MetadataMutation::IdentityNotFound);
            }
            let existing = transaction
                .query_row(
                    "SELECT value FROM identity_metadata WHERE identity_id = ? AND key = ?",
                    params![identity_id, key.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| classify(error, "Read identity metadata before set"))?;
            if existing.as_deref() == Some(value.as_str()) {
                return Ok(MetadataMutation::Set { changed: false });
            }
            if existing.is_none() {
                let count = transaction
                    .query_row(
                        "SELECT COUNT(*) FROM identity_metadata WHERE identity_id = ?",
                        [identity_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(|error| classify(error, "Count identity metadata"))?;
                if count >= MAX_METADATA_ENTRIES as i64 {
                    return Ok(MetadataMutation::EntryLimit);
                }
            }
            transaction
                .execute(
                    "INSERT INTO identity_metadata (identity_id, key, value) VALUES (?, ?, ?)
                     ON CONFLICT(identity_id, key) DO UPDATE SET value = excluded.value",
                    params![identity_id, key.as_str(), value.as_str()],
                )
                .map_err(|error| classify(error, "Set identity metadata"))?;
            Ok(MetadataMutation::Set { changed: true })
        })
    }

    fn remove_metadata(
        &mut self,
        identity_id: &str,
        key: &MetadataKey,
    ) -> Result<MetadataMutation, Self::Error> {
        with_immediate_transaction(self, "identity metadata", |transaction| {
            let active = transaction
                .query_row(
                    "SELECT 1 FROM identities WHERE id = ? AND retired_at_ms IS NULL",
                    [identity_id],
                    |_| Ok(()),
                )
                .optional()
                .map_err(|error| classify(error, "Find identity for metadata removal"))?
                .is_some();
            if !active {
                return Ok(MetadataMutation::IdentityNotFound);
            }
            let removed = transaction
                .execute(
                    "DELETE FROM identity_metadata WHERE identity_id = ? AND key = ?",
                    params![identity_id, key.as_str()],
                )
                .map_err(|error| classify(error, "Remove identity metadata"))?
                == 1;
            Ok(MetadataMutation::Removed { removed })
        })
    }

    fn apply_metadata(
        &mut self,
        identity_id: &str,
        changes: &MetadataChanges,
    ) -> Result<MetadataApply, Self::Error> {
        with_immediate_transaction(self, "identity metadata", |transaction| {
            let active = transaction
                .query_row(
                    "SELECT 1 FROM identities WHERE id = ? AND retired_at_ms IS NULL",
                    [identity_id],
                    |_| Ok(()),
                )
                .optional()
                .map_err(|error| classify(error, "Apply identity metadata"))?;
            if active.is_none() {
                return Ok(MetadataApply::IdentityNotFound);
            }
            let mut statement = transaction
                .prepare(
                    "SELECT key, value FROM identity_metadata
                     WHERE identity_id = ? ORDER BY key COLLATE BINARY",
                )
                .map_err(|error| classify(error, "Read identity metadata before apply"))?;
            let current = statement
                .query_map([identity_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .and_then(|rows| rows.collect::<rusqlite::Result<BTreeMap<_, _>>>())
                .map_err(|error| classify(error, "Read identity metadata before apply"))?;
            let next = match changes.evaluate(&current) {
                Ok(next) => next,
                Err(refusal) => return Ok(refusal),
            };
            for change in changes.changes() {
                let key = change.key.as_str();
                if current.get(key) == next.get(key) {
                    continue;
                }
                match next.get(key) {
                    Some(value) => {
                        transaction
                            .execute(
                                "INSERT INTO identity_metadata (identity_id, key, value) VALUES (?, ?, ?)
                                 ON CONFLICT(identity_id, key) DO UPDATE SET value = excluded.value",
                                params![identity_id, key, value],
                            )
                            .map_err(|error| classify(error, "Apply identity metadata"))?;
                    }
                    None => {
                        transaction
                            .execute(
                                "DELETE FROM identity_metadata WHERE identity_id = ? AND key = ?",
                                params![identity_id, key],
                            )
                            .map_err(|error| classify(error, "Apply identity metadata"))?;
                    }
                }
            }
            Ok(MetadataApply::Applied {
                changed: current != next,
            })
        })
    }

    fn list_identities_matching(
        &self,
        filters: &[MetadataFilter],
    ) -> Result<Vec<Identity>, Self::Error> {
        let mut sql = String::from(
            "SELECT i.id, i.name, i.canonical_name, i.lifetime, i.created_at, i.updated_at \
             FROM identities i WHERE i.retired_at_ms IS NULL",
        );
        let mut values = Vec::new();
        for (index, filter) in filters.iter().enumerate() {
            let alias = format!("m{index}");
            match filter {
                MetadataFilter::Equals { key, value } => {
                    sql.push_str(&format!(
                        " AND EXISTS (SELECT 1 FROM identity_metadata {alias} INDEXED BY identity_metadata_search WHERE {alias}.key = ? AND {alias}.value = ? AND {alias}.identity_id = i.id)"
                    ));
                    values.push(key.as_str().to_owned());
                    values.push(value.as_str().to_owned());
                }
                MetadataFilter::Has(key) => {
                    sql.push_str(&format!(
                        " AND EXISTS (SELECT 1 FROM identity_metadata {alias} WHERE {alias}.identity_id = i.id AND {alias}.key = ?)"
                    ));
                    values.push(key.as_str().to_owned());
                }
            }
        }
        sql.push_str(" ORDER BY i.canonical_name COLLATE BINARY");
        let mut statement = self
            .connection()?
            .prepare(&sql)
            .map_err(|error| classify(error, "Prepare filtered identity list"))?;
        statement
            .query_map(params_from_iter(values.iter()), |row| {
                identity_row_at(row, 0)
            })
            .and_then(|rows| rows.collect())
            .map_err(|error| classify(error, "List filtered identities"))
    }
}

#[cfg(test)]
mod tests;
