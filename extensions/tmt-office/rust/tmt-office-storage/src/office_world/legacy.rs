//! One-time projection of retained blocks; identities without stored blocks create no rooms.

use super::layout::WorldStoreError;
use crate::core_references::CoreReferences;
use rusqlite::Connection;
use serde_json::{Value, json};
use tmt_adapters::storage::classify;
use tmt_core::content_digest::framed_sha256;
use tmt_office_model::codec::office_block::default_local_layout;
use tmt_office_model::codec::office_world::lobby_objects;
use tmt_office_model::codec::office_world::new_world;
use tmt_office_model::codec::office_world::placement_id;
use tmt_office_model::codec::office_world::world_value;
use tmt_office_model::office_block::LocalBlockTarget;
use tmt_office_model::office_map::Area;
use tmt_office_model::office_map::AreaKind;
use tmt_office_model::office_map::Axis;
use tmt_office_model::office_map::Edge;
use tmt_office_model::office_map::FloorSpan;
use tmt_office_model::office_map::MapDraft;
use tmt_office_model::office_map::OfficeMap;
use tmt_office_model::office_world::ObjectKind;
use tmt_office_model::office_world::Surface;
use tmt_office_model::office_world::WorldLayout;
use tmt_office_model::office_world::WorldObject;

struct Block {
    id: String,
    lobby: bool,
    identity: Option<String>,
    eligible: bool,
    layout: tmt_office_model::office_block::LocalBlockLayout,
}

pub(super) fn project(
    connection: &Connection,
    references: &dyn CoreReferences,
    world_id: Option<&str>,
) -> Result<(WorldLayout, String), WorldStoreError> {
    let mut query = connection
        .prepare(
            "SELECT block_id,target_kind,identity_id,revision,layout,updated_at_ms \
         FROM office_local_blocks ORDER BY block_id LIMIT ?",
        )
        .map_err(|error| classify(error, "Read retained Office layouts"))?;
    let rows = query
        .query_map(
            [tmt_office_model::office_map::MAX_AREAS as i64 + 1],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .map_err(|error| classify(error, "Query retained Office layouts"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| classify(error, "Decode retained Office layouts"))?;
    if rows.len() > tmt_office_model::office_map::MAX_AREAS {
        return Err(WorldStoreError::MigrationInvalid);
    }
    if rows.is_empty() {
        let layout = new_world(world_id.unwrap_or("uninitialized-local-world"));
        let encoded = serde_json::to_vec(&json!([world_id, [], world_value(&layout)]))
            .expect("finite new-world projection");
        let basis = framed_sha256(b"tmt-office-legacy-basis-v1", &encoded);
        return Ok((layout, basis));
    }
    let mut source = Vec::<Value>::new();
    let mut blocks = Vec::new();
    for (id, target, identity, revision, encoded, timestamp) in rows {
        // Core owns identity lifetime and retirement; the fence digest carries the
        // retired state rather than its timestamp, which Office does not read.
        let (lifetime, retired) = match identity.as_deref() {
            Some(identity_id) => references
                .identity(identity_id)?
                .map_or((None, None), |identity| {
                    (Some(identity.lifetime.as_str()), Some(identity.retired))
                }),
            None => (None, None),
        };
        let lobby = target == "lobby";
        if !(lobby && identity.is_none()
            || target == "identity"
                && identity.is_some()
                && matches!(lifetime, Some("saved" | "temporary")))
        {
            return Err(WorldStoreError::StoredInvalid);
        }
        source.push(json!([
            id, target, identity, revision, encoded, timestamp, lifetime, retired
        ]));
        let layout = crate::office_local::decode_layout(&encoded)
            .map_err(|_| WorldStoreError::StoredInvalid)?;
        blocks.push(Block {
            id,
            lobby,
            identity,
            eligible: lifetime == Some("saved") && retired == Some(false),
            layout,
        });
    }
    if !blocks.iter().any(|block| block.lobby) {
        blocks.push(Block {
            id: placement_id(world_id.unwrap_or("uninitialized-local-world"), u64::MAX),
            lobby: true,
            identity: None,
            eligible: false,
            layout: default_local_layout(&LocalBlockTarget::Lobby),
        });
    }
    blocks.sort_by(|a, b| b.lobby.cmp(&a.lobby).then(a.id.cmp(&b.id)));
    let layout = build(&blocks)?;
    let encoded = serde_json::to_vec(&json!([world_id, source, world_value(&layout)]))
        .expect("finite migration snapshot");
    let basis = framed_sha256(b"tmt-office-legacy-basis-v1", &encoded);
    Ok((layout, basis))
}

fn build(blocks: &[Block]) -> Result<WorldLayout, WorldStoreError> {
    if blocks.len() > tmt_office_model::office_map::MAX_AREAS
        || blocks.iter().filter(|block| block.lobby).count() != 1
    {
        return Err(WorldStoreError::MigrationInvalid);
    }
    let lobby_id = blocks[0].id.clone();
    let mut draft = MapDraft {
        primary_lobby_id: lobby_id,
        areas: vec![],
        floor: vec![],
        doors: vec![],
    };
    let mut objects = lobby_objects(&blocks[0].id);
    let mut slot = 0_i32;
    let mut row_widths = std::collections::BTreeMap::<i32, i32>::new();
    for block in blocks {
        let x = (slot % 6) * 36;
        let y = (slot / 6) * 40;
        let width = if block.lobby { 72 } else { 36 };
        row_widths
            .entry(y)
            .and_modify(|end| *end = (*end).max(x + width))
            .or_insert(x + width);
        draft.areas.push(Area {
            id: block.id.clone(),
            name: if block.lobby {
                "Lobby"
            } else if block.eligible {
                "Workspace"
            } else {
                "Retained workspace"
            }
            .into(),
            kind: if block.lobby {
                AreaKind::Lobby
            } else {
                AreaKind::Personal {
                    identity_id: block.identity.clone().filter(|_| block.eligible),
                }
            },
        });
        for row in y..y + 36 {
            draft.floor.push(FloorSpan {
                y: row,
                start: x,
                end: x + width,
                area_id: Some(block.id.clone()),
            });
        }
        draft.doors.push(Edge {
            x: x + width / 2,
            y: y + 36,
            axis: Axis::Horizontal,
        });
        for (index, prop) in block.layout.objects().iter().enumerate() {
            let mut placement = prop.clone();
            placement.x += x + if block.lobby { 38 } else { 2 };
            placement.y += y + 2;
            objects.push(WorldObject {
                id: placement_id(&block.id, index as u64),
                placement,
                surface: Surface::Floor { base: None },
                kind: ObjectKind::Decoration,
                extension: None,
            });
        }
        slot += if block.lobby { 2 } else { 1 };
    }
    let height = row_widths.last_key_value().expect("Lobby row").0 + 40;
    // A four-tile spine joins row corridors; margins keep every legacy edge prop
    // away from new door openings without moving it relative to its neighbors.
    for y in 0..height {
        draft.floor.push(FloorSpan {
            y,
            start: -4,
            end: 0,
            area_id: None,
        });
    }
    for (y, width) in row_widths {
        for row in y + 36..y + 40 {
            draft.floor.push(FloorSpan {
                y: row,
                start: 0,
                end: width,
                area_id: None,
            });
        }
    }
    let map = OfficeMap::new(draft).map_err(|_| WorldStoreError::MigrationInvalid)?;
    WorldLayout::new(map, objects).map_err(|_| WorldStoreError::MigrationInvalid)
}
