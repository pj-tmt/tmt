//! SQLite-backed local Office prop catalog with revision CAS and bounded reads.

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::collections::HashMap;

use tmt_office_model::codec::office_prop::PACK_INPUT_LIMIT;
use tmt_office_model::codec::office_prop::PropPackError;
use tmt_office_model::codec::office_prop::ValidatedPropPack;
use tmt_office_model::codec::office_prop::builtin_by_digest;
use tmt_office_model::codec::office_prop::builtin_packs;
use tmt_office_model::codec::office_prop::parse_pack_digest;
use tmt_office_model::codec::office_prop::validate_pack;

use crate::OfficeStore;
use tmt_adapters::storage::{StorageError, StorageErrorCode, classify};

const PACK_QUOTA: u64 = 64;
const PROP_QUOTA: u64 = 256;
const PAGE_LIMIT: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalPropSnapshot {
    pub catalog_revision: u64,
    pub builtin: bool,
    pub installed_at_ms: Option<u64>,
    pub pack: ValidatedPropPack,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalPropMutation {
    pub catalog_revision: u64,
    pub changed: bool,
    pub snapshot: Option<LocalPropSnapshot>,
    pub digest: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalPropExcludedReason {
    Oversized,
    DigestMismatch,
    InvalidDocument,
}

impl LocalPropExcludedReason {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Oversized => "oversized",
            Self::DigestMismatch => "digestMismatch",
            Self::InvalidDocument => "invalidDocument",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalPropExcluded {
    pub digest: String,
    pub reason: LocalPropExcludedReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalPropCatalogList {
    pub catalog_revision: u64,
    pub builtins: Vec<LocalPropSnapshot>,
    pub packs: Vec<LocalPropSnapshot>,
    pub excluded: Vec<LocalPropExcluded>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalPropResolutionBatch {
    pub catalog_revision: u64,
    pub packs: Vec<LocalPropSnapshot>,
    pub unavailable: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedLocalPropPack {
    pub pack: ValidatedPropPack,
    pub builtin: bool,
    pub installed_at_ms: Option<u64>,
}

enum CachedLocalPropPack {
    Available(Box<ResolvedLocalPropPack>),
    Missing,
    Corrupt,
}

/// One bounded catalog loader and cache for one caller-owned SQLite snapshot.
pub(super) struct LocalPropResolver<'connection> {
    connection: &'connection rusqlite::Connection,
    cache: HashMap<String, CachedLocalPropPack>,
    #[cfg(test)]
    database_reads: usize,
}

impl<'connection> LocalPropResolver<'connection> {
    pub(super) fn new(connection: &'connection rusqlite::Connection) -> Self {
        Self {
            connection,
            cache: HashMap::new(),
            #[cfg(test)]
            database_reads: 0,
        }
    }

    pub(super) fn resolve(
        &mut self,
        digest: &str,
    ) -> Result<&ResolvedLocalPropPack, LocalPropCatalogError> {
        if !self.cache.contains_key(digest) {
            let loaded = if let Some(pack) = builtin_by_digest(digest) {
                CachedLocalPropPack::Available(Box::new(ResolvedLocalPropPack {
                    pack: pack.clone(),
                    builtin: true,
                    installed_at_ms: None,
                }))
            } else {
                #[cfg(test)]
                {
                    self.database_reads += 1;
                }
                match load_bounded_row(self.connection, digest)? {
                    Some(row) => match resolved_from_bounded_row(row) {
                        Ok(pack) => CachedLocalPropPack::Available(Box::new(pack)),
                        Err(LocalPropCatalogError::Corrupt) => CachedLocalPropPack::Corrupt,
                        Err(error) => return Err(error),
                    },
                    None => CachedLocalPropPack::Missing,
                }
            };
            self.cache.insert(digest.to_owned(), loaded);
        }
        match self.cache.get(digest).expect("prop cache populated") {
            CachedLocalPropPack::Available(pack) => Ok(pack),
            CachedLocalPropPack::Missing => Err(LocalPropCatalogError::NotFound),
            CachedLocalPropPack::Corrupt => Err(LocalPropCatalogError::Corrupt),
        }
    }

    #[cfg(test)]
    pub(super) fn database_reads(&self) -> usize {
        self.database_reads
    }
}

#[derive(Debug)]
pub enum LocalPropCatalogError {
    Storage(StorageError),
    Invalid,
    Corrupt,
    NotFound,
    Limit,
    Builtin,
    RevisionConflict,
    RevisionExhausted,
    CursorInvalid,
    CursorStale,
}

impl std::fmt::Display for LocalPropCatalogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Storage(_) => "Could not access local Office prop storage.",
            Self::Invalid => "Invalid Office prop pack or digest.",
            Self::Corrupt => "The local Office prop catalog is corrupt.",
            Self::NotFound => "The local Office prop pack was not found.",
            Self::Limit => "The local Office prop catalog limit was reached.",
            Self::Builtin => "The embedded Office prop pack cannot be removed.",
            Self::RevisionConflict => "The local Office prop catalog changed.",
            Self::RevisionExhausted => "The local Office prop catalog revision is exhausted.",
            Self::CursorInvalid => "Invalid local Office prop catalog cursor.",
            Self::CursorStale => "The local Office prop catalog cursor is stale.",
        })
    }
}

impl std::error::Error for LocalPropCatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StorageError> for LocalPropCatalogError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl OfficeStore {
    pub fn install_local_prop_pack(
        &mut self,
        expected_revision: u64,
        candidate: &ValidatedPropPack,
    ) -> Result<LocalPropMutation, LocalPropCatalogError> {
        let now = current_time_ms()?;
        let transaction = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| classify(error, "Acquire local prop catalog lock"))?;
        let state = read_state(&transaction)?;
        if expected_revision != state.revision {
            if state.recognizes("install", candidate.digest(), expected_revision) {
                let stored = load_bounded_row(&transaction, candidate.digest())?
                    .ok_or(LocalPropCatalogError::RevisionConflict)?;
                let snapshot = snapshot_from_bounded_row(state.revision, stored)?;
                if snapshot.pack.bytes() != candidate.bytes() {
                    return Err(LocalPropCatalogError::Corrupt);
                }
                transaction
                    .commit()
                    .map_err(|error| classify(error, "Finish local prop install retry"))?;
                return Ok(LocalPropMutation {
                    catalog_revision: state.revision,
                    changed: false,
                    digest: candidate.digest().to_owned(),
                    snapshot: Some(snapshot),
                });
            }
            return Err(LocalPropCatalogError::RevisionConflict);
        }

        if let Some(pack) = builtin_by_digest(candidate.digest()) {
            if candidate.bytes() != pack.bytes() {
                return Err(LocalPropCatalogError::Corrupt);
            }
            let snapshot = builtin_snapshot(state.revision, pack);
            transaction
                .commit()
                .map_err(|error| classify(error, "Finish built-in prop install no-op"))?;
            return Ok(LocalPropMutation {
                catalog_revision: state.revision,
                changed: false,
                digest: candidate.digest().to_owned(),
                snapshot: Some(snapshot),
            });
        }

        if let Some(stored) = load_bounded_row(&transaction, candidate.digest())? {
            let snapshot = snapshot_from_bounded_row(state.revision, stored)?;
            if snapshot.pack.bytes() != candidate.bytes() {
                return Err(LocalPropCatalogError::Corrupt);
            }
            transaction
                .commit()
                .map_err(|error| classify(error, "Finish local prop install no-op"))?;
            return Ok(LocalPropMutation {
                catalog_revision: state.revision,
                changed: false,
                digest: candidate.digest().to_owned(),
                snapshot: Some(snapshot),
            });
        }

        let (rows, props) = quota_totals(&transaction)?;
        let candidate_props = u64::try_from(candidate.pack().props.len())
            .map_err(|_| LocalPropCatalogError::Invalid)?;
        if rows >= PACK_QUOTA || props.saturating_add(candidate_props) > PROP_QUOTA {
            return Err(LocalPropCatalogError::Limit);
        }
        let next = state
            .next_revision()
            .ok_or(LocalPropCatalogError::RevisionExhausted)?;
        transaction
            .execute(
                "INSERT INTO office_prop_packs (digest, bytes, prop_count, installed_revision, installed_at_ms) VALUES (?, ?, ?, ?, ?)",
                params![
                    candidate.digest(),
                    candidate.bytes(),
                    i64::try_from(candidate_props).expect("prop limit fits SQLite"),
                    i64::try_from(next).expect("catalog revision fits SQLite"),
                    i64::try_from(now).expect("timestamp fits SQLite")
                ],
            )
            .map_err(|error| classify(error, "Install local Office prop pack"))?;
        write_state(
            &transaction,
            next,
            "install",
            candidate.digest(),
            state.revision,
        )?;
        transaction
            .commit()
            .map_err(|error| classify(error, "Commit local Office prop install"))?;
        Ok(LocalPropMutation {
            catalog_revision: next,
            changed: true,
            digest: candidate.digest().to_owned(),
            snapshot: Some(LocalPropSnapshot {
                catalog_revision: next,
                builtin: false,
                installed_at_ms: Some(now),
                pack: candidate.clone(),
            }),
        })
    }

    pub fn remove_local_prop_pack(
        &mut self,
        expected_revision: u64,
        digest: &str,
    ) -> Result<LocalPropMutation, LocalPropCatalogError> {
        if parse_pack_digest(digest).is_none() {
            return Err(LocalPropCatalogError::Invalid);
        }
        if builtin_by_digest(digest).is_some() {
            return Err(LocalPropCatalogError::Builtin);
        }
        let transaction = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| classify(error, "Acquire local prop catalog lock"))?;
        let state = read_state(&transaction)?;
        if expected_revision != state.revision {
            if state.recognizes("remove", digest, expected_revision)
                && !row_exists(&transaction, digest)?
            {
                transaction
                    .commit()
                    .map_err(|error| classify(error, "Finish local prop removal retry"))?;
                return Ok(LocalPropMutation {
                    catalog_revision: state.revision,
                    changed: false,
                    snapshot: None,
                    digest: digest.to_owned(),
                });
            }
            return Err(LocalPropCatalogError::RevisionConflict);
        }
        if !row_exists(&transaction, digest)? {
            transaction
                .commit()
                .map_err(|error| classify(error, "Finish local prop removal no-op"))?;
            return Ok(LocalPropMutation {
                catalog_revision: state.revision,
                changed: false,
                snapshot: None,
                digest: digest.to_owned(),
            });
        }
        let next = state
            .next_revision()
            .ok_or(LocalPropCatalogError::RevisionExhausted)?;
        transaction
            .execute("DELETE FROM office_prop_packs WHERE digest = ?", [digest])
            .map_err(|error| classify(error, "Remove local Office prop pack"))?;
        write_state(&transaction, next, "remove", digest, state.revision)?;
        transaction
            .commit()
            .map_err(|error| classify(error, "Commit local Office prop removal"))?;
        Ok(LocalPropMutation {
            catalog_revision: next,
            changed: true,
            snapshot: None,
            digest: digest.to_owned(),
        })
    }

    pub fn show_local_prop_pack(
        &mut self,
        digest: &str,
    ) -> Result<LocalPropSnapshot, LocalPropCatalogError> {
        if parse_pack_digest(digest).is_none() {
            return Err(LocalPropCatalogError::Invalid);
        }
        let transaction = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| classify(error, "Observe local prop catalog"))?;
        let revision = read_state(&transaction)?.revision;
        let result = {
            let mut resolver = LocalPropResolver::new(&transaction);
            snapshot_from_resolved(revision, resolver.resolve(digest)?.clone())
        };
        transaction
            .commit()
            .map_err(|error| classify(error, "Finish local prop catalog observation"))?;
        Ok(result)
    }

    pub fn resolve_local_prop_packs(
        &mut self,
        digests: &[String],
    ) -> Result<LocalPropResolutionBatch, LocalPropCatalogError> {
        if digests
            .iter()
            .any(|digest| parse_pack_digest(digest).is_none())
        {
            return Err(LocalPropCatalogError::Invalid);
        }
        let transaction = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| classify(error, "Observe local prop catalog"))?;
        let revision = read_state(&transaction)?.revision;
        let mut packs = Vec::new();
        let mut unavailable = Vec::new();
        {
            let mut resolver = LocalPropResolver::new(&transaction);
            for digest in digests {
                match resolver.resolve(digest) {
                    Ok(pack) => packs.push(snapshot_from_resolved(revision, pack.clone())),
                    Err(LocalPropCatalogError::NotFound | LocalPropCatalogError::Corrupt) => {
                        unavailable.push(digest.clone());
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        transaction
            .commit()
            .map_err(|error| classify(error, "Finish local prop catalog observation"))?;
        Ok(LocalPropResolutionBatch {
            catalog_revision: revision,
            packs,
            unavailable,
        })
    }

    pub fn list_local_prop_packs(
        &mut self,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<LocalPropCatalogList, LocalPropCatalogError> {
        if !(1..=PAGE_LIMIT).contains(&limit) {
            return Err(LocalPropCatalogError::CursorInvalid);
        }
        let decoded = cursor.map(decode_cursor).transpose()?;
        let transaction = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| classify(error, "Observe local prop catalog"))?;
        let revision = read_state(&transaction)?.revision;
        let after = match decoded {
            Some((cursor_revision, _)) if cursor_revision != revision => {
                return Err(LocalPropCatalogError::CursorStale);
            }
            Some((_, digest)) => {
                if !row_exists(&transaction, &digest)? {
                    return Err(LocalPropCatalogError::CursorInvalid);
                }
                digest
            }
            None => String::new(),
        };
        let mut statement = transaction
            .prepare(
                "SELECT digest, length(bytes), CASE WHEN length(bytes) <= ? THEN bytes ELSE NULL END, installed_at_ms \
                 FROM office_prop_packs WHERE digest > ? ORDER BY digest COLLATE BINARY LIMIT ?",
            )
            .map_err(|error| classify(error, "Prepare local prop catalog page"))?;
        let rows = statement
            .query_map(
                params![
                    i64::try_from(PACK_INPUT_LIMIT).expect("pack bound fits SQLite"),
                    after,
                    i64::try_from(limit).expect("page limit fits SQLite")
                ],
                |row| {
                    Ok(BoundedRow {
                        digest: row.get(0)?,
                        length: row.get(1)?,
                        bytes: row.get(2)?,
                        installed_at_ms: row.get(3)?,
                    })
                },
            )
            .map_err(|error| classify(error, "Read local prop catalog page"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| classify(error, "Decode local prop catalog page"))?;
        drop(statement);
        let last = rows.last().map(|row| row.digest.clone());
        let mut packs = Vec::new();
        let mut excluded = Vec::new();
        for row in rows {
            match snapshot_from_bounded_row(revision, row.clone()) {
                Ok(snapshot) => packs.push(snapshot),
                Err(LocalPropCatalogError::Corrupt) => {
                    let reason = exclusion_reason(&row);
                    excluded.push(LocalPropExcluded {
                        digest: row.digest,
                        reason,
                    });
                }
                Err(error) => return Err(error),
            }
        }
        let next_cursor = match last {
            Some(last) if has_row_after(&transaction, &last)? => {
                Some(encode_cursor(revision, &last)?)
            }
            _ => None,
        };
        transaction
            .commit()
            .map_err(|error| classify(error, "Finish local prop catalog page"))?;
        Ok(LocalPropCatalogList {
            catalog_revision: revision,
            builtins: builtin_packs()
                .iter()
                .map(|pack| builtin_snapshot(revision, pack))
                .collect(),
            packs,
            excluded,
            next_cursor,
        })
    }
}

#[derive(Debug, Clone)]
struct BoundedRow {
    digest: String,
    length: i64,
    bytes: Option<Vec<u8>>,
    installed_at_ms: i64,
}

fn read_state(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<crate::catalog_replay::CatalogReplayState, LocalPropCatalogError> {
    let (revision, previous_kind, previous_digest, previous_base_revision, previous_result_revision) = transaction
        .query_row(
            "SELECT revision, previous_kind, previous_digest, previous_base_revision, previous_result_revision FROM office_prop_catalog WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get(1)?, row.get(2)?, row.get::<_, Option<i64>>(3)?, row.get::<_, Option<i64>>(4)?)),
        )
        .map_err(|error| classify(error, "Read local prop catalog revision"))?;
    Ok(crate::catalog_replay::CatalogReplayState {
        revision: stored_u64(revision)?,
        previous_kind,
        previous_digest,
        previous_base_revision: stored_optional_u64(previous_base_revision)?,
        previous_result_revision: stored_optional_u64(previous_result_revision)?,
    })
}

fn write_state(
    transaction: &rusqlite::Transaction<'_>,
    result_revision: u64,
    kind: &str,
    digest: &str,
    base_revision: u64,
) -> Result<(), LocalPropCatalogError> {
    let changed = transaction
        .execute(
            "UPDATE office_prop_catalog SET revision = ?, previous_kind = ?, previous_digest = ?, previous_base_revision = ?, previous_result_revision = ? WHERE singleton = 1 AND revision = ?",
            params![
                i64::try_from(result_revision).expect("catalog revision fits SQLite"),
                kind,
                digest,
                i64::try_from(base_revision).expect("catalog revision fits SQLite"),
                i64::try_from(result_revision).expect("catalog revision fits SQLite"),
                i64::try_from(base_revision).expect("catalog revision fits SQLite")
            ],
        )
        .map_err(|error| classify(error, "Advance local prop catalog revision"))?;
    (changed == 1)
        .then_some(())
        .ok_or(LocalPropCatalogError::RevisionConflict)
}

fn quota_totals(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(u64, u64), LocalPropCatalogError> {
    let (rows, props, invalid): (i64, i64, i64) = transaction
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(CASE WHEN prop_count BETWEEN 1 AND 16 THEN prop_count ELSE 0 END), 0), COALESCE(SUM(CASE WHEN prop_count BETWEEN 1 AND 16 THEN 0 ELSE 1 END), 0) FROM office_prop_packs",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| classify(error, "Read local prop catalog quotas"))?;
    if invalid != 0 {
        return Err(LocalPropCatalogError::Corrupt);
    }
    Ok((stored_u64(rows)?, stored_u64(props)?))
}

fn row_exists(
    transaction: &rusqlite::Transaction<'_>,
    digest: &str,
) -> Result<bool, LocalPropCatalogError> {
    transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM office_prop_packs WHERE digest = ?)",
            [digest],
            |row| row.get(0),
        )
        .map_err(|error| classify(error, "Find local prop catalog row").into())
}

fn has_row_after(
    transaction: &rusqlite::Transaction<'_>,
    digest: &str,
) -> Result<bool, LocalPropCatalogError> {
    transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM office_prop_packs WHERE digest > ?)",
            [digest],
            |row| row.get(0),
        )
        .map_err(|error| classify(error, "Find next local prop catalog row").into())
}

