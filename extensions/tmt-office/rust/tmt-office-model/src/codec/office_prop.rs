//! Strict data-only Office prop-pack validation and immutable references.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::OnceLock};

mod customization;
mod quality;
pub use customization::{PropCapabilities, TextCapability, TextRegion, TintCapability};
pub use quality::{QualityWarning, quality_warnings};

pub const V1_PACK_INPUT_LIMIT: usize = 128 * 1024;
pub const PACK_INPUT_LIMIT: usize = 512 * 1024;
pub const ENCODED_INPUT_LIMIT: usize = PACK_INPUT_LIMIT.div_ceil(3) * 4;
pub const PROTOCOL_INPUT_LIMIT: usize = ENCODED_INPUT_LIMIT + 1024;
pub const PACK_PROP_LIMIT: usize = 16;
pub const PACK_PIXEL_LIMIT: usize = 65_536;
pub const PROP_PIXEL_LIMIT: usize = 4_096;
pub const PROP_LABEL_LIMIT: usize = crate::indexed_art::LABEL_LIMIT;
/// Version byte + catalog revision + digest, encoded as unpadded base64url.
pub const CATALOG_CURSOR_MAX_BYTES: usize = crate::codec::catalog_cursor::ENCODED_MAX_BYTES;
pub const RASTER_LIMIT: usize = 64;
pub const PALETTE_LIMIT: usize = crate::indexed_art::PALETTE_LIMIT;
pub const FOOTPRINT_LIMIT: u8 = 8;
pub const BUILTIN_DIGEST: &str = crate::office_block::BUILTIN_PROP_PACK_DIGEST;
pub const BUILTIN_BYTES: &[u8] =
    include_bytes!("../../../../contracts/builtin-props-v1.tmtprop.json");
pub const WORKSHOP_DIGEST: &str =
    "sha256:288fb4f9ef08db8bdf635fbd1b16a3d602fbe0d53a095d96a7988969dabe3529";
pub const WORKSHOP_BYTES: &[u8] =
    include_bytes!("../../../../contracts/workshop-furniture-v2.tmtprop.json");
pub const COMMONS_DIGEST: &str =
    "sha256:39a02590febbe0b7e9175951db1d7908a9b66ef32aa37eeb1ed9aa5cd524f63c";
pub const COMMONS_BYTES: &[u8] =
    include_bytes!("../../../../contracts/commons-props-v2.tmtprop.json");
pub const WHITEBOARD_DIGEST: &str =
    "sha256:2514687c911f644e28ea816e2c28b611d2c074e0105e584e6bba86ca208797e8";
pub const WHITEBOARD_BYTES: &[u8] =
    include_bytes!("../../../../contracts/whiteboard-props-v2.tmtprop.json");
pub const BROADCASTER_DIGEST: &str =
    "sha256:00f2f262077a0486fb3f4524a4efb6125c64a4349faaada079b13f6b25067a04";
pub const BROADCASTER_BYTES: &[u8] =
    include_bytes!("../../../../contracts/broadcaster-props-v2.tmtprop.json");
pub const STUDY_DIGEST: &str =
    "sha256:78f0c0dc0700aaa37c55ae8cbe91c2d96585555a06e093a9131b529791360eed";
pub const STUDY_BYTES: &[u8] =
    include_bytes!("../../../../contracts/study-furniture-v2.tmtprop.json");
pub const WALL_DIGEST: &str =
    "sha256:5303fe9a3e5bf8a22c9958faeef1922a3cc21cfefb95a7b701a6a86213ac4415";
pub const WALL_BYTES: &[u8] = include_bytes!("../../../../contracts/wall-props-v2.tmtprop.json");
pub const MODULAR_WORKSTATION_DIGEST: &str =
    "sha256:10dc14a38d1cb0c92148c084b5e6239a54ee401348070444ae65fe8f6d815755";
pub const MODULAR_WORKSTATION_BYTES: &[u8] =
    include_bytes!("../../../../contracts/modular-workstation-v2.tmtprop.json");
pub const MODULAR_MOUNTED_DIGEST: &str =
    "sha256:86e7784ccb08d6c8804de7deeb2e3063898d3804e6ed81e3f7c8735b1996c7eb";
pub const MODULAR_MOUNTED_BYTES: &[u8] =
    include_bytes!("../../../../contracts/modular-mounted-v2.tmtprop.json");
