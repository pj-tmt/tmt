//! Installation-owned Office data stored beside the existing identity repository.

use rusqlite::{OptionalExtension, params};
use tmt_office_model::office_block::{
    BlockLayout, LocalBlockLayout, LocalBlockTarget, MAX_REVISION, PropPlacement,
};

use tmt_office_model::codec::office_prop::parse_prop_reference;

use crate::{
    OfficeStore,
    office_prop::{LocalPropCatalogError, LocalPropResolver},
    store::{preflight_complete, with_immediate_transaction},
};
use tmt_adapters::storage::{StorageError, StorageErrorCode, classify};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalBlockSnapshot {
    pub target: LocalBlockTarget,
    pub label: String,
    pub block_id: Option<String>,
    pub revision: u64,
    pub layout: LocalBlockLayout,
    pub updated_at_ms: u64,
    pub changed: bool,
    pub resolutions: Vec<LocalPropResolution>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalPropResolution {
    Available { label: String },
    Unavailable { digest_prefix: String },
}

impl LocalBlockSnapshot {
    pub fn exists(&self) -> bool {
        self.block_id.is_some()
    }
}

#[derive(Debug)]
pub enum LocalOfficeError {
    Storage(StorageError),
    IdentityInactive,
    RevisionConflict,
    RevisionExhausted,
    StoredLayoutInvalid,
    LayoutInvalid,
    PropNotFound,
    PropCorrupt,
}

impl std::fmt::Display for LocalOfficeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Storage(_) => "Could not access local Office storage.",
            Self::IdentityInactive => "The selected identity is not active.",
            Self::RevisionConflict => "The local Office block changed.",
            Self::RevisionExhausted => "The local Office block revision is exhausted.",
            Self::StoredLayoutInvalid => "The stored local Office layout is invalid.",
            Self::LayoutInvalid => "The proposed local Office layout is invalid.",
            Self::PropNotFound => "The referenced local Office prop is unavailable.",
            Self::PropCorrupt => "The referenced local Office prop pack is corrupt.",
        })
    }
}

impl std::error::Error for LocalOfficeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StorageError> for LocalOfficeError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl OfficeStore {
    pub fn show_local_block(
        &mut self,
        target: &LocalBlockTarget,
    ) -> Result<LocalBlockSnapshot, LocalOfficeError> {
        let label = target_label(self, target)?;
        let transaction = self
            .connection_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
            .map_err(|error| classify(error, "Observe local Office block"))?;
        let stored = read_block(&transaction, target)?;
        let snapshot = snapshot(
            &mut LocalPropResolver::new(&transaction),
            target,
            label,
            stored,
        )?;
        transaction
            .commit()
            .map_err(|error| classify(error, "Finish local Office block observation"))?;
        Ok(snapshot)
    }

