//! Data-only customization capabilities declared by directional artwork.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::{PropDefinition, present};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropCapabilities {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub tint: Option<TintCapability>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub text: Option<TextCapability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TintCapability {
    pub indices: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextCapability {
    pub regions: [TextRegion; 4],
    pub color: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextRegion {
    pub x: u8,
    pub y: u8,
    pub width: u8,
    pub height: u8,
}

/// Called only after the four indexed rasters have passed ordinary admission.
pub(super) fn valid(prop: &PropDefinition, palette_len: usize) -> bool {
    let Some(customization) = &prop.customization else {
        return true;
    };
    let Some(frames) = &prop.frames else {
        return false;
    };
    if customization.tint.is_none() && customization.text.is_none() {
        return false;
    }
    if let Some(tint) = &customization.tint {
        let indices: HashSet<u8> = tint.indices.iter().copied().collect();
        if indices.is_empty()
            || indices.len() != tint.indices.len()
            || indices
                .iter()
                .any(|index| *index == 0 || usize::from(*index) >= palette_len)
        {
            return false;
        }
        let mut used = HashSet::new();
        for frame in frames {
            let mut frame_has_tint = false;
            for row in frame {
                for pixel in row.as_bytes().chunks_exact(2) {
                    let value = u8::from_str_radix(
                        std::str::from_utf8(pixel).expect("admitted ASCII raster"),
                        16,
                    )
                    .expect("admitted hexadecimal index");
                    if indices.contains(&value) {
                        used.insert(value);
                        frame_has_tint = true;
                    }
                }
            }
            if !frame_has_tint {
                return false;
            }
        }
        if used != indices {
            return false;
        }
    }
    if let Some(text) = &customization.text {
        let color = text.color.as_bytes();
        if color.len() != 7
            || color[0] != b'#'
            || !color[1..]
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return false;
        }
        for (region, frame) in text.regions.iter().zip(frames) {
            if region.width == 0
                || region.height == 0
                || usize::from(region.x) + usize::from(region.width) > frame[0].len() / 2
                || usize::from(region.y) + usize::from(region.height) > frame.len()
            {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::super::{pack_projection, validate_pack};
    use serde_json::{Value, json};

    fn vectors() -> Value {
        serde_json::from_str(include_str!(
            "../../../../../contracts/prop-customization-vectors.json"
        ))
        .unwrap()
    }

    #[test]
    fn customization_admission_matches_shared_vectors_and_summary_preserves_capabilities() {
        let vectors = vectors();
        for case in vectors["cases"].as_array().unwrap() {
            let mut pack = vectors["pack"].clone();
            pack["props"][0]["customization"] = case["value"].clone();
            let admitted = validate_pack(&serde_json::to_vec(&pack).unwrap());
            assert_eq!(
                admitted.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            if let Ok(pack) = admitted {
                let fields = ["tint", "text"]
                    .into_iter()
                    .filter(|field| case["value"].get(field).is_some())
                    .collect::<Vec<_>>();
                assert_eq!(
                    pack_projection(&pack)["props"][0]["customizable"],
                    json!(fields)
                );
            }
        }
    }

    #[test]
    fn tint_requires_a_visible_channel_in_every_frame_and_never_changes_v1() {
        let mut pack = vectors()["pack"].clone();
        pack["props"][0]["customization"] = json!({"tint":{"indices":[1]}});
        pack["props"][0]["frames"][3] = json!(["0202", "0202", "0202", "0000"]);
        assert!(validate_pack(&serde_json::to_vec(&pack).unwrap()).is_err());
        pack["formatVersion"] = json!(1);
        pack["props"][0].as_object_mut().unwrap().remove("frames");
        pack["props"][0]["pixels"] = json!(["1"]);
        assert!(validate_pack(&serde_json::to_vec(&pack).unwrap()).is_err());
    }
}