pub const MODULAR_LOUNGE_DIGEST: &str =
    "sha256:a4538f15b7da963679094f89d6f954215453492b5eb23bde40a4ffc6969a64b2";
pub const MODULAR_LOUNGE_BYTES: &[u8] =
    include_bytes!("../../../../contracts/modular-lounge-v2.tmtprop.json");
pub const MODULAR_FACILITIES_DIGEST: &str =
    "sha256:b400ccadbacad373c9f420845a820256840829de7856f587cd5fd7d78786a55c";
pub const MODULAR_FACILITIES_BYTES: &[u8] =
    include_bytes!("../../../../contracts/modular-facilities-v2.tmtprop.json");
pub const MODULAR_RECEPTION_DIGEST: &str =
    "sha256:a00df6330d569dd6ab8d94bab391c074306be6529fdd224d5da9f9ef601052dc";
pub const MODULAR_RECEPTION_BYTES: &[u8] =
    include_bytes!("../../../../contracts/modular-reception-v2.tmtprop.json");

const DIGEST_DOMAIN: &[u8] = b"TMT-OFFICE-PROP-PACK-V1\0";
pub const DIRECTIONAL_WORKSTATION_DIGEST: &str =
    "sha256:510f5c18585f9c626260ca7d851c10df9ee1f6858e2dedb494aff8dc7ad82003";
pub const DIRECTIONAL_WORKSTATION_BYTES: &[u8] =
    include_bytes!("../../../../contracts/directional-workstation-v2.tmtprop.json");
pub const DIRECTIONAL_LOUNGE_DIGEST: &str =
    "sha256:79b890d1e7f7a9139e856a45efdbfb111052dd4b9414050367f8290f677ea802";
pub const DIRECTIONAL_LOUNGE_BYTES: &[u8] =
    include_bytes!("../../../../contracts/directional-lounge-v2.tmtprop.json");
pub const DIRECTIONAL_RECEPTION_DIGEST: &str =
    "sha256:bbd2099aec2ebef001386e84bc28c7ff119cdecb2cb566665b51b1fabb99a93b";
pub const DIRECTIONAL_RECEPTION_BYTES: &[u8] =
    include_bytes!("../../../../contracts/directional-reception-v2.tmtprop.json");
pub const DIRECTIONAL_FACILITIES_DIGEST: &str =
    "sha256:206562d849112c6ad6bfe2099bd1dc62fcdb82c73f58c3ff7e6062dd9fcf924f";
pub const DIRECTIONAL_FACILITIES_BYTES: &[u8] =
    include_bytes!("../../../../contracts/directional-facilities-v2.tmtprop.json");
const DIRECTIONAL_DIGEST_DOMAIN: &[u8] = b"TMT-OFFICE-PROP-PACK-V2\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropPack {
    pub format_version: u8,
    pub label: String,
    pub credit: String,
    pub license: String,
    pub palette: Vec<String>,
    pub props: Vec<PropDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropDefinition {
    pub key: String,
    pub label: String,
    pub footprint: Footprint,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub pixels: Option<Vec<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub frames: Option<[Vec<String>; 4]>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub customization: Option<PropCapabilities>,
}

// An omitted version-specific field is allowed; an explicit null is not.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

impl PropDefinition {
    pub fn permits_customization(
        &self,
        value: Option<&crate::office_block::PropCustomization>,
    ) -> bool {
        let Some(value) = value else {
            return true;
        };
        self.customization.as_ref().is_some_and(|capabilities| {
            (value.tint.is_none() || capabilities.tint.is_some())
                && (value.text.is_none() || capabilities.text.is_some())
        })
    }

