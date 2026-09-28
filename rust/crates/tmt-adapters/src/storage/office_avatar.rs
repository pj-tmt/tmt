//! SQLite-backed local Office avatar catalog with its own revision and cursor domain.

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::{Storage, StorageError, StorageErrorCode, errors::classify};
use tmt_office_model::codec::office_avatar::PACK_INPUT_LIMIT;
use tmt_office_model::codec::office_avatar::ValidatedAvatarPack;
use tmt_office_model::codec::office_avatar::builtin_by_digest;
use tmt_office_model::codec::office_avatar::builtin_packs;
use tmt_office_model::codec::office_avatar::parse_pack_digest;
use tmt_office_model::codec::office_avatar::validate_pack;

const PACK_QUOTA: u64 = 64;
const AVATAR_QUOTA: u64 = 256;
const PAGE_LIMIT: usize = 20;
const CURSOR_DOMAIN: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAvatarSnapshot {
    pub catalog_revision: u64,
    pub installed_at_ms: Option<u64>,
    pub pack: ValidatedAvatarPack,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAvatarMutation {
    pub catalog_revision: u64,
    pub changed: bool,
    pub digest: String,
    pub snapshot: Option<LocalAvatarSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalAvatarExcludedReason {
    Oversized,
    InvalidDocument,
    DigestMismatch,
}

impl LocalAvatarExcludedReason {
    pub fn code(self) -> &'static str {
        match self {
            Self::Oversized => "oversized",
            Self::InvalidDocument => "invalidDocument",
            Self::DigestMismatch => "digestMismatch",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAvatarExcluded {
    pub digest: String,
    pub reason: LocalAvatarExcludedReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAvatarCatalogList {
    pub catalog_revision: u64,
    pub builtins: Vec<LocalAvatarSnapshot>,
    pub packs: Vec<LocalAvatarSnapshot>,
    pub excluded: Vec<LocalAvatarExcluded>,
    pub next_cursor: Option<String>,
}

#[derive(Debug)]
pub enum LocalAvatarCatalogError {
    Storage(StorageError),
    Invalid,
    Corrupt,
    NotFound,
    Limit,
    RevisionConflict,
    RevisionExhausted,
    CursorInvalid,
    CursorStale,
}

impl std::fmt::Display for LocalAvatarCatalogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Storage(_) => "Could not access local Office avatar storage.",
            Self::Invalid => "Invalid Office avatar pack or digest.",
            Self::Corrupt => "The local Office avatar catalog is corrupt.",
            Self::NotFound => "The local Office avatar pack was not found.",
            Self::Limit => "The local Office avatar catalog limit was reached.",
            Self::RevisionConflict => "The local Office avatar catalog changed.",
            Self::RevisionExhausted => "The local Office avatar catalog revision is exhausted.",
            Self::CursorInvalid => "Invalid local Office avatar catalog cursor.",
            Self::CursorStale => "The local Office avatar catalog cursor is stale.",
        })
    }
}
impl std::error::Error for LocalAvatarCatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            _ => None,
        }
    }
}
impl From<StorageError> for LocalAvatarCatalogError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl Storage {
    pub fn install_local_avatar_pack(
        &mut self,
        expected_revision: u64,
        candidate: &ValidatedAvatarPack,
    ) -> Result<LocalAvatarMutation, LocalAvatarCatalogError> {
        let now = current_time_ms()?;
        let transaction = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| classify(error, "Acquire local avatar catalog lock"))?;
        let state = read_state(&transaction)?;
        if expected_revision != state.revision {
            if state.recognizes("install", candidate.digest(), expected_revision) {
                let snapshot = load_snapshot(&transaction, state.revision, candidate.digest())?
                    .ok_or(LocalAvatarCatalogError::RevisionConflict)?;
                if snapshot.pack.bytes() != candidate.bytes() {
                    return Err(LocalAvatarCatalogError::Corrupt);
                }
                transaction
                    .commit()
                    .map_err(|error| classify(error, "Finish local avatar install retry"))?;
                return Ok(LocalAvatarMutation {
                    catalog_revision: state.revision,
                    changed: false,
                    digest: candidate.digest().into(),
                    snapshot: Some(snapshot),
                });
            }
            return Err(LocalAvatarCatalogError::RevisionConflict);
        }
        if let Some(snapshot) = load_snapshot(&transaction, state.revision, candidate.digest())? {
            if snapshot.pack.bytes() != candidate.bytes() {
                return Err(LocalAvatarCatalogError::Corrupt);
            }
            transaction
                .commit()
                .map_err(|error| classify(error, "Finish local avatar install no-op"))?;
            return Ok(LocalAvatarMutation {
                catalog_revision: state.revision,
                changed: false,
                digest: candidate.digest().into(),
                snapshot: Some(snapshot),
            });
        }
        let (packs, avatars) = quota_totals(&transaction)?;
        let candidate_avatars = u64::try_from(candidate.pack().avatars.len())
            .map_err(|_| LocalAvatarCatalogError::Invalid)?;
        if packs >= PACK_QUOTA || avatars.saturating_add(candidate_avatars) > AVATAR_QUOTA {
            return Err(LocalAvatarCatalogError::Limit);
        }
        let next = state
            .next_revision()
            .ok_or(LocalAvatarCatalogError::RevisionExhausted)?;
        transaction.execute(
            "INSERT INTO office_avatar_packs (digest, bytes, avatar_count, installed_revision, installed_at_ms) VALUES (?, ?, ?, ?, ?)",
            params![candidate.digest(), candidate.bytes(), i64::try_from(candidate_avatars).unwrap(), i64::try_from(next).unwrap(), i64::try_from(now).unwrap()],
        ).map_err(|error| classify(error, "Install local Office avatar pack"))?;
        write_state(
            &transaction,
            next,
            "install",
            candidate.digest(),
            state.revision,
        )?;
        transaction
            .commit()
            .map_err(|error| classify(error, "Commit local Office avatar install"))?;
        Ok(LocalAvatarMutation {
            catalog_revision: next,
            changed: true,
            digest: candidate.digest().into(),
            snapshot: Some(LocalAvatarSnapshot {
                catalog_revision: next,
                installed_at_ms: Some(now),
                pack: candidate.clone(),
            }),
        })
    }

    pub fn remove_local_avatar_pack(
        &mut self,
        expected_revision: u64,
        digest: &str,
    ) -> Result<LocalAvatarMutation, LocalAvatarCatalogError> {
        if parse_pack_digest(digest).is_none() || builtin_by_digest(digest).is_some() {
            return Err(LocalAvatarCatalogError::Invalid);
        }
        let transaction = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| classify(error, "Acquire local avatar catalog lock"))?;
        let state = read_state(&transaction)?;
        if expected_revision != state.revision {
            if state.recognizes("remove", digest, expected_revision)
                && !row_exists(&transaction, digest)?
            {
                transaction
                    .commit()
                    .map_err(|error| classify(error, "Finish local avatar removal retry"))?;
                return Ok(LocalAvatarMutation {
                    catalog_revision: state.revision,
                    changed: false,
                    digest: digest.into(),
                    snapshot: None,
                });
            }
            return Err(LocalAvatarCatalogError::RevisionConflict);
        }
        if !row_exists(&transaction, digest)? {
            transaction
                .commit()
                .map_err(|error| classify(error, "Finish local avatar removal no-op"))?;
            return Ok(LocalAvatarMutation {
                catalog_revision: state.revision,
                changed: false,
                digest: digest.into(),
                snapshot: None,
            });
        }
        let next = state
            .next_revision()
            .ok_or(LocalAvatarCatalogError::RevisionExhausted)?;
        transaction
            .execute("DELETE FROM office_avatar_packs WHERE digest = ?", [digest])
            .map_err(|error| classify(error, "Remove local Office avatar pack"))?;
        write_state(&transaction, next, "remove", digest, state.revision)?;
        transaction
            .commit()
            .map_err(|error| classify(error, "Commit local Office avatar removal"))?;
        Ok(LocalAvatarMutation {
            catalog_revision: next,
            changed: true,
            digest: digest.into(),
            snapshot: None,
        })
    }

    pub fn show_local_avatar_pack(
        &mut self,
        digest: &str,
    ) -> Result<LocalAvatarSnapshot, LocalAvatarCatalogError> {
        if parse_pack_digest(digest).is_none() {
            return Err(LocalAvatarCatalogError::Invalid);
        }
        let transaction = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| classify(error, "Observe local avatar catalog"))?;
        let revision = read_state(&transaction)?.revision;
        let snapshot = load_snapshot(&transaction, revision, digest)?
            .ok_or(LocalAvatarCatalogError::NotFound)?;
        transaction
            .commit()
            .map_err(|error| classify(error, "Finish local avatar catalog observation"))?;
        Ok(snapshot)
    }

    pub fn list_local_avatar_packs(
        &mut self,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<LocalAvatarCatalogList, LocalAvatarCatalogError> {
        if !(1..=PAGE_LIMIT).contains(&limit) {
            return Err(LocalAvatarCatalogError::CursorInvalid);
        }
        let decoded = cursor.map(decode_cursor).transpose()?;
        let transaction = self
            .connection_mut()?
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|error| classify(error, "Observe local avatar catalog"))?;
        let revision = read_state(&transaction)?.revision;
        let after = match decoded {
            Some((cursor_revision, _)) if cursor_revision != revision => {
                return Err(LocalAvatarCatalogError::CursorStale);
            }
            Some((_, digest)) if !row_exists(&transaction, &digest)? => {
                return Err(LocalAvatarCatalogError::CursorInvalid);
            }
            Some((_, digest)) => digest,
            None => String::new(),
        };
        let mut statement = transaction.prepare(
            "SELECT digest, length(bytes), CASE WHEN length(bytes) <= ? THEN bytes ELSE NULL END, installed_at_ms FROM office_avatar_packs WHERE digest > ? ORDER BY digest COLLATE BINARY LIMIT ?"
        ).map_err(|error| classify(error, "Prepare local avatar catalog page"))?;
        let rows = statement
            .query_map(
                params![
                    i64::try_from(PACK_INPUT_LIMIT).unwrap(),
                    after,
                    i64::try_from(limit).unwrap()
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
            .map_err(|error| classify(error, "Read local avatar catalog page"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| classify(error, "Decode local avatar catalog page"))?;
        drop(statement);
        let last = rows.last().map(|row| row.digest.clone());
        let mut packs = Vec::new();
        let mut excluded = Vec::new();
        for row in rows {
            match snapshot_from_row(revision, row.clone()) {
                Ok(snapshot) => packs.push(snapshot),
                Err(LocalAvatarCatalogError::Corrupt) => excluded.push(LocalAvatarExcluded {
                    digest: row.digest.clone(),
                    reason: exclusion_reason(&row),
                }),
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
            .map_err(|error| classify(error, "Finish local avatar catalog page"))?;
        Ok(LocalAvatarCatalogList {
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
) -> Result<super::catalog_replay::CatalogReplayState, LocalAvatarCatalogError> {
    let values = transaction.query_row(
        "SELECT revision, previous_kind, previous_digest, previous_base_revision, previous_result_revision FROM office_avatar_catalog WHERE singleton = 1", [],
        |row| Ok((row.get::<_, i64>(0)?, row.get(1)?, row.get(2)?, row.get::<_, Option<i64>>(3)?, row.get::<_, Option<i64>>(4)?)),
    ).map_err(|error| classify(error, "Read local avatar catalog revision"))?;
    Ok(super::catalog_replay::CatalogReplayState {
        revision: stored_u64(values.0)?,
        previous_kind: values.1,
        previous_digest: values.2,
        previous_base_revision: values.3.map(stored_u64).transpose()?,
        previous_result_revision: values.4.map(stored_u64).transpose()?,
    })
}

fn write_state(
    transaction: &rusqlite::Transaction<'_>,
    result: u64,
    kind: &str,
    digest: &str,
    base: u64,
) -> Result<(), LocalAvatarCatalogError> {
    let changed = transaction.execute(
        "UPDATE office_avatar_catalog SET revision = ?, previous_kind = ?, previous_digest = ?, previous_base_revision = ?, previous_result_revision = ? WHERE singleton = 1 AND revision = ?",
        params![i64::try_from(result).unwrap(), kind, digest, i64::try_from(base).unwrap(), i64::try_from(result).unwrap(), i64::try_from(base).unwrap()],
    ).map_err(|error| classify(error, "Advance local avatar catalog revision"))?;
    (changed == 1)
        .then_some(())
        .ok_or(LocalAvatarCatalogError::RevisionConflict)
}

fn quota_totals(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(u64, u64), LocalAvatarCatalogError> {
    let (rows, avatars, invalid): (i64, i64, i64) = transaction.query_row(
        "SELECT COUNT(*), COALESCE(SUM(CASE WHEN avatar_count BETWEEN 1 AND 16 THEN avatar_count ELSE 0 END), 0), COALESCE(SUM(CASE WHEN avatar_count BETWEEN 1 AND 16 THEN 0 ELSE 1 END), 0) FROM office_avatar_packs", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).map_err(|error| classify(error, "Read local avatar catalog quotas"))?;
    if invalid != 0 {
        return Err(LocalAvatarCatalogError::Corrupt);
    }
    Ok((stored_u64(rows)?, stored_u64(avatars)?))
}

fn row_exists(
    connection: &rusqlite::Connection,
    digest: &str,
) -> Result<bool, LocalAvatarCatalogError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM office_avatar_packs WHERE digest = ?)",
            [digest],
            |row| row.get(0),
        )
        .map_err(|error| classify(error, "Find local avatar catalog row").into())
}
fn has_row_after(
    connection: &rusqlite::Connection,
    digest: &str,
) -> Result<bool, LocalAvatarCatalogError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM office_avatar_packs WHERE digest > ?)",
            [digest],
            |row| row.get(0),
        )
        .map_err(|error| classify(error, "Find next local avatar catalog row").into())
}

fn load_row(
    connection: &rusqlite::Connection,
    digest: &str,
) -> Result<Option<BoundedRow>, LocalAvatarCatalogError> {
    connection.query_row(
        "SELECT digest, length(bytes), CASE WHEN length(bytes) <= ? THEN bytes ELSE NULL END, installed_at_ms FROM office_avatar_packs WHERE digest = ?",
        params![i64::try_from(PACK_INPUT_LIMIT).unwrap(), digest],
        |row| Ok(BoundedRow { digest: row.get(0)?, length: row.get(1)?, bytes: row.get(2)?, installed_at_ms: row.get(3)? }),
    ).optional().map_err(|error| classify(error, "Read bounded local avatar catalog row").into())
}
fn load_snapshot(
    connection: &rusqlite::Connection,
    revision: u64,
    digest: &str,
) -> Result<Option<LocalAvatarSnapshot>, LocalAvatarCatalogError> {
    if let Some(pack) = builtin_by_digest(digest) {
        return Ok(Some(builtin_snapshot(revision, pack)));
    }
    load_row(connection, digest)?
        .map(|row| snapshot_from_row(revision, row))
        .transpose()
}

pub(super) fn reference_available(
    connection: &rusqlite::Connection,
    value: &str,
) -> Result<bool, StorageError> {
    let Some((digest, key)) =
        tmt_office_model::office_art_reference::parse_office_art_reference(value)
    else {
        return Ok(false);
    };
    match load_snapshot(connection, 0, digest) {
        Ok(Some(snapshot)) => Ok(snapshot
            .pack
            .pack()
            .avatars
            .iter()
            .any(|avatar| avatar.key == key)),
        Ok(None)
        | Err(LocalAvatarCatalogError::Corrupt)
        | Err(LocalAvatarCatalogError::Invalid) => Ok(false),
        Err(LocalAvatarCatalogError::Storage(error)) => Err(error),
        Err(_) => Ok(false),
    }
}
fn snapshot_from_row(
    revision: u64,
    row: BoundedRow,
) -> Result<LocalAvatarSnapshot, LocalAvatarCatalogError> {
    let bytes = row.bytes.ok_or(LocalAvatarCatalogError::Corrupt)?;
    let pack = validate_pack(&bytes).map_err(|_| LocalAvatarCatalogError::Corrupt)?;
    if pack.digest() != row.digest {
        return Err(LocalAvatarCatalogError::Corrupt);
    }
    Ok(LocalAvatarSnapshot {
        catalog_revision: revision,
        installed_at_ms: Some(stored_u64(row.installed_at_ms)?),
        pack,
    })
}
fn builtin_snapshot(revision: u64, pack: &ValidatedAvatarPack) -> LocalAvatarSnapshot {
    LocalAvatarSnapshot {
        catalog_revision: revision,
        installed_at_ms: None,
        pack: pack.clone(),
    }
}
fn exclusion_reason(row: &BoundedRow) -> LocalAvatarExcludedReason {
    if row.length < 0
        || usize::try_from(row.length)
            .ok()
            .is_none_or(|length| length > PACK_INPUT_LIMIT)
    {
        return LocalAvatarExcludedReason::Oversized;
    }
    match row
        .bytes
        .as_deref()
        .and_then(|bytes| validate_pack(bytes).ok())
    {
        Some(pack) if pack.digest() != row.digest => LocalAvatarExcludedReason::DigestMismatch,
        _ => LocalAvatarExcludedReason::InvalidDocument,
    }
}
fn encode_cursor(revision: u64, digest: &str) -> Result<String, LocalAvatarCatalogError> {
    super::catalog_cursor::encode(CURSOR_DOMAIN, revision, digest)
        .ok_or(LocalAvatarCatalogError::CursorInvalid)
}
fn decode_cursor(value: &str) -> Result<(u64, String), LocalAvatarCatalogError> {
    super::catalog_cursor::decode(CURSOR_DOMAIN, value)
        .ok_or(LocalAvatarCatalogError::CursorInvalid)
}
fn stored_u64(value: i64) -> Result<u64, LocalAvatarCatalogError> {
    value
        .try_into()
        .map_err(|_| LocalAvatarCatalogError::Corrupt)
}
fn current_time_ms() -> Result<u64, LocalAvatarCatalogError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            StorageError::new(StorageErrorCode::Unknown, "Read system clock failed")
                .caused_by(error)
        })?
        .as_millis()
        .try_into()
        .map_err(|error| {
            LocalAvatarCatalogError::Storage(
                StorageError::new(StorageErrorCode::Unknown, "System timestamp is too large")
                    .caused_by(error),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    type CatalogState = (
        (
            i64,
            i64,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<i64>,
        ),
        Vec<(String, Vec<u8>, i64, i64, i64)>,
    );

    fn sample() -> ValidatedAvatarPack {
        validate_pack(include_bytes!(
            "../../../../../contracts/office/avatar-pack-v1-sample.tmtavatar.json"
        ))
        .unwrap()
    }

    fn named(index: usize) -> ValidatedAvatarPack {
        let source =
            include_str!("../../../../../contracts/office/avatar-pack-v1-sample.tmtavatar.json");
        validate_pack(
            source
                .replace("Signal bots", &format!("Signal bots {index}"))
                .as_bytes(),
        )
        .unwrap()
    }

    fn multi(index: usize, count: usize) -> ValidatedAvatarPack {
        let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../../contracts/office/avatar-pack-v1-sample.tmtavatar.json"
        ))
        .unwrap();
        value["label"] = serde_json::json!(format!("Multi {index}"));
        let template = value["avatars"][0].clone();
        value["avatars"] = serde_json::json!(
            (0..count)
                .map(|avatar_index| {
                    let mut avatar = template.clone();
                    avatar["key"] = serde_json::json!(format!("bot-{avatar_index}"));
                    avatar
                })
                .collect::<Vec<_>>()
        );
        validate_pack(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    fn catalog_state(storage: &Storage) -> CatalogState {
        let connection = storage.connection().unwrap();
        let catalog = connection
            .query_row(
                "SELECT singleton, revision, previous_kind, previous_digest, previous_base_revision, previous_result_revision FROM office_avatar_catalog",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .unwrap();
        let mut statement = connection
            .prepare("SELECT digest, bytes, avatar_count, installed_revision, installed_at_ms FROM office_avatar_packs ORDER BY digest COLLATE BINARY")
            .unwrap();
        let packs = statement
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
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        (catalog, packs)
    }

    #[test]
    fn bundled_robots_resolve_without_seeding_or_advancing_the_catalog() {
        let directory = crate::test_support::TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("avatar.db")).unwrap();
        let before = catalog_state(&storage);
        let pack = &builtin_packs()[0];
        let listed = storage.list_local_avatar_packs(20, None).unwrap();
        assert_eq!(listed.builtins.len(), 1);
        assert!(listed.packs.is_empty());
        assert_eq!(listed.catalog_revision, 0);
        let shown = storage.show_local_avatar_pack(pack.digest()).unwrap();
        assert_eq!(shown.pack, *pack);
        assert_eq!(shown.installed_at_ms, None);
        for avatar in &pack.pack().avatars {
            assert!(
                reference_available(
                    storage.connection().unwrap(),
                    &format!("{}/{}", pack.digest(), avatar.key)
                )
                .unwrap()
            );
        }
        assert!(
            !reference_available(
                storage.connection().unwrap(),
                &format!("{}/missing", pack.digest())
            )
            .unwrap()
        );
        assert!(!storage.install_local_avatar_pack(0, pack).unwrap().changed);
        assert!(matches!(
            storage.remove_local_avatar_pack(0, pack.digest()),
            Err(LocalAvatarCatalogError::Invalid)
        ));
        assert_eq!(catalog_state(&storage), before);
    }

    #[test]
    fn install_show_list_remove_and_retries_are_revisioned() {
        let directory = crate::test_support::TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("avatar.db")).unwrap();
        let candidate = sample();
        let installed = storage.install_local_avatar_pack(0, &candidate).unwrap();
        assert!(installed.changed);
        assert_eq!(installed.catalog_revision, 1);
        assert!(
            !storage
                .install_local_avatar_pack(0, &candidate)
                .unwrap()
                .changed
        );
        assert!(
            !storage
                .install_local_avatar_pack(1, &candidate)
                .unwrap()
                .changed
        );
        assert_eq!(
            storage
                .show_local_avatar_pack(candidate.digest())
                .unwrap()
                .pack,
            candidate
        );
        let list = storage.list_local_avatar_packs(20, None).unwrap();
        assert_eq!(list.catalog_revision, 1);
        assert_eq!(list.packs.len(), 1);
        assert!(
            storage
                .remove_local_avatar_pack(1, candidate.digest())
                .unwrap()
                .changed
        );
        assert!(
            !storage
                .remove_local_avatar_pack(1, candidate.digest())
                .unwrap()
                .changed
        );
        assert!(matches!(
            storage.show_local_avatar_pack(candidate.digest()),
            Err(LocalAvatarCatalogError::NotFound)
        ));
    }

    #[test]
    fn prop_and_avatar_catalog_revisions_and_cursor_domains_are_independent() {
        let directory = crate::test_support::TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("catalogs.db")).unwrap();
        let avatar = sample();
        storage.install_local_avatar_pack(0, &avatar).unwrap();
        assert_eq!(
            storage
                .list_local_prop_packs(20, None)
                .unwrap()
                .catalog_revision,
            0
        );
        let avatar_cursor = encode_cursor(1, avatar.digest()).unwrap();
        assert!(super::super::catalog_cursor::decode(1, &avatar_cursor).is_none());
        assert!(matches!(
            storage.show_local_avatar_pack(tmt_office_model::codec::office_prop::BUILTIN_DIGEST),
            Err(LocalAvatarCatalogError::NotFound)
        ));
    }

    #[test]
    fn stale_unrelated_mutations_leave_the_exact_catalog_state_unchanged() {
        let directory = crate::test_support::TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("stale.db")).unwrap();
        let installed = sample();
        let unrelated = named(1);
        storage.install_local_avatar_pack(0, &installed).unwrap();
        let before = catalog_state(&storage);
        assert!(matches!(
            storage.install_local_avatar_pack(0, &unrelated),
            Err(LocalAvatarCatalogError::RevisionConflict)
        ));
        assert_eq!(catalog_state(&storage), before);
        assert!(matches!(
            storage.remove_local_avatar_pack(0, unrelated.digest()),
            Err(LocalAvatarCatalogError::RevisionConflict)
        ));
        assert_eq!(catalog_state(&storage), before);
    }

    #[test]
    fn corrupt_rows_are_bounded_excluded_rejected_by_show_and_removable() {
        let directory = crate::test_support::TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("corrupt.db")).unwrap();
        let invalid = sample();
        let wrong_digest = named(1);
        let oversized = named(2);
        storage
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO office_avatar_packs (digest, bytes, avatar_count, installed_revision, installed_at_ms) VALUES (?, ?, 1, 1, 1), (?, ?, 1, 2, 1)",
                params![
                    invalid.digest(),
                    b"{}".as_slice(),
                    wrong_digest.digest(),
                    sample().bytes()
                ],
            )
            .unwrap();
        storage
            .connection()
            .unwrap()
            .execute_batch("PRAGMA ignore_check_constraints = ON")
            .unwrap();
        storage
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO office_avatar_packs (digest, bytes, avatar_count, installed_revision, installed_at_ms) VALUES (?, ?, 1, 3, 1)",
                params![oversized.digest(), vec![b' '; PACK_INPUT_LIMIT + 1]],
            )
            .unwrap();
        storage
            .connection()
            .unwrap()
            .execute_batch("PRAGMA ignore_check_constraints = OFF")
            .unwrap();
        let list = storage.list_local_avatar_packs(20, None).unwrap();
        assert!(list.packs.is_empty());
        for (pack, reason) in [
            (&invalid, LocalAvatarExcludedReason::InvalidDocument),
            (&wrong_digest, LocalAvatarExcludedReason::DigestMismatch),
            (&oversized, LocalAvatarExcludedReason::Oversized),
        ] {
            assert!(list.excluded.contains(&LocalAvatarExcluded {
                digest: pack.digest().into(),
                reason,
            }));
            assert!(matches!(
                storage.show_local_avatar_pack(pack.digest()),
                Err(LocalAvatarCatalogError::Corrupt)
            ));
        }
        let mut revision = 0;
        for pack in [&invalid, &wrong_digest, &oversized] {
            let removed = storage
                .remove_local_avatar_pack(revision, pack.digest())
                .unwrap();
            assert!(removed.changed);
            revision = removed.catalog_revision;
        }
        assert!(
            storage
                .list_local_avatar_packs(20, None)
                .unwrap()
                .packs
                .is_empty()
        );
        assert!(
            storage
                .list_local_avatar_packs(20, None)
                .unwrap()
                .excluded
                .is_empty()
        );
    }

    #[test]
    fn corrupt_payloads_still_consume_pack_and_avatar_capacity() {
        let pack_directory = crate::test_support::TestDirectory::new();
        let mut packs = Storage::open(pack_directory.path.join("packs.db")).unwrap();
        let mut pack_revision = 0;
        for index in 0..PACK_QUOTA {
            pack_revision = packs
                .install_local_avatar_pack(pack_revision, &named(index as usize))
                .unwrap()
                .catalog_revision;
        }
        packs
            .connection()
            .unwrap()
            .execute(
                "UPDATE office_avatar_packs SET bytes = ? WHERE installed_revision = 1",
                [b"{}".as_slice()],
            )
            .unwrap();
        assert!(matches!(
            packs.install_local_avatar_pack(pack_revision, &named(PACK_QUOTA as usize)),
            Err(LocalAvatarCatalogError::Limit)
        ));

        let avatar_directory = crate::test_support::TestDirectory::new();
        let mut avatars = Storage::open(avatar_directory.path.join("avatars.db")).unwrap();
        let mut avatar_revision = 0;
        for index in 0..16 {
            avatar_revision = avatars
                .install_local_avatar_pack(avatar_revision, &multi(index, 16))
                .unwrap()
                .catalog_revision;
        }
        avatars
            .connection()
            .unwrap()
            .execute(
                "UPDATE office_avatar_packs SET bytes = ? WHERE installed_revision = 1",
                [b"{}".as_slice()],
            )
            .unwrap();
        assert!(matches!(
            avatars.install_local_avatar_pack(avatar_revision, &multi(16, 1)),
            Err(LocalAvatarCatalogError::Limit)
        ));
    }

    #[test]
    fn pack_quota_and_stale_cursor_are_enforced() {
        let directory = crate::test_support::TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("quota.db")).unwrap();
        let mut revision = 0;
        for index in 0..PACK_QUOTA {
            let mutation = storage
                .install_local_avatar_pack(revision, &named(index as usize))
                .unwrap();
            revision = mutation.catalog_revision;
        }
        assert!(matches!(
            storage.install_local_avatar_pack(revision, &named(PACK_QUOTA as usize)),
            Err(LocalAvatarCatalogError::Limit)
        ));
        let first = storage.list_local_avatar_packs(1, None).unwrap();
        let cursor = first.next_cursor.unwrap();
        let removed = first.packs[0].pack.digest().to_owned();
        storage
            .remove_local_avatar_pack(revision, &removed)
            .unwrap();
        assert!(matches!(
            storage.list_local_avatar_packs(1, Some(&cursor)),
            Err(LocalAvatarCatalogError::CursorStale)
        ));
    }

    #[test]
    fn avatar_quota_is_independent_from_pack_quota() {
        let directory = crate::test_support::TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("avatar-quota.db")).unwrap();
        let mut revision = 0;
        for index in 0..16 {
            revision = storage
                .install_local_avatar_pack(revision, &multi(index, 16))
                .unwrap()
                .catalog_revision;
        }
        assert!(matches!(
            storage.install_local_avatar_pack(revision, &multi(16, 1)),
            Err(LocalAvatarCatalogError::Limit)
        ));
        assert_eq!(
            storage
                .list_local_prop_packs(20, None)
                .unwrap()
                .catalog_revision,
            0
        );
    }
}
