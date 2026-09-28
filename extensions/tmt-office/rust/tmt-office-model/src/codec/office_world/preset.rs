//! A read-only starting composition. Saving and later reading never reseeds it.

use crate::codec::office_extension::bundled_definitions;
use crate::codec::office_extension::bundled_instances;
use crate::office_extension::ExtensionAttachment;
use crate::office_world::ObjectKind;
use crate::office_world::Surface;
use crate::office_world::WorldObject;
use tmt_core::content_digest::framed_sha256;

// Domain-framed UUIDv8 keeps preview IDs stable without materializing resources.
pub fn placement_id(seed: &str, ordinal: u64) -> String {
    let input = serde_json::to_vec(&(seed, ordinal)).expect("finite placement key");
    let hash = framed_sha256(b"tmt-office-placement-v1", &input);
    let mut bytes = *uuid::Uuid::parse_str(&hash[..32])
        .expect("SHA-256 hex prefix")
        .as_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

/// The former bundled functional entries become ordinary editable world objects.
pub fn lobby_objects(area_id: &str) -> Vec<WorldObject> {
    bundled_instances()
        .iter()
        .map(|instance| {
            let definition = bundled_definitions()
                .iter()
                .find(|definition| definition.id == instance.definition)
                .expect("bundled instance has a bundled definition");
            let mut placement =
                definition
                    .appearance
                    .placement(instance.x, instance.y, instance.rotation);
            // The left half of the migration Lobby preserves the old commons composition.
            placement.x += 2;
            placement.y += 2;
            WorldObject {
                id: placement_id(&format!("{area_id}:{}", instance.id), 0),
                placement,
                surface: Surface::Floor { base: None },
                kind: ObjectKind::Decoration,
                extension: Some(ExtensionAttachment {
                    definition: instance.definition.clone(),
                    binding: instance.binding.clone().into(),
                }),
            }
        })
        .collect()
}