    fn rasters(&self) -> &[Vec<String>] {
        match (&self.pixels, &self.frames) {
            (Some(pixels), None) => std::slice::from_ref(pixels),
            (None, Some(frames)) => frames.as_slice(),
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Footprint {
    pub width: u8,
    pub height: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedPropPack {
    bytes: Vec<u8>,
    digest: String,
    pack: PropPack,
    pixel_count: usize,
}

impl ValidatedPropPack {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn pack(&self) -> &PropPack {
        &self.pack
    }

    pub fn pixel_count(&self) -> usize {
        self.pixel_count
    }

    pub fn prop(&self, key: &str) -> Option<&PropDefinition> {
        self.pack.props.iter().find(|prop| prop.key == key)
    }
}

#[derive(Debug)]
pub enum PropPackError {
    Invalid,
    TooLarge,
    Io(std::io::Error),
}

impl PartialEq for PropPackError {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Invalid, Self::Invalid)
                | (Self::TooLarge, Self::TooLarge)
                | (Self::Io(_), Self::Io(_))
        )
    }
}

impl Eq for PropPackError {}

impl std::fmt::Display for PropPackError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "Invalid Office prop pack.",
            Self::TooLarge => "Office prop pack exceeds its format's byte limit.",
            Self::Io(_) => "Could not read the Office prop pack file.",
        })
    }
}

impl std::error::Error for PropPackError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

pub fn validate_pack(bytes: &[u8]) -> Result<ValidatedPropPack, PropPackError> {
    if bytes.len() > PACK_INPUT_LIMIT {
        return Err(PropPackError::TooLarge);
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) || std::str::from_utf8(bytes).is_err() {
        return Err(PropPackError::Invalid);
    }
    let pack: PropPack = serde_json::from_slice(bytes).map_err(|_| PropPackError::Invalid)?;
    if pack.format_version == 1 && bytes.len() > V1_PACK_INPUT_LIMIT {
        return Err(PropPackError::TooLarge);
    }
    let pixel_count = validate_document(&pack)?;
    Ok(ValidatedPropPack {
        bytes: bytes.to_vec(),
        digest: framed_digest(bytes, pack.format_version),
        pack,
        pixel_count,
    })
}

pub fn builtin_pack() -> ValidatedPropPack {
    builtin_packs()[0].clone()
}

/// Immutable built-ins are admitted once per invocation, never installed as catalog rows.
pub fn builtin_packs() -> &'static [ValidatedPropPack] {
    static PACKS: OnceLock<Vec<ValidatedPropPack>> = OnceLock::new();
    PACKS.get_or_init(|| {
        [
            (BUILTIN_DIGEST, BUILTIN_BYTES),
            (WORKSHOP_DIGEST, WORKSHOP_BYTES),
            (COMMONS_DIGEST, COMMONS_BYTES),
            (WHITEBOARD_DIGEST, WHITEBOARD_BYTES),
            (BROADCASTER_DIGEST, BROADCASTER_BYTES),
            (STUDY_DIGEST, STUDY_BYTES),
            (WALL_DIGEST, WALL_BYTES),
            (MODULAR_WORKSTATION_DIGEST, MODULAR_WORKSTATION_BYTES),
            (MODULAR_MOUNTED_DIGEST, MODULAR_MOUNTED_BYTES),
            (MODULAR_LOUNGE_DIGEST, MODULAR_LOUNGE_BYTES),
            (MODULAR_FACILITIES_DIGEST, MODULAR_FACILITIES_BYTES),
            (MODULAR_RECEPTION_DIGEST, MODULAR_RECEPTION_BYTES),
            (
                DIRECTIONAL_WORKSTATION_DIGEST,
                DIRECTIONAL_WORKSTATION_BYTES,
            ),
            (DIRECTIONAL_LOUNGE_DIGEST, DIRECTIONAL_LOUNGE_BYTES),
            (DIRECTIONAL_RECEPTION_DIGEST, DIRECTIONAL_RECEPTION_BYTES),
            (DIRECTIONAL_FACILITIES_DIGEST, DIRECTIONAL_FACILITIES_BYTES),
        ]
        .into_iter()
        .map(|(digest, bytes)| {
            let pack = validate_pack(bytes).expect("embedded prop pack is a reviewed fixture");
            assert_eq!(pack.digest(), digest);
            pack
        })
        .collect()
    })
}

pub fn builtin_by_digest(digest: &str) -> Option<&'static ValidatedPropPack> {
    builtin_packs().iter().find(|pack| pack.digest() == digest)
}

pub fn command_pack_input(pack: &ValidatedPropPack) -> Value {
    json!({"bytes":STANDARD.encode(pack.bytes())})
}

fn framed_digest(bytes: &[u8], version: u8) -> String {
    format!(
        "sha256:{}",
        tmt_core::content_digest::framed_sha256(
            if version == 2 {
                DIRECTIONAL_DIGEST_DOMAIN
            } else {
                DIGEST_DOMAIN
            },
            bytes
        )
    )
}

