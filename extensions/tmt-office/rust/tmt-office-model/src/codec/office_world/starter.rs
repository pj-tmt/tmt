//! Furnished new-world composition; retained layouts never pass through this preset.

use super::{lobby_objects, placement_id};
use crate::codec::office_prop::{
    MODULAR_FACILITIES_DIGEST, MODULAR_LOUNGE_DIGEST, MODULAR_RECEPTION_DIGEST,
    MODULAR_WORKSTATION_DIGEST, STUDY_DIGEST, WORKSHOP_DIGEST, builtin_packs,
};
use crate::office_block::PropPlacement;
use crate::office_map::Area;
use crate::office_map::AreaKind;
use crate::office_map::OfficeMap;
use crate::office_map::modules::Material;
use crate::office_map::modules::Module;
use crate::office_map::modules::ModuleDraft;
use crate::office_map::modules::ModuleLayout;
use crate::office_map::modules::Slot;
use crate::office_world::ObjectKind;
use crate::office_world::Surface;
use crate::office_world::WorldLayout;
use crate::office_world::WorldObject;

/// The catalog, not the composition, owns artwork dimensions and identity.
fn appearance(digest: &str, key: &str, x: i32, y: i32) -> PropPlacement {
    let definition = builtin_packs()
        .iter()
        .find(|pack| pack.digest() == digest)
        .and_then(|pack| pack.prop(key))
        .expect("starter art is an admitted bundled prop");
    PropPlacement {
        prop: format!("{digest}/{key}"),
        footprint_width: definition.footprint.width,
        footprint_height: definition.footprint.height,
        x,
        y,
        rotation: 0,
        customization: None,
    }
}

fn prop(seed: &str, ordinal: usize, digest: &str, key: &str, x: i32, y: i32) -> WorldObject {
    WorldObject {
        id: placement_id(seed, ordinal as u64),
        placement: appearance(digest, key, x, y),
        kind: ObjectKind::Decoration,
        surface: Surface::Floor { base: None },
        extension: None,
    }
}

pub fn new_world(seed: &str) -> WorldLayout {
    let lobby_id = placement_id(seed, u64::MAX);
    let mut modules = vec![Module {
        area: Area {
            id: lobby_id.clone(),
            name: "Lobby".into(),
            kind: AreaKind::Lobby,
        },
        slot: Slot::Lobby,
        material: Material::Workshop,
    }];
    for (index, (column, row)) in [(0, -1), (1, -1), (0, 2), (1, 2)].into_iter().enumerate() {
        modules.push(Module {
            area: Area {
                id: placement_id(&format!("{seed}:office"), index as u64),
                name: format!("Office {:02}", index + 1),
                kind: AreaKind::Personal { identity_id: None },
            },
            slot: Slot::Office { column, row },
            material: Material::Workshop,
        });
    }
    let map = OfficeMap::from_modules(ModuleDraft {
        primary_lobby_id: lobby_id.clone(),
        modules,
        layout: ModuleLayout::UnifiedAreas,
    })
    .expect("starter modules have connected cardinal circulation");

    // Reuse canonical host bindings. Reading the preset creates no documents,
    // identities, room memberships or message resources.
    let mut objects = lobby_objects(&lobby_id);
    for object in &mut objects {
        let (key, x, y) = match object
            .extension
            .as_ref()
            .expect("bundled binding")
            .definition
            .as_str()
        {
            "tmt-discussion-board" => ("lobby-discussion-board", 64, 18),
            "tmt-whiteboard" => ("lobby-whiteboard", 82, 18),
            "tmt-broadcaster" => ("lobby-radio", 76, 51),
            _ => unreachable!("starter uses the three canonical Lobby facilities"),
        };
        object.placement = appearance(MODULAR_FACILITIES_DIGEST, key, x, y);
    }

    // Four furnished pockets flank the open x=48..56 / y=40..48 cross-axis.
    // Rugs precede furniture so the stored painter order remains intentional.
    for (key, x, y) in [
        ("woven-rug", 17, 27),
        ("woven-rug", 26, 58),
        ("woven-rug", 72, 62),
    ] {
        objects.push(prop(seed, objects.len(), WORKSHOP_DIGEST, key, x, y));
    }
    // A broad welcoming lounge and a smaller reading nook, not four repeated
    // desks. These are ordinary movable props, not a flattened background.
    for (key, x, y) in [
        ("lounge-sofa", 8, 8),
        ("lounge-sofa", 24, 8),
        ("lounge-plant", 3, 6),
        ("lounge-plant", 93, 6),
        ("lounge-plant", 93, 58),
        ("lounge-plant", 39, 6),
        ("lounge-plant", 58, 6),
    ] {
        objects.push(prop(seed, objects.len(), MODULAR_LOUNGE_DIGEST, key, x, y));
    }
    for (key, x, y) in [
        ("reception-table", 16, 24),
        ("reception-armchair", 4, 26),
        ("reception-armchair", 34, 26),
        ("reception-armchair", 28, 50),
        ("reception-table", 72, 54),
        ("reception-armchair", 58, 58),
    ] {
        objects.push(prop(
            seed,
            objects.len(),
            MODULAR_RECEPTION_DIGEST,
            key,
            x,
            y,
        ));
    }
    for (key, x, y) in [
        ("oak-bookcase", 4, 50),
        ("reading-lamp", 18, 52),
        ("oak-bookcase", 88, 52),
    ] {
        objects.push(prop(seed, objects.len(), STUDY_DIGEST, key, x, y));
    }
    // Identical furniture does not create identities or assign residents.
    let source = map.modules().expect("starter uses modules");
    for module in source
        .modules
        .iter()
        .filter(|module| matches!(module.slot, Slot::Office { .. }))
    {
        let bounds = module
            .bounds(source.layout)
            .expect("admitted office bounds");
        let (x, y) = (bounds.x, bounds.y);
        // A centered workstation forms a furnished island on the open slab.
        // Keep both bridge approaches clear; the rug paints below the desk.
        objects.push(prop(
            seed,
            objects.len(),
            WORKSHOP_DIGEST,
            "woven-rug",
            x + 12,
            y + 18,
        ));
        for (key, dx, dy) in [
            ("workstation-chair", 16, 10),
            ("workstation-desk", 12, 14),
            ("workstation-terminal", 16, 16),
            ("workstation-bookcase", 34, 8),
        ] {
            objects.push(prop(
                seed,
                objects.len(),
                MODULAR_WORKSTATION_DIGEST,
                key,
                x + dx,
                y + dy,
            ));
        }
        objects.push(prop(
            seed,
            objects.len(),
            WORKSHOP_DIGEST,
            "leafy-plant",
            x + 4,
            y + 6,
        ));
    }
    // Functional objects sit on top of their supporting furniture. Preserve
    // relative order within each group; this is the normal editable prop stack.
    objects.sort_by_key(|object| object.extension.is_some());
    WorldLayout::new(map, objects).expect("starter placements fit their platforms")
}
