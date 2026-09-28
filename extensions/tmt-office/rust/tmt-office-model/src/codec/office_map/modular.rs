//! Versioned module projections, never a competing editable floor/door graph.

use super::{AreaDocument, MapCodecError};
use crate::json_integer::whole;
use crate::office_map::{
    OfficeMap,
    modules::{Material, Module, ModuleDraft, ModuleLayout, Slot},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ModuleDocument {
    #[serde(deserialize_with = "whole")]
    version: u32,
    primary_lobby_id: String,
    modules: Vec<ModuleEntry>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ModuleEntry {
    area: AreaDocument,
    slot: ModuleSlot,
    material: ModuleMaterial,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum ModuleSlot {
    Lobby {},
    Office {
        #[serde(deserialize_with = "whole")]
        column: i32,
        #[serde(deserialize_with = "whole")]
        row: i32,
    },
    Meeting {
        #[serde(deserialize_with = "whole")]
        index: u32,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum ModuleMaterial {
    Workshop,
    Moonlight,
    Copper,
}

pub(super) fn admit(document: ModuleDocument) -> Result<OfficeMap, MapCodecError> {
    let layout = match document.version {
        2 => ModuleLayout::ShortLinks,
        3 => ModuleLayout::Grid,
        4 => ModuleLayout::CentralGrid,
        5 => ModuleLayout::CompactGrid,
        6 => ModuleLayout::Skybridges,
        7 => ModuleLayout::IndependentMeetings,
        8 => ModuleLayout::UnifiedAreas,
        _ => return Err(MapCodecError::UnsupportedVersion),
    };
    OfficeMap::from_modules(ModuleDraft {
        layout,
        primary_lobby_id: document.primary_lobby_id,
        modules: document
            .modules
            .into_iter()
            .map(|module| Module {
                area: module.area.into_area(),
                slot: match module.slot {
                    ModuleSlot::Lobby {} => Slot::Lobby,
                    ModuleSlot::Office { column, row } => Slot::Office { column, row },
                    ModuleSlot::Meeting { index } => Slot::Meeting { index },
                },
                material: match module.material {
                    ModuleMaterial::Workshop => Material::Workshop,
                    ModuleMaterial::Moonlight => Material::Moonlight,
                    ModuleMaterial::Copper => Material::Copper,
                },
            })
            .collect(),
    })
    .map_err(MapCodecError::Geometry)
}

pub(super) fn value(draft: &ModuleDraft) -> Value {
    serde_json::to_value(ModuleDocument {
        version: match draft.layout {
            ModuleLayout::ShortLinks => 2,
            ModuleLayout::Grid => 3,
            ModuleLayout::CentralGrid => 4,
            ModuleLayout::CompactGrid => 5,
            ModuleLayout::Skybridges => 6,
            ModuleLayout::IndependentMeetings => 7,
            ModuleLayout::UnifiedAreas => 8,
        },
        primary_lobby_id: draft.primary_lobby_id.clone(),
        modules: draft
            .modules
            .iter()
            .map(|module| ModuleEntry {
                area: AreaDocument::from_area(&module.area),
                slot: match module.slot {
                    Slot::Lobby => ModuleSlot::Lobby {},
                    Slot::Office { column, row } => ModuleSlot::Office { column, row },
                    Slot::Meeting { index } => ModuleSlot::Meeting { index },
                },
                material: match module.material {
                    Material::Workshop => ModuleMaterial::Workshop,
                    Material::Moonlight => ModuleMaterial::Moonlight,
                    Material::Copper => ModuleMaterial::Copper,
                },
            })
            .collect(),
    })
    .expect("admitted module plans contain only finite integer/string fields")
}

#[cfg(test)]
mod tests;