    pub fn list_local_blocks(&mut self) -> Result<Vec<LocalBlockSnapshot>, LocalOfficeError> {
        // Core owns the active roster and its order; Office joins its own rows.
        let active: std::collections::HashMap<String, (usize, String)> = self
            .references()
            .active_identities()?
            .into_iter()
            .enumerate()
            .map(|(order, identity)| (identity.id, (order, identity.name)))
            .collect();
        let transaction = self
            .connection_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
            .map_err(|error| classify(error, "Observe active local Office blocks"))?;
        let mut statement = transaction
            .prepare(
                "SELECT identity_id, block_id, revision, layout, updated_at_ms \
                 FROM office_local_blocks WHERE target_kind = 'identity'",
            )
            .map_err(|error| classify(error, "Prepare active local Office blocks"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map_err(|error| classify(error, "List active local Office blocks"))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| classify(error, "Read active local Office block"))?;
        drop(statement);
        let mut rows: Vec<_> = rows
            .into_iter()
            .filter_map(|(identity_id, block_id, revision, layout, updated_at_ms)| {
                let (order, name) = active.get(&identity_id)?.clone();
                Some((
                    order,
                    identity_id,
                    name,
                    block_id,
                    revision,
                    layout,
                    updated_at_ms,
                ))
            })
            .collect();
        rows.sort_by(|left, right| (left.0, &left.3).cmp(&(right.0, &right.3)));
        let mut resolver = LocalPropResolver::new(&transaction);
        let mut snapshots = vec![snapshot(
            &mut resolver,
            &LocalBlockTarget::Lobby,
            "Lobby".into(),
            read_block(&transaction, &LocalBlockTarget::Lobby)?,
        )?];
        snapshots.extend(
            rows.into_iter()
                .map(
                    |(_, identity_id, identity_name, block_id, revision, layout, updated_at_ms)| {
                        snapshot(
                            &mut resolver,
                            &LocalBlockTarget::Identity(identity_id),
                            identity_name,
                            Some((block_id, revision, layout, updated_at_ms)),
                        )
                    },
                )
                .collect::<Result<Vec<_>, LocalOfficeError>>()?,
        );
        transaction
            .commit()
            .map_err(|error| classify(error, "Finish active local Office block observation"))?;
        Ok(snapshots)
    }

    pub fn apply_local_block(
        &mut self,
        target: &LocalBlockTarget,
        expected_revision: u64,
        layout: &LocalBlockLayout,
    ) -> Result<LocalBlockSnapshot, LocalOfficeError> {
        let label = target_label(self, target)?;
        preflight_complete();
        with_immediate_transaction(self, "local Office block", |transaction| {
            // A retirement Office recorded after preflight blocks this write.
            if let LocalBlockTarget::Identity(identity_id) = target
                && crate::retirement::is_marked(
                    transaction,
                    crate::retirement::Marker::Identity,
                    identity_id,
                )?
            {
                return Err(LocalOfficeError::IdentityInactive);
            }
            let current = read_block(transaction, target)?;
            let encoded = serde_json::to_string(
                &tmt_office_model::codec::office_block::local_layout_value(layout),
            )
            .map_err(|error| {
                StorageError::new(
                    StorageErrorCode::Unknown,
                    "Encode local Office layout failed",
                )
                .caused_by(error)
            })?;
            let now = current_time_ms()?;
            if encoded.len() > tmt_office_model::office_block::STORED_LAYOUT_LIMIT {
                return Err(LocalOfficeError::LayoutInvalid);
            }
            let mut resolver = LocalPropResolver::new(transaction);
            let (block_id, revision, updated_at_ms, changed) = match current {
                None if expected_revision == 0 => {
                    validate_mutation(None, layout, &mut resolver)?;
                    crate::office_world::ensure_world(transaction, || {
                        Ok(i64::try_from(now).expect("current timestamp fits SQLite"))
                    })?;
                    let block_id = uuid::Uuid::new_v4().to_string();
                    transaction
                        .execute(
                            "INSERT INTO office_local_blocks (block_id, target_kind, identity_id, revision, layout, updated_at_ms) VALUES (?, ?, ?, 1, ?, ?)",
                            params![
                                block_id,
                                target.kind(),
                                target.identity_id(),
                                encoded,
                                i64::try_from(now).expect("current timestamp fits SQLite")
                            ],
                        )
                        .map_err(|error| classify(error, "Create local Office block"))?;
                    (block_id, 1, now, true)
                }
                None => return Err(LocalOfficeError::RevisionConflict),
                Some((block_id, revision, stored_layout, updated_at_ms))
                    if expected_revision.checked_add(1) == Some(stored_u64(revision)?)
                        && decode_layout(&stored_layout)? == *layout =>
                {
                    (
                        block_id,
                        stored_u64(revision)?,
                        stored_u64(updated_at_ms)?,
                        false,
                    )
                }
                Some((_, revision, _, _)) if stored_u64(revision)? != expected_revision => {
                    return Err(LocalOfficeError::RevisionConflict);
                }
                Some((block_id, revision, stored_layout, updated_at_ms))
                    if decode_layout(&stored_layout)? == *layout =>
                {
                    (
                        block_id,
                        stored_u64(revision)?,
                        stored_u64(updated_at_ms)?,
                        false,
                    )
                }
                Some((_, revision, _, _)) if stored_u64(revision)? >= MAX_REVISION => {
                    return Err(LocalOfficeError::RevisionExhausted);
                }
                Some((block_id, stored_revision, stored_layout, _)) => {
                    let current_layout = decode_layout(&stored_layout)?;
                    validate_mutation(Some(&current_layout), layout, &mut resolver)?;
                    let revision = stored_u64(stored_revision)?;
                    let next = revision + 1;
                    let changed = transaction
                        .execute(
                            "UPDATE office_local_blocks SET revision = ?, layout = ?, updated_at_ms = ? \
                             WHERE block_id = ? AND target_kind = ? AND identity_id IS ? AND revision = ?",
                            params![
                                i64::try_from(next).expect("safe Office revision fits SQLite"),
                                encoded,
                                i64::try_from(now).expect("current timestamp fits SQLite"),
                                block_id,
                                target.kind(),
                                target.identity_id(),
                                stored_revision
                            ],
                        )
                        .map_err(|error| classify(error, "Update local Office block"))?;
                    if changed != 1 {
                        return Err(LocalOfficeError::RevisionConflict);
                    }
                    (block_id, next, now, true)
                }
            };
            let resolutions = resolve_layout(layout, &mut resolver)?;
            Ok(LocalBlockSnapshot {
                target: target.clone(),
                label,
                block_id: Some(block_id),
                revision,
                layout: layout.clone(),
                updated_at_ms,
                changed,
                resolutions,
            })
        })
    }
}

type StoredBlock = (String, i64, String, i64);

/// Preflight: identity blocks belong only to active identities.
fn target_label(
    store: &OfficeStore,
    target: &LocalBlockTarget,
) -> Result<String, LocalOfficeError> {
    match target {
        LocalBlockTarget::Lobby => Ok("Lobby".into()),
        LocalBlockTarget::Identity(identity_id) => {
            match store.references().identity(identity_id)? {
                Some(identity) if !identity.retired => Ok(identity.name),
                _ => Err(LocalOfficeError::IdentityInactive),
            }
        }
    }
}

fn read_block(
    connection: &rusqlite::Connection,
    target: &LocalBlockTarget,
) -> Result<Option<StoredBlock>, LocalOfficeError> {
    connection.query_row(
        "SELECT block_id, revision, layout, updated_at_ms FROM office_local_blocks WHERE target_kind = ? AND identity_id IS ?",
        params![target.kind(), target.identity_id()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).optional().map_err(|error| classify(error, "Read local Office block").into())
}

fn snapshot(
    resolver: &mut LocalPropResolver<'_>,
    target: &LocalBlockTarget,
    label: String,
    stored: Option<StoredBlock>,
) -> Result<LocalBlockSnapshot, LocalOfficeError> {
    let Some((block_id, revision, layout, updated_at_ms)) = stored else {
        let layout = tmt_office_model::codec::office_block::default_local_layout(target);
        let resolutions = resolve_layout(&layout, resolver)?;
        return Ok(LocalBlockSnapshot {
            target: target.clone(),
            label,
            block_id: None,
            revision: 0,
            layout,
            updated_at_ms: 0,
            changed: false,
            resolutions,
        });
    };
    let layout = decode_layout(&layout)?;
    let resolutions = resolve_layout(&layout, resolver)?;
    Ok(LocalBlockSnapshot {
        target: target.clone(),
        label,
        block_id: Some(block_id),
        revision: stored_u64(revision)?,
        layout,
        updated_at_ms: stored_u64(updated_at_ms)?,
        changed: false,
        resolutions,
    })
}

fn stored_u64(value: i64) -> Result<u64, LocalOfficeError> {
    value
        .try_into()
        .map_err(|_| LocalOfficeError::StoredLayoutInvalid)
}

pub(super) fn decode_layout(value: &str) -> Result<LocalBlockLayout, LocalOfficeError> {
    if let Ok(tokens) = serde_json::from_str::<Vec<String>>(value) {
        return BlockLayout::decode(&tokens)
            .map(|layout| LocalBlockLayout::from_legacy(&layout))
            .map_err(|_| LocalOfficeError::StoredLayoutInvalid);
    }
    tmt_office_model::codec::office_block::decode_local_layout(value.as_bytes())
        .map_err(|_| LocalOfficeError::StoredLayoutInvalid)
}

fn resolve_layout(
    layout: &LocalBlockLayout,
    resolver: &mut LocalPropResolver<'_>,
) -> Result<Vec<LocalPropResolution>, LocalOfficeError> {
    layout
        .objects()
        .iter()
        .map(|item| match resolve_item(item, resolver)? {
            ItemResolution::Available(label) => Ok(LocalPropResolution::Available { label }),
            ItemResolution::Missing
            | ItemResolution::Corrupt
            | ItemResolution::DefinitionMismatch => Ok(LocalPropResolution::Unavailable {
                digest_prefix: digest_prefix(&item.prop),
            }),
        })
        .collect()
}

fn validate_mutation(
    current: Option<&LocalBlockLayout>,
    proposed: &LocalBlockLayout,
    resolver: &mut LocalPropResolver<'_>,
) -> Result<(), LocalOfficeError> {
    let mut retained = std::collections::HashMap::<PropPlacement, usize>::new();
    if let Some(current) = current {
        for item in current.objects() {
            if !matches!(resolve_item(item, resolver)?, ItemResolution::Available(_)) {
                *retained.entry(item.clone()).or_default() += 1;
            }
        }
    }
    for item in proposed.objects() {
        let resolution = resolve_item(item, resolver)?;
        if matches!(resolution, ItemResolution::Available(_)) {
            continue;
        }
        if let Some(count) = retained.get_mut(item).filter(|count| **count > 0) {
            *count -= 1;
            continue;
        }
        return Err(match resolution {
            ItemResolution::Corrupt => LocalOfficeError::PropCorrupt,
            ItemResolution::DefinitionMismatch => LocalOfficeError::LayoutInvalid,
            ItemResolution::Missing => LocalOfficeError::PropNotFound,
            ItemResolution::Available(_) => unreachable!(),
        });
    }
    Ok(())
}

pub(super) enum ItemResolution {
    Available(String),
    Missing,
    Corrupt,
    DefinitionMismatch,
}

pub(super) fn resolve_item(
    item: &PropPlacement,
    resolver: &mut LocalPropResolver<'_>,
) -> Result<ItemResolution, LocalOfficeError> {
    let (digest, key) =
        parse_prop_reference(&item.prop).ok_or(LocalOfficeError::StoredLayoutInvalid)?;
    match resolver.resolve(digest) {
        Ok(resolved) => match resolved.pack.prop(key) {
            Some(prop)
                if prop.footprint.width == item.footprint_width
                    && prop.footprint.height == item.footprint_height
                    && prop.permits_customization(item.customization.as_ref()) =>
            {
                Ok(ItemResolution::Available(prop.label.clone()))
            }
            Some(_) => Ok(ItemResolution::DefinitionMismatch),
            None => Ok(ItemResolution::Missing),
        },
        Err(LocalPropCatalogError::NotFound) => Ok(ItemResolution::Missing),
        Err(LocalPropCatalogError::Corrupt) => Ok(ItemResolution::Corrupt),
        Err(LocalPropCatalogError::Storage(error)) => Err(LocalOfficeError::Storage(error)),
        Err(_) => Err(LocalOfficeError::StoredLayoutInvalid),
    }
}

fn digest_prefix(reference: &str) -> String {
    reference
        .strip_prefix("sha256:")
        .and_then(|value| value.get(..12))
        .unwrap_or("000000000000")
        .to_owned()
}

fn current_time_ms() -> Result<u64, LocalOfficeError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| {
            StorageError::new(StorageErrorCode::Unknown, "Read system clock failed")
                .caused_by(error)
        })?
        .as_millis()
        .try_into()
        .map_err(|error| {
            LocalOfficeError::Storage(
                StorageError::new(StorageErrorCode::Unknown, "System timestamp is too large")
                    .caused_by(error),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use tmt_office_model::codec::office_prop::validate_pack;
    use tmt_office_model::office_block::{Furniture, FurnitureAsset};

    fn insert_identity(storage: &OfficeStore, id: &str, name: &str, lifetime: &str) {
        storage
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO identities (id, name, canonical_name, lifetime, created_at, updated_at) \
                 VALUES (?, ?, lower(?), ?, 'now', 'now')",
                params![id, name, name, lifetime],
            )
            .unwrap();
    }

    #[test]
    fn lobby_defaults_are_read_only_and_an_empty_override_survives_restart() {
        let directory = TestDirectory::new();
        let path = directory.path.join("state.db");
        let mut storage = OfficeStore::open(&path).unwrap();
        let target = LocalBlockTarget::Lobby;
        let initial = storage.show_local_block(&target).unwrap();
        assert!(!initial.exists());
        assert_eq!((initial.revision, initial.updated_at_ms), (0, 0));
        assert_eq!(initial.layout.objects().len(), 10);
        assert!(
            initial
                .resolutions
                .iter()
                .all(|item| matches!(item, LocalPropResolution::Available { .. }))
        );
        assert_eq!(storage.list_local_blocks().unwrap(), vec![initial.clone()]);
        for table in [
            "identities",
            "office_local_worlds",
            "office_local_blocks",
            "office_prop_packs",
        ] {
            let count: i64 = storage
                .connection()
                .unwrap()
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "observation must not materialize {table}");
        }

        let saved = storage
            .apply_local_block(&target, 0, &initial.layout)
            .unwrap();
        assert!(saved.changed);
        assert_eq!(saved.revision, 1);
        assert!(saved.exists());
        let retry = storage
            .apply_local_block(&target, 0, &initial.layout)
            .unwrap();
        assert!(!retry.changed);
        assert_eq!(retry.block_id, saved.block_id);
        assert_eq!(retry.updated_at_ms, saved.updated_at_ms);
        let empty = LocalBlockLayout::new(Vec::new()).unwrap();
        assert!(matches!(
            storage.apply_local_block(&target, 0, &empty),
            Err(LocalOfficeError::RevisionConflict)
        ));

        let cleared = storage.apply_local_block(&target, 1, &empty).unwrap();
        assert_eq!(cleared.revision, 2);
        assert_eq!(cleared.block_id, saved.block_id);
        storage.close().unwrap();
        let mut reopened = OfficeStore::open(&path).unwrap();
        let restored = reopened.show_local_block(&target).unwrap();
        assert!(restored.exists());
        assert_eq!(restored.revision, 2);
        assert_eq!(restored.layout, empty);
        assert_eq!(reopened.list_local_blocks().unwrap(), vec![restored]);
        let identities: i64 = reopened
            .connection()
            .unwrap()
            .query_row("SELECT count(*) FROM identities", [], |row| row.get(0))
            .unwrap();
        assert_eq!(identities, 0);
        reopened.close().unwrap();
    }

    #[test]
    fn invalid_lobby_prop_does_not_create_a_world_or_override() {
        let directory = TestDirectory::new();
        let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
        let layout = LocalBlockLayout::new(vec![PropPlacement {
            prop: format!("sha256:{}/missing", "a".repeat(64)),
            footprint_width: 1,
            footprint_height: 1,
            x: 0,
            y: 0,
            rotation: 0,
            customization: None,
        }])
        .unwrap();
        assert!(matches!(
            storage.apply_local_block(&LocalBlockTarget::Lobby, 0, &layout),
            Err(LocalOfficeError::PropNotFound)
        ));
        for table in ["office_local_worlds", "office_local_blocks"] {
            let count: i64 = storage
                .connection()
                .unwrap()
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0);
        }
        assert!(
            !storage
                .show_local_block(&LocalBlockTarget::Lobby)
                .unwrap()
                .exists()
        );
        storage.close().unwrap();
    }

