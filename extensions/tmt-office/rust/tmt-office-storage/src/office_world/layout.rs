use super::{ensure_world, legacy};
use crate::{
    OfficeStore,
    core_references::{CoreIdentity, CoreReferences, CoreRoom},
    office_local::{ItemResolution, LocalOfficeError, resolve_item},
    office_prop::LocalPropResolver,
    store::{preflight_complete, with_immediate_transaction_and_references},
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use tmt_adapters::storage::{StorageError, classify};
use tmt_core::identity::Lifetime;
use tmt_core::limits::MAX_JS_SAFE_INTEGER;
use tmt_office_model::codec::office_extension::ExtensionError;
use tmt_office_model::codec::office_extension::validate_attachment;
use tmt_office_model::codec::office_world::WORLD_DOCUMENT_LIMIT;
use tmt_office_model::codec::office_world::decode_world;
use tmt_office_model::codec::office_world::world_value;
use tmt_office_model::office_map::AreaKind;
pub use tmt_office_model::office_world::LocalWorldSnapshot;
use tmt_office_model::office_world::WorldLayout;

#[derive(Debug)]
pub enum WorldStoreError {
    Storage(StorageError),
    Prop(LocalOfficeError),
    Extension(ExtensionError),
    StoredInvalid,
    MigrationInvalid,
    RevisionConflict,
    RevisionExhausted,
    InvalidInput,
    IdentityIneligible,
    RoomMissing,
    PropUnavailable,
}
impl std::fmt::Display for WorldStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Storage(_) => "Could not access Office world storage.",
            Self::Extension(_) => "An Office object has an incompatible resource binding.",
            Self::Prop(_) | Self::PropUnavailable => {
                "A new or changed Office artwork reference is unavailable."
            }
            Self::StoredInvalid => "The stored Office world cannot be decoded safely.",
            Self::MigrationInvalid => {
                "Existing Office layouts cannot fit the world safely; original data is unchanged."
            }
            Self::RevisionConflict => {
                "The Office world changed. Keep this draft and reload before saving."
            }
            Self::RevisionExhausted => "The Office world revision is exhausted.",
            Self::InvalidInput => "Invalid Office world save input.",
            Self::IdentityIneligible => {
                "Only active saved identities may receive a new personal area assignment."
            }
            Self::RoomMissing => "The selected meeting room no longer exists.",
        })
    }
}
impl std::error::Error for WorldStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(e) => Some(e),
            Self::Prop(e) => Some(e),
            Self::Extension(e) => Some(e),
            _ => None,
        }
    }
}
impl From<StorageError> for WorldStoreError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl OfficeStore {
    pub fn show_local_world(&mut self) -> Result<LocalWorldSnapshot, WorldStoreError> {
        let (connection, references) = self.split()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
            .map_err(|error| classify(error, "Observe Office world"))?;
        let snapshot = read(&transaction, references)?;
        transaction
            .commit()
            .map_err(|error| classify(error, "Finish Office world observation"))?;
        Ok(snapshot)
    }

    pub fn apply_local_world(
        &mut self,
        expected_revision: u64,
        legacy_basis: Option<&str>,
        proposed: &WorldLayout,
        now_ms: u64,
    ) -> Result<LocalWorldSnapshot, WorldStoreError> {
        if now_ms == 0 || now_ms > MAX_JS_SAFE_INTEGER || expected_revision > MAX_JS_SAFE_INTEGER {
            return Err(WorldStoreError::InvalidInput);
        }
        let encoded = serde_json::to_string(&world_value(proposed)).expect("finite world document");
        if encoded.len() > WORLD_DOCUMENT_LIMIT {
            return Err(WorldStoreError::InvalidInput);
        }
        let bindings = Bindings::preflight(self.references(), proposed)?;
        preflight_complete();
        with_immediate_transaction_and_references(
            self,
            "Office world",
            |transaction, references| {
                let current = read(transaction, references)?;
                if current.revision != expected_revision
                    || current.legacy_basis.as_deref() != legacy_basis
                {
                    return Err(WorldStoreError::RevisionConflict);
                }
                bindings.validate(&current.layout, proposed)?;
                validate_props(transaction, &current.layout, proposed)?;
                if current.revision > 0 && world_value(&current.layout) == world_value(proposed) {
                    return Ok(current);
                }
                if current.revision == MAX_JS_SAFE_INTEGER {
                    return Err(WorldStoreError::RevisionExhausted);
                }
                let id = ensure_world(transaction, || Ok(now_ms as i64))?;
                let revision = current.revision + 1;
                transaction.execute(
                "UPDATE office_local_worlds SET layout_revision=?,layout_json=?,layout_updated_at_ms=? WHERE singleton=1 AND layout_revision=?",
                params![revision as i64,encoded,now_ms as i64,current.revision as i64],
            ).map_err(|error| classify(error, "Save complete Office world"))?;
                // The same transaction retires the superseded rows only after the
                // complete candidate passed validation. Resource content tables are untouched.
                if current.revision == 0 {
                    transaction
                        .execute("DELETE FROM office_local_blocks", [])
                        .map_err(|error| classify(error, "Retire migrated Office block rows"))?;
                }
                Ok(LocalWorldSnapshot {
                    world_id: Some(id),
                    revision,
                    legacy_basis: None,
                    layout: proposed.clone(),
                    updated_at_ms: now_ms,
                    changed: true,
                })
            },
        )
    }
}