fn load_bounded_row(
    connection: &rusqlite::Connection,
    digest: &str,
) -> Result<Option<BoundedRow>, LocalPropCatalogError> {
    connection
        .query_row(
            "SELECT digest, length(bytes), CASE WHEN length(bytes) <= ? THEN bytes ELSE NULL END, installed_at_ms FROM office_prop_packs WHERE digest = ?",
            params![i64::try_from(PACK_INPUT_LIMIT).expect("pack bound fits SQLite"), digest],
            |row| {
                Ok(BoundedRow {
                    digest: row.get(0)?,
                    length: row.get(1)?,
                    bytes: row.get(2)?,
                    installed_at_ms: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|error| classify(error, "Read bounded local prop catalog row").into())
}

fn snapshot_from_bounded_row(
    revision: u64,
    row: BoundedRow,
) -> Result<LocalPropSnapshot, LocalPropCatalogError> {
    Ok(snapshot_from_resolved(
        revision,
        resolved_from_bounded_row(row)?,
    ))
}

fn resolved_from_bounded_row(
    row: BoundedRow,
) -> Result<ResolvedLocalPropPack, LocalPropCatalogError> {
    let bytes = row.bytes.ok_or(LocalPropCatalogError::Corrupt)?;
    let pack = validate_pack(&bytes).map_err(|_| LocalPropCatalogError::Corrupt)?;
    if pack.digest() != row.digest {
        return Err(LocalPropCatalogError::Corrupt);
    }
    Ok(ResolvedLocalPropPack {
        builtin: false,
        installed_at_ms: Some(stored_u64(row.installed_at_ms)?),
        pack,
    })
}

fn snapshot_from_resolved(revision: u64, resolved: ResolvedLocalPropPack) -> LocalPropSnapshot {
    LocalPropSnapshot {
        catalog_revision: revision,
        builtin: resolved.builtin,
        installed_at_ms: resolved.installed_at_ms,
        pack: resolved.pack,
    }
}

fn exclusion_reason(row: &BoundedRow) -> LocalPropExcludedReason {
    if row.length < 0
        || usize::try_from(row.length)
            .ok()
            .is_none_or(|length| length > PACK_INPUT_LIMIT)
    {
        return LocalPropExcludedReason::Oversized;
    }
    match row.bytes.as_deref().map(validate_pack) {
        Some(Err(PropPackError::TooLarge)) => LocalPropExcludedReason::Oversized,
        Some(Ok(pack)) if pack.digest() != row.digest => LocalPropExcludedReason::DigestMismatch,
        _ => LocalPropExcludedReason::InvalidDocument,
    }
}

fn builtin_snapshot(revision: u64, pack: &ValidatedPropPack) -> LocalPropSnapshot {
    LocalPropSnapshot {
        catalog_revision: revision,
        builtin: true,
        installed_at_ms: None,
        pack: pack.clone(),
    }
}

fn encode_cursor(revision: u64, digest: &str) -> Result<String, LocalPropCatalogError> {
    tmt_office_model::codec::catalog_cursor::encode(1, revision, digest)
        .ok_or(LocalPropCatalogError::CursorInvalid)
}

fn decode_cursor(value: &str) -> Result<(u64, String), LocalPropCatalogError> {
    tmt_office_model::codec::catalog_cursor::decode(1, value)
        .ok_or(LocalPropCatalogError::CursorInvalid)
}

fn stored_u64(value: i64) -> Result<u64, LocalPropCatalogError> {
    value.try_into().map_err(|_| LocalPropCatalogError::Corrupt)
}

fn stored_optional_u64(value: Option<i64>) -> Result<Option<u64>, LocalPropCatalogError> {
    value.map(stored_u64).transpose()
}

fn current_time_ms() -> Result<u64, LocalPropCatalogError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            StorageError::new(StorageErrorCode::Unknown, "Read system clock failed")
                .caused_by(error)
        })?
        .as_millis()
        .try_into()
        .map_err(|error| {
            LocalPropCatalogError::Storage(
                StorageError::new(StorageErrorCode::Unknown, "System timestamp is too large")
                    .caused_by(error),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use tmt_office_model::codec::office_prop::BROADCASTER_DIGEST;
    use tmt_office_model::codec::office_prop::BUILTIN_DIGEST;
    use tmt_office_model::codec::office_prop::COMMONS_DIGEST;
    use tmt_office_model::codec::office_prop::MODULAR_LOUNGE_DIGEST;
    use tmt_office_model::codec::office_prop::MODULAR_MOUNTED_DIGEST;
    use tmt_office_model::codec::office_prop::MODULAR_WORKSTATION_DIGEST;
    use tmt_office_model::codec::office_prop::STUDY_DIGEST;
    use tmt_office_model::codec::office_prop::WALL_DIGEST;
    use tmt_office_model::codec::office_prop::WHITEBOARD_DIGEST;
    use tmt_office_model::codec::office_prop::WORKSHOP_DIGEST;
    use tmt_office_model::codec::office_prop::validate_pack;

    fn custom(label: &str) -> ValidatedPropPack {
        validate_pack(
            format!(r##"{{"formatVersion":1,"label":"{label}","credit":"Test","license":"MIT","palette":["#00000000","#ffffffff"],"props":[{{"key":"lamp","label":"Lamp","footprint":{{"width":1,"height":1}},"pixels":["1"]}}]}}"##).as_bytes(),
        )
        .unwrap()
    }

    fn custom_with_props(label: &str, count: usize) -> ValidatedPropPack {
        let props = (0..count)
            .map(|index| {
                serde_json::json!({
                    "key": format!("prop-{index}"),
                    "label": format!("Prop {index}"),
                    "footprint": { "width": 1, "height": 1 },
                    "pixels": ["1"]
                })
            })
            .collect::<Vec<_>>();
        validate_pack(
            &serde_json::to_vec(&serde_json::json!({
                "formatVersion": 1,
                "label": label,
                "credit": "Test",
                "license": "MIT",
                "palette": ["#00000000", "#ffffffff"],
                "props": props
            }))
            .unwrap(),
        )
        .unwrap()
    }

    fn stored_rows(storage: &OfficeStore) -> Vec<(String, Vec<u8>, i64, i64, i64)> {
        storage
            .connection()
            .unwrap()
            .prepare(
                "SELECT digest, bytes, prop_count, installed_revision, installed_at_ms FROM office_prop_packs ORDER BY digest",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn install_list_show_remove_and_exact_retries_share_one_revision() {
        let directory = TestDirectory::new();
        let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
        let pack = custom("First");
        let installed = storage.install_local_prop_pack(0, &pack).unwrap();
        assert!(installed.changed);
        assert_eq!(installed.catalog_revision, 1);
        assert!(!storage.install_local_prop_pack(0, &pack).unwrap().changed);
        assert!(!storage.install_local_prop_pack(1, &pack).unwrap().changed);
        let list = storage.list_local_prop_packs(1, None).unwrap();
        assert_eq!(list.catalog_revision, 1);
        assert_eq!(list.builtins[0].pack.digest(), BUILTIN_DIGEST);
        assert_eq!(list.packs[0].pack.digest(), pack.digest());
        assert_eq!(
            storage.show_local_prop_pack(pack.digest()).unwrap().pack,
            pack
        );

        let removed = storage.remove_local_prop_pack(1, pack.digest()).unwrap();
        assert!(removed.changed);
        assert_eq!(removed.catalog_revision, 2);
        assert!(
            !storage
                .remove_local_prop_pack(1, pack.digest())
                .unwrap()
                .changed
        );
        assert!(
            !storage
                .remove_local_prop_pack(2, pack.digest())
                .unwrap()
                .changed
        );
        assert!(matches!(
            storage.show_local_prop_pack(pack.digest()),
            Err(LocalPropCatalogError::NotFound)
        ));
        storage.close().unwrap();
    }

    #[test]
    fn prop_quota_accepts_256_and_rejects_overflow_without_mutation() {
        let directory = TestDirectory::new();
        let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
        for revision in 0..16 {
            let candidate = custom_with_props(&format!("Pack {revision}"), 16);
            let installed = storage
                .install_local_prop_pack(revision, &candidate)
                .unwrap();
            assert!(installed.changed);
            assert_eq!(installed.catalog_revision, revision + 1);
        }

        let before_catalog = storage.list_local_prop_packs(20, None).unwrap();
        assert_eq!(before_catalog.catalog_revision, 16);
        assert_eq!(before_catalog.packs.len(), 16);
        assert_eq!(
            before_catalog
                .packs
                .iter()
                .map(|entry| entry.pack.pack().props.len())
                .sum::<usize>(),
            256
        );
        assert!(before_catalog.next_cursor.is_none());
        let before_rows = stored_rows(&storage);
        assert_eq!(before_rows.len(), 16);
        assert_eq!(
            before_rows
                .iter()
                .map(|(_, _, prop_count, _, _)| prop_count)
                .sum::<i64>(),
            256
        );

        let overflow = custom_with_props("Overflow", 16);
        assert!(matches!(
            storage.install_local_prop_pack(16, &overflow),
            Err(LocalPropCatalogError::Limit)
        ));
        assert_eq!(
            storage.list_local_prop_packs(20, None).unwrap(),
            before_catalog
        );
        assert_eq!(stored_rows(&storage), before_rows);
        storage.close().unwrap();
    }

    #[test]
    fn directional_pack_above_legacy_bound_survives_storage_reopen() {
        let directory = TestDirectory::new();
        let database = directory.path.join("state.db");
        let mut bytes = include_bytes!(
            "../../../../../extensions/tmt-office/contracts/prop-pack-v2-sample.tmtprop.json"
        )
        .to_vec();
        bytes.resize(
            tmt_office_model::codec::office_prop::V1_PACK_INPUT_LIMIT + 1,
            b' ',
        );
        let pack = tmt_office_model::codec::office_prop::validate_pack(&bytes).unwrap();
        let mut storage = OfficeStore::open(&database).unwrap();
        let mutation = storage.install_local_prop_pack(0, &pack).unwrap();
        assert!(mutation.changed);
        storage.close().unwrap();
        let mut reopened = OfficeStore::open(&database).unwrap();
        let saved = reopened.show_local_prop_pack(pack.digest()).unwrap();
        assert_eq!(saved.pack.bytes(), bytes);
        assert_eq!(saved.pack.pack().format_version, 2);
        assert!(!reopened.install_local_prop_pack(1, &pack).unwrap().changed);
        reopened.close().unwrap();
    }

    #[test]
    fn v1_rows_above_their_format_limit_are_excluded_as_oversized() {
        let directory = TestDirectory::new();
        let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
        let pack = custom("Oversized v1");
        storage.install_local_prop_pack(0, &pack).unwrap();
        let mut bytes = pack.bytes().to_vec();
        bytes.resize(
            tmt_office_model::codec::office_prop::V1_PACK_INPUT_LIMIT + 1,
            b' ',
        );
        storage
            .connection()
            .unwrap()
            .execute(
                "UPDATE office_prop_packs SET bytes = ? WHERE digest = ?",
                params![bytes, pack.digest()],
            )
            .unwrap();
        let list = storage.list_local_prop_packs(20, None).unwrap();
        assert!(list.packs.is_empty());
        assert_eq!(list.excluded.len(), 1);
        assert_eq!(list.excluded[0].reason, LocalPropExcludedReason::Oversized);
        assert_eq!(list.catalog_revision, 1);
        storage.close().unwrap();
    }

    #[test]
    fn install_noop_and_retry_bound_corrupt_same_digest_rows() {
        let oversized = vec![b' '; PACK_INPUT_LIMIT + 1];

        let current_directory = TestDirectory::new();
        let mut current = OfficeStore::open(current_directory.path.join("state.db")).unwrap();
        let pack = custom("Current");
        current
            .connection()
            .unwrap()
            .execute_batch("PRAGMA ignore_check_constraints = ON")
            .unwrap();
        current
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO office_prop_packs (digest, bytes, prop_count, installed_revision, installed_at_ms) VALUES (?, ?, 1, 0, 1)",
                params![pack.digest(), oversized],
            )
            .unwrap();
        current
            .connection()
            .unwrap()
            .execute_batch("PRAGMA ignore_check_constraints = OFF")
            .unwrap();
        assert!(matches!(
            current.install_local_prop_pack(0, &pack),
            Err(LocalPropCatalogError::Corrupt)
        ));
        assert_eq!(
            current
                .connection()
                .unwrap()
                .query_row("SELECT revision FROM office_prop_catalog", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );

        let retry_directory = TestDirectory::new();
        let mut retry = OfficeStore::open(retry_directory.path.join("state.db")).unwrap();
        let pack = custom("Retry");
        retry.install_local_prop_pack(0, &pack).unwrap();
        retry
            .connection()
            .unwrap()
            .execute_batch("PRAGMA ignore_check_constraints = ON")
            .unwrap();
        retry
            .connection()
            .unwrap()
            .execute(
                "UPDATE office_prop_packs SET bytes = ? WHERE digest = ?",
                params![vec![b' '; PACK_INPUT_LIMIT + 1], pack.digest()],
            )
            .unwrap();
        retry
            .connection()
            .unwrap()
            .execute_batch("PRAGMA ignore_check_constraints = OFF")
            .unwrap();
        assert!(matches!(
            retry.install_local_prop_pack(0, &pack),
            Err(LocalPropCatalogError::Corrupt)
        ));
        assert_eq!(
            retry
                .connection()
                .unwrap()
                .query_row("SELECT revision FROM office_prop_catalog", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn builtin_is_discoverable_install_noop_and_remove_protected() {
        let directory = TestDirectory::new();
        let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
        for builtin in builtin_packs() {
            let mut resolver = LocalPropResolver::new(storage.connection().unwrap());
            assert_eq!(
                resolver.resolve(builtin.digest()).unwrap().pack.bytes(),
                builtin.bytes()
            );
            assert_eq!(resolver.database_reads, 0);
            let result = storage.install_local_prop_pack(0, builtin).unwrap();
            assert!(!result.changed);
            assert_eq!(result.catalog_revision, 0);
            assert!(result.snapshot.unwrap().builtin);
            let shown = storage.show_local_prop_pack(builtin.digest()).unwrap();
            assert!(shown.builtin);
            assert_eq!(shown.pack.bytes(), builtin.bytes());
            assert!(shown.installed_at_ms.is_none());
            assert!(matches!(
                storage.remove_local_prop_pack(0, builtin.digest()),
                Err(LocalPropCatalogError::Builtin)
            ));
        }
        let list = storage.list_local_prop_packs(1, None).unwrap();
        assert_eq!(list.catalog_revision, 0);
        assert_eq!(
            list.builtins
                .iter()
                .map(|item| item.pack.digest())
                .collect::<Vec<_>>(),
            [
                BUILTIN_DIGEST,
                WORKSHOP_DIGEST,
                COMMONS_DIGEST,
                WHITEBOARD_DIGEST,
                BROADCASTER_DIGEST,
                STUDY_DIGEST,
                WALL_DIGEST,
                MODULAR_WORKSTATION_DIGEST,
                MODULAR_MOUNTED_DIGEST,
                MODULAR_LOUNGE_DIGEST,
                tmt_office_model::codec::office_prop::MODULAR_FACILITIES_DIGEST,
                tmt_office_model::codec::office_prop::MODULAR_RECEPTION_DIGEST,
                tmt_office_model::codec::office_prop::DIRECTIONAL_WORKSTATION_DIGEST,
                tmt_office_model::codec::office_prop::DIRECTIONAL_LOUNGE_DIGEST,
                tmt_office_model::codec::office_prop::DIRECTIONAL_RECEPTION_DIGEST,
                tmt_office_model::codec::office_prop::DIRECTIONAL_FACILITIES_DIGEST,
            ]
        );
        assert!(list.packs.is_empty());
        assert!(list.next_cursor.is_none());
        assert_eq!(
            storage
                .connection()
                .unwrap()
                .query_row("SELECT count(*) FROM office_prop_packs", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        storage.close().unwrap();
    }

    #[test]
    fn next_observation_detects_same_revision_byte_tampering_and_remove_recovers() {
        let directory = TestDirectory::new();
        let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
        let pack = custom("Tamper");
        storage.install_local_prop_pack(0, &pack).unwrap();
        storage
            .connection()
            .unwrap()
            .execute(
                "UPDATE office_prop_packs SET bytes = x'7b7d' WHERE digest = ?",
                [pack.digest()],
            )
            .unwrap();
        let list = storage.list_local_prop_packs(20, None).unwrap();
        assert!(list.packs.is_empty());
        assert_eq!(
            list.excluded[0].reason,
            LocalPropExcludedReason::InvalidDocument
        );
        assert!(
            storage
                .remove_local_prop_pack(1, pack.digest())
                .unwrap()
                .changed
        );
        storage.close().unwrap();
    }

    #[test]
    fn pagination_advances_past_an_all_corrupt_page_and_rejects_stale_cursors() {
        let directory = TestDirectory::new();
        let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
        let candidates = [custom("A"), custom("B")];
        let [first, second] = if candidates[0].digest() < candidates[1].digest() {
            candidates
        } else {
            [candidates[1].clone(), candidates[0].clone()]
        };
        storage.install_local_prop_pack(0, &first).unwrap();
        storage.install_local_prop_pack(1, &second).unwrap();
        storage
            .connection()
            .unwrap()
            .execute(
                "UPDATE office_prop_packs SET bytes = x'7b7d' WHERE digest = ?",
                [first.digest()],
            )
            .unwrap();

        let page = storage.list_local_prop_packs(1, None).unwrap();
        assert!(page.packs.is_empty());
        assert_eq!(page.excluded.len(), 1);
        assert_eq!(page.excluded[0].digest, first.digest());
        let cursor = page
            .next_cursor
            .expect("a corrupt leading row still advances");
        let next = storage.list_local_prop_packs(1, Some(&cursor)).unwrap();
        assert_eq!(next.packs.len(), 1);
        assert_eq!(next.packs[0].pack.digest(), second.digest());
        assert!(next.excluded.is_empty());
        assert!(next.next_cursor.is_none());

        let third = custom("C");
        storage.install_local_prop_pack(2, &third).unwrap();
        assert!(matches!(
            storage.list_local_prop_packs(1, Some(&cursor)),
            Err(LocalPropCatalogError::CursorStale)
        ));
        storage.close().unwrap();
    }
}
