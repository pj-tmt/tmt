//! Full, idempotent reconciliation of Office's stored references against core.
//!
//! Every identity and room UUID Office stores is looked up through the core
//! reference port; the ones core confirms retired are recorded in the
//! monotonic markers of `office.db`, which Office transactions writing a
//! reference then check. Nothing is erased: an unknown UUID is not retired,
//! and a lookup error stops the run before anything is written, so failure
//! means "retry", never "deleted". The shared core file before the switch has
//! no markers, and core stays authoritative there.

use crate::OfficeStore;
use rusqlite::Connection;
use std::{collections::BTreeSet, time::Instant};
use tmt_adapters::storage::{StorageError, classify};
use tmt_office_model::{codec::office_world::decode_world, office_map::AreaKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reconciliation {
    pub identities: usize,
    pub rooms: usize,
    pub newly_marked: usize,
    pub elapsed_ms: u64,
}

impl OfficeStore {
    pub fn reconcile(&mut self, now_ms: i64) -> Result<Reconciliation, StorageError> {
        let started = Instant::now();
        if !self.is_office_database() {
            return Ok(Reconciliation::default());
        }
        let (identities, rooms) = references(self.connection()?)?;
        let mut retired_identities = Vec::new();
        for id in &identities {
            if self
                .references()
                .identity(id)?
                .is_some_and(|identity| identity.retired)
            {
                retired_identities.push(id);
            }
        }
        let mut retired_rooms = Vec::new();
        for id in &rooms {
            if self.references().room(id)?.is_some_and(|room| room.retired) {
                retired_rooms.push(id);
            }
        }
        let newly_marked = crate::store::with_immediate_transaction(
            self,
            "Office reconciliation",
            |transaction| -> Result<usize, StorageError> {
                let mut marked = 0;
                for id in &retired_identities {
                    marked += transaction
                        .execute(
                            "INSERT INTO office_retired_identities (identity_id, retired_at_ms, recorded_at_ms) VALUES (?, ?, ?) ON CONFLICT (identity_id) DO NOTHING",
                            rusqlite::params![id, now_ms, now_ms],
                        )
                        .map_err(|error| classify(error, "Record retired identity"))?;
                }
                for id in &retired_rooms {
                    marked += transaction
                        .execute(
                            "INSERT INTO office_retired_rooms (room_id, recorded_at_ms) VALUES (?, ?) ON CONFLICT (room_id) DO NOTHING",
                            rusqlite::params![id, now_ms],
                        )
                        .map_err(|error| classify(error, "Record retired room"))?;
                }
                Ok(marked)
            },
        )?;
        Ok(Reconciliation {
            identities: identities.len(),
            rooms: rooms.len(),
            newly_marked,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}

/// Every identity and room UUID stored in Office rows, deduplicated.
fn references(
    connection: &Connection,
) -> Result<(BTreeSet<String>, BTreeSet<String>), StorageError> {
    let read = |sql: &str| -> Result<Vec<String>, StorageError> {
        connection
            .prepare(sql)
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()
            })
            .map_err(|error| classify(error, "Read Office references"))
    };
    let mut identities: BTreeSet<String> = BTreeSet::new();
    identities.extend(read("SELECT identity_id FROM office_local_profiles")?);
    identities.extend(read(
        "SELECT identity_id FROM office_local_blocks WHERE identity_id IS NOT NULL",
    )?);
    identities.extend(read(
        "SELECT DISTINCT author_id FROM office_board_entries WHERE author_kind = 'identity'",
    )?);
    let mut rooms: BTreeSet<String> =
        read("SELECT DISTINCT category_id FROM office_board_entries WHERE category_kind = 'room'")?
            .into_iter()
            .collect();
    for layout in read("SELECT layout_json FROM office_local_worlds WHERE layout_json IS NOT NULL")?
    {
        // A stored layout that no longer decodes is reported by its own reads.
        let Ok(world) = decode_world(layout.as_bytes()) else {
            continue;
        };
        for area in &world.map().draft().areas {
            match &area.kind {
                AreaKind::Personal {
                    identity_id: Some(id),
                } => {
                    identities.insert(id.clone());
                }
                AreaKind::Meeting { room_id } => {
                    rooms.insert(room_id.clone());
                }
                _ => {}
            }
        }
    }
    Ok((identities, rooms))
}