pub fn parse_pack_digest(value: &str) -> Option<&str> {
    value
        .strip_prefix("sha256:")
        .filter(|digest| tmt_core::content_digest::is_sha256(digest))
}

pub fn parse_prop_reference(value: &str) -> Option<(&str, &str)> {
    crate::office_art_reference::parse_office_art_reference(value)
}

fn validate_document(pack: &PropPack) -> Result<usize, PropPackError> {
    let directional = pack.format_version == 2;
    let footprint_limit = if directional {
        crate::office_block::PROP_FOOTPRINT_LIMIT
    } else {
        FOOTPRINT_LIMIT
    };
    if !matches!(pack.format_version, 1 | 2)
        || !crate::indexed_art::valid_text(&pack.label, PROP_LABEL_LIMIT)
        || !crate::indexed_art::valid_text(&pack.credit, crate::indexed_art::CREDIT_LIMIT)
        || !crate::indexed_art::valid_license(&pack.license)
        || !crate::indexed_art::valid_palette_with_limit(
            &pack.palette,
            if directional { 256 } else { PALETTE_LIMIT },
        )
        || !(1..=PACK_PROP_LIMIT).contains(&pack.props.len())
    {
        return Err(PropPackError::Invalid);
    }
    let mut keys = HashSet::with_capacity(pack.props.len());
    let mut total = 0usize;
    for prop in &pack.props {
        if !crate::indexed_art::valid_key(&prop.key)
            || !keys.insert(prop.key.as_str())
            || !crate::indexed_art::valid_text(&prop.label, PROP_LABEL_LIMIT)
            || !(1..=footprint_limit).contains(&prop.footprint.width)
            || !(1..=footprint_limit).contains(&prop.footprint.height)
            || (directional && (prop.frames.is_none() || prop.pixels.is_some()))
            || (!directional && (prop.pixels.is_none() || prop.frames.is_some()))
            || (!directional && prop.customization.is_some())
        {
            return Err(PropPackError::Invalid);
        }
        for pixels in prop.rasters() {
            let count = crate::indexed_art::validate_encoded_raster(
                pixels,
                pack.palette.len(),
                None,
                None,
                if directional { 128 } else { RASTER_LIMIT },
                if directional {
                    16_384
                } else {
                    PROP_PIXEL_LIMIT
                },
                if directional { 2 } else { 1 },
            )
            .ok_or(PropPackError::Invalid)?;
            if directional
                && !pixels
                    .iter()
                    .any(|row| row.bytes().any(|byte| byte != b'0'))
            {
                return Err(PropPackError::Invalid);
            }
            total = total.checked_add(count).ok_or(PropPackError::Invalid)?;
        }
        if !customization::valid(prop, pack.palette.len()) {
            return Err(PropPackError::Invalid);
        }
    }
    (total
        <= if directional {
            131_072
        } else {
            PACK_PIXEL_LIMIT
        })
    .then_some(total)
    .ok_or(PropPackError::Invalid)
}

pub fn pack_projection(pack: &ValidatedPropPack) -> Value {
    json!({
        "digest":pack.digest(),
        "formatVersion":pack.pack().format_version,
        "label":pack.pack().label,
        "credit":pack.pack().credit,
        "license":pack.pack().license,
        "fileBytes":pack.bytes().len(),
        "pixelCount":pack.pixel_count(),
        "props":pack.pack().props.iter().map(|prop| {
            let mut value = json!({"key":prop.key,"label":prop.label,"footprint":prop.footprint});
            let dimensions = prop.rasters().iter().map(|pixels| json!({
                "width": pixels[0].len() / if pack.pack().format_version == 2 { 2 } else { 1 },
                "height": pixels.len()
            })).collect::<Vec<_>>();
            if pack.pack().format_version == 2 { value["frames"] = json!(dimensions); }
            else { value["raster"] = dimensions[0].clone(); }
            if let Some(customization) = &prop.customization {
                let fields = [("tint", customization.tint.is_some()), ("text", customization.text.is_some())]
                    .into_iter().filter_map(|(name, present)| present.then_some(name)).collect::<Vec<_>>();
                value["customizable"] = json!(fields);
            }
            value
        }).collect::<Vec<_>>()
    })
}