    #[test]
    fn customized_layouts_preserve_capacity_and_unavailable_occurrences_across_restart() {
        use tmt_office_model::office_block::PropCustomization;
        let directory = TestDirectory::new();
        let path = directory.path.join("state.db");
        let mut storage = OfficeStore::open(&path).unwrap();
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../extensions/tmt-office/contracts/prop-customization-vectors.json"
        ))
        .unwrap();
        let mut input = vectors["pack"].clone();
        let key = format!("p{}", "x".repeat(31));
        input["props"][0]["key"] = serde_json::json!(key);
        input["props"][0]["customization"] = serde_json::json!({
            "tint":vectors["cases"][0]["value"]["tint"],
            "text":vectors["cases"][1]["value"]["text"]
        });
        let pack = validate_pack(&serde_json::to_vec(&input).unwrap()).unwrap();
        storage.install_local_prop_pack(0, &pack).unwrap();
        // Multibyte text exercises the byte ceiling independently of character count.
        let item = PropPlacement {
            prop: format!("{}/{key}", pack.digest()),
            footprint_width: 2,
            footprint_height: 1,
            x: 0,
            y: 0,
            rotation: 0,
            customization: Some(PropCustomization {
                tint: Some("#cc8855".into()),
                text: Some("\u{754c}".repeat(21)),
            }),
        };
        let layout = LocalBlockLayout::new(vec![item; 16]).unwrap();
        let encoded = serde_json::to_vec(
            &tmt_office_model::codec::office_block::local_layout_value(&layout),
        )
        .unwrap();
        assert!(encoded.len() > 4096);
        assert!(encoded.len() <= tmt_office_model::office_block::STORED_LAYOUT_LIMIT);
        let target = LocalBlockTarget::Lobby;
        let saved = storage.apply_local_block(&target, 0, &layout).unwrap();
        assert_eq!(saved.revision, 1);
        assert!(
            !storage
                .apply_local_block(&target, 0, &layout)
                .unwrap()
                .changed
        );
        storage.close().unwrap();
        let mut storage = OfficeStore::open(&path).unwrap();
        assert_eq!(storage.show_local_block(&target).unwrap().layout, layout);
        storage.remove_local_prop_pack(1, pack.digest()).unwrap();
        let missing = storage.show_local_block(&target).unwrap();
        assert_eq!(missing.layout, layout);
        assert!(
            missing
                .resolutions
                .iter()
                .all(|item| matches!(item, LocalPropResolution::Unavailable { .. }))
        );
        let mut changed = layout.objects().to_vec();
        changed[0].customization.as_mut().unwrap().text = Some("Changed".into());
        assert!(matches!(
            storage.apply_local_block(&target, 1, &LocalBlockLayout::new(changed).unwrap()),
            Err(LocalOfficeError::PropNotFound)
        ));
        let retained = LocalBlockLayout::new(layout.objects()[1..].to_vec()).unwrap();
        assert_eq!(
            storage
                .apply_local_block(&target, 1, &retained)
                .unwrap()
                .revision,
            2
        );
        storage.install_local_prop_pack(2, &pack).unwrap();
        let restored = storage.show_local_block(&target).unwrap();
        assert_eq!(restored.layout, retained);
        assert_eq!(restored.revision, 2);
        assert!(
            restored
                .resolutions
                .iter()
                .all(|item| matches!(item, LocalPropResolution::Available { .. }))
        );
        storage.close().unwrap();
    }

    #[test]
    fn a_prop_without_declared_capabilities_cannot_receive_custom_values() {
        let directory = TestDirectory::new();
        let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
        let mut layout = LocalBlockLayout::from_legacy(
            &BlockLayout::new(vec![Furniture {
                asset: FurnitureAsset::Rug,
                x: 0,
                y: 0,
                rotation: 0,
            }])
            .unwrap(),
        )
        .objects()
        .to_vec();
        layout[0].customization = Some(tmt_office_model::office_block::PropCustomization {
            tint: Some("#123456".into()),
            text: None,
        });
        assert!(matches!(
            storage.apply_local_block(
                &LocalBlockTarget::Lobby,
                0,
                &LocalBlockLayout::new(layout).unwrap()
            ),
            Err(LocalOfficeError::LayoutInvalid)
        ));
        assert!(
            !storage
                .show_local_block(&LocalBlockTarget::Lobby)
                .unwrap()
                .exists()
        );
        storage.close().unwrap();
    }

    #[test]
    fn one_snapshot_resolves_repeated_digest_once_across_validation_and_projection() {
        let directory = TestDirectory::new();
        let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
        let pack = validate_pack(br##"{"formatVersion":1,"label":"Cache","credit":"Test","license":"MIT","palette":["#00000000","#ffffffff"],"props":[{"key":"lamp","label":"Lamp","footprint":{"width":1,"height":1},"pixels":["1"]}]}"##).unwrap();
        storage.install_local_prop_pack(0, &pack).unwrap();
        let layout = LocalBlockLayout::new(vec![
            PropPlacement {
                prop: format!("{}/lamp", pack.digest()),
                footprint_width: 1,
                footprint_height: 1,
                x: 1,
                y: 1,
                rotation: 0,
                customization: None,
            },
            PropPlacement {
                prop: format!("{}/lamp", pack.digest()),
                footprint_width: 1,
                footprint_height: 1,
                x: 2,
                y: 2,
                rotation: 0,
                customization: None,
            },
        ])
        .unwrap();
        let transaction = storage
            .connection_mut()
            .unwrap()
            .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
            .unwrap();
        {
            let mut resolver = LocalPropResolver::new(&transaction);
            validate_mutation(None, &layout, &mut resolver).unwrap();
            assert_eq!(resolve_layout(&layout, &mut resolver).unwrap().len(), 2);
            assert_eq!(resolve_layout(&layout, &mut resolver).unwrap().len(), 2);
            assert_eq!(resolver.database_reads(), 1);
        }
        transaction.commit().unwrap();
        storage.close().unwrap();
    }

    #[test]
    fn local_blocks_use_identity_uuid_cas_and_active_projection() {
        let directory = TestDirectory::new();
        let mut storage = OfficeStore::open(directory.path.join("state.db")).unwrap();
        let first = "11111111-1111-4111-8111-111111111111";
        let first_target = LocalBlockTarget::Identity(first.into());
        insert_identity(&storage, first, "Alice", "temporary");
        let missing = storage.show_local_block(&first_target).unwrap();
        assert!(!missing.exists());
        assert_eq!(missing.revision, 0);

        let legacy_layout = BlockLayout::new(vec![Furniture {
            asset: FurnitureAsset::Desk,
            x: 1,
            y: 2,
            rotation: 0,
        }])
        .unwrap();
        let layout = LocalBlockLayout::from_legacy(&legacy_layout);
        let created = storage
            .apply_local_block(&first_target, 0, &layout)
            .unwrap();
        assert_eq!(created.revision, 1);
        let retried = storage
            .apply_local_block(&first_target, 0, &layout)
            .unwrap();
        assert_eq!(retried.revision, created.revision);
        assert_eq!(retried.layout, created.layout);
        assert!(!retried.changed);
        assert!(matches!(
            storage.apply_local_block(
                &first_target,
                0,
                &LocalBlockLayout::new(Vec::new()).expect("empty layout is valid")
            ),
            Err(LocalOfficeError::RevisionConflict)
        ));

        storage
            .connection()
            .unwrap()
            .execute(
                "UPDATE identities SET retired_at_ms = 1 WHERE id = ?",
                [first],
            )
            .unwrap();
        assert!(matches!(
            storage.show_local_block(&first_target),
            Err(LocalOfficeError::IdentityInactive)
        ));
        let active = storage.list_local_blocks().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].target, LocalBlockTarget::Lobby);

        let second = "22222222-2222-4222-8222-222222222222";
        let second_target = LocalBlockTarget::Identity(second.into());
        insert_identity(&storage, second, "Alice", "saved");
        assert!(!storage.show_local_block(&second_target).unwrap().exists());
        let replacement = storage
            .apply_local_block(&second_target, 0, &layout)
            .unwrap();
        assert_ne!(replacement.block_id, created.block_id);
        let active = storage.list_local_blocks().unwrap();
        assert_eq!(active.len(), 2);
        assert_eq!(active[1].target, second_target);
        storage.close().unwrap();
    }
    #[test]
    fn block_identity_retired_inside_the_preflight_window_commits_like_apply_then_retire() {
        let directory = TestDirectory::new();
        let path = directory.path.join("race.db");
        let mut storage = OfficeStore::open(&path).unwrap();
        let id = "32323232-1111-4111-8111-111111111111";
        insert_identity(&storage, id, "Alice", "saved");
        let target = LocalBlockTarget::Identity(id.into());
        let initial = storage.show_local_block(&target).unwrap();
        crate::test_support::at_next_preflight_execute(
            &path,
            "UPDATE identities SET retired_at_ms = 1 WHERE id = ?",
            id,
        );
        let saved = storage
            .apply_local_block(&target, 0, &initial.layout)
            .unwrap();
        assert_eq!(saved.revision, 1);
        // Serial equivalent: the row is retained; the active projection hides it.
        let rows: i64 = storage
            .connection()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM office_local_blocks WHERE identity_id = ?",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1);
        assert_eq!(storage.list_local_blocks().unwrap().len(), 1, "Lobby only");
        assert!(matches!(
            storage.apply_local_block(&target, 1, &initial.layout),
            Err(LocalOfficeError::IdentityInactive)
        ));
    }
}