fn read(
    connection: &Connection,
    references: &dyn CoreReferences,
) -> Result<LocalWorldSnapshot, WorldStoreError> {
    let row = connection.query_row(
        "SELECT id,layout_revision,layout_json,layout_updated_at_ms FROM office_local_worlds WHERE singleton=1", [],
        |row| Ok((row.get::<_, String>(0)?,row.get::<_, i64>(1)?,row.get::<_, Option<String>>(2)?,row.get::<_, i64>(3)?)),
    ).optional().map_err(|error| classify(error, "Read Office world snapshot"))?;
    let (id, revision, encoded, timestamp) = row
        .map(|(id, revision, encoded, timestamp)| (Some(id), revision, encoded, timestamp))
        .unwrap_or((None, 0, None, 0));
    if revision == 0 && encoded.is_none() && timestamp == 0 {
        let (layout, basis) = legacy::project(connection, references, id.as_deref())?;
        return Ok(LocalWorldSnapshot {
            world_id: id,
            revision: 0,
            legacy_basis: Some(basis),
            layout,
            updated_at_ms: 0,
            changed: false,
        });
    }
    if revision <= 0
        || revision as u64 > MAX_JS_SAFE_INTEGER
        || timestamp <= 0
        || timestamp as u64 > MAX_JS_SAFE_INTEGER
    {
        return Err(WorldStoreError::StoredInvalid);
    }
    let layout = decode_world(encoded.ok_or(WorldStoreError::StoredInvalid)?.as_bytes())
        .map_err(|_| WorldStoreError::StoredInvalid)?;
    Ok(LocalWorldSnapshot {
        world_id: id,
        revision: revision as u64,
        legacy_basis: None,
        layout,
        updated_at_ms: timestamp as u64,
        changed: false,
    })
}

/// Preflighted core references named by a proposed layout's area bindings.
struct Bindings {
    identities: HashMap<String, Option<CoreIdentity>>,
    rooms: HashMap<String, Option<CoreRoom>>,
}

impl Bindings {
    fn preflight(
        references: &dyn CoreReferences,
        proposed: &WorldLayout,
    ) -> Result<Self, StorageError> {
        let mut bindings = Self {
            identities: HashMap::new(),
            rooms: HashMap::new(),
        };
        for area in &proposed.map().draft().areas {
            match &area.kind {
                AreaKind::Personal {
                    identity_id: Some(id),
                } if !bindings.identities.contains_key(id) => {
                    bindings
                        .identities
                        .insert(id.clone(), references.identity(id)?);
                }
                AreaKind::Meeting { room_id } if !bindings.rooms.contains_key(room_id) => {
                    bindings
                        .rooms
                        .insert(room_id.clone(), references.room(room_id)?);
                }
                _ => {}
            }
        }
        Ok(bindings)
    }

    fn validate(
        &self,
        current: &WorldLayout,
        proposed: &WorldLayout,
    ) -> Result<(), WorldStoreError> {
        for area in &proposed.map().draft().areas {
            match &area.kind {
                AreaKind::Personal {
                    identity_id: Some(id),
                } => {
                    let identity = self.identities.get(id).and_then(Option::as_ref);
                    let saved =
                        identity.is_some_and(|identity| identity.lifetime == Lifetime::Saved);
                    let active = identity.is_some_and(|identity| !identity.retired);
                    if active && saved {
                        continue;
                    }
                    let retained = current
                        .map()
                        .draft()
                        .areas
                        .iter()
                        .any(|old| old.id == area.id && old.kind == area.kind);
                    // Retirement suppresses the avatar, not retained layout content.
                    // A retained saved UUID cannot grant a new assignment elsewhere.
                    if retained && !active && saved {
                        continue;
                    }
                    return Err(WorldStoreError::IdentityIneligible);
                }
                AreaKind::Meeting { room_id } => {
                    let room = self.rooms.get(room_id).and_then(Option::as_ref);
                    if room.is_some_and(|room| !room.retired) {
                        continue;
                    }
                    if current
                        .map()
                        .draft()
                        .areas
                        .iter()
                        .any(|old| old.id == area.id && old.kind == area.kind)
                        && room.is_some_and(|room| room.retired)
                    {
                        continue;
                    }
                    return Err(WorldStoreError::RoomMissing);
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn validate_props(
    connection: &Connection,
    current: &WorldLayout,
    proposed: &WorldLayout,
) -> Result<(), WorldStoreError> {
    let mut resolver = LocalPropResolver::new(connection);
    let previous: std::collections::HashMap<_, _> = current
        .objects()
        .iter()
        .map(|object| (&object.id, object))
        .collect();
    for object in proposed.objects() {
        if let Some(attachment) = &object.extension {
            validate_attachment(attachment).map_err(WorldStoreError::Extension)?;
        }
        if matches!(
            resolve_item(&object.placement, &mut resolver).map_err(WorldStoreError::Prop)?,
            ItemResolution::Available(_)
        ) {
            continue;
        }
        if previous.get(&object.id).is_some_and(|old| {
            let a = &old.placement;
            let b = &object.placement;
            old.kind == object.kind
                && a.prop == b.prop
                && a.footprint_width == b.footprint_width
                && a.footprint_height == b.footprint_height
                && a.customization == b.customization
        }) {
            continue;
        }
        return Err(WorldStoreError::PropUnavailable);
    }
    Ok(())
}
