//! Strict data-only Office avatar-pack validation and immutable references.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::OnceLock};

mod quality;
pub use quality::{QualityWarning, quality_warnings};

pub const PACK_INPUT_LIMIT: usize = 32 * 1024;
pub const PACK_AVATAR_LIMIT: usize = 16;
pub const PACK_CELL_LIMIT: usize = 6_144;
pub const AVATAR_WIDTH: usize = 16;
pub const AVATAR_HEIGHT: usize = 24;
#[derive(Debug, Clone, Copy)]
pub struct AvatarFormat {
    pub width: usize,
    pub height: usize,
    pub index_width: usize,
    pub palette_limit: usize,
}

pub fn avatar_format(version: u64) -> Option<AvatarFormat> {
    let (width, height, index_width, palette_limit) = match version {
        1 => (AVATAR_WIDTH, AVATAR_HEIGHT, 1, 16),
        2 => (32, 48, 2, 256),
        _ => return None,
    };
    Some(AvatarFormat {
        width,
        height,
        index_width,
        palette_limit,
    })
}
/// Raw 32 KiB source after base64 plus its strict JSON request envelope.
pub const PROTOCOL_INPUT_LIMIT: usize = 44_000;
/// Twenty maximum summary projections plus JSON escaping and envelope overhead.
pub const PROTOCOL_OUTPUT_LIMIT: usize = 128 * 1024;
const DIGEST_DOMAIN: &[u8] = b"TMT-OFFICE-AVATAR-PACK-V1\0";
const DETAIL_DIGEST_DOMAIN: &[u8] = b"TMT-OFFICE-AVATAR-PACK-V2\0";

pub fn builtin_packs() -> &'static [ValidatedAvatarPack] {
    static PACKS: OnceLock<Vec<ValidatedAvatarPack>> = OnceLock::new();
    PACKS.get_or_init(|| {
        vec![
            validate_pack(include_bytes!(
                "../../../../../../contracts/office/modular-robots-v2.tmtavatar.json"
            ))
            .expect("bundled robot pack must pass normal admission"),
        ]
    })
}

pub fn builtin_by_digest(digest: &str) -> Option<&'static ValidatedAvatarPack> {
    builtin_packs().iter().find(|pack| pack.digest() == digest)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AvatarPack {
    pub format_version: u8,
    pub label: String,
    pub credit: String,
    pub license: String,
    pub palette: Vec<String>,
    pub avatars: Vec<AvatarDefinition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvatarDefinition {
    pub key: String,
    pub label: String,
    pub pixels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedAvatarPack {
    bytes: Vec<u8>,
    digest: String,
    pack: AvatarPack,
    cell_count: usize,
}

impl ValidatedAvatarPack {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn pack(&self) -> &AvatarPack {
        &self.pack
    }
    pub fn cell_count(&self) -> usize {
        self.cell_count
    }
    pub fn format(&self) -> AvatarFormat {
        avatar_format(u64::from(self.pack.format_version)).expect("admitted avatar format")
    }
    pub fn avatar(&self, key: &str) -> Option<&AvatarDefinition> {
        self.pack.avatars.iter().find(|avatar| avatar.key == key)
    }
}

#[derive(Debug)]
pub enum AvatarPackError {
    Invalid,
    TooLarge,
    Io(std::io::Error),
}

impl PartialEq for AvatarPackError {
    fn eq(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Invalid, Self::Invalid)
                | (Self::TooLarge, Self::TooLarge)
                | (Self::Io(_), Self::Io(_))
        )
    }
}
impl Eq for AvatarPackError {}
impl std::fmt::Display for AvatarPackError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "Invalid Office avatar pack.",
            Self::TooLarge => "Office avatar pack exceeds 32 KiB.",
            Self::Io(_) => "Could not read the Office avatar pack file.",
        })
    }
}
impl std::error::Error for AvatarPackError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

pub fn validate_pack(bytes: &[u8]) -> Result<ValidatedAvatarPack, AvatarPackError> {
    if bytes.len() > PACK_INPUT_LIMIT {
        return Err(AvatarPackError::TooLarge);
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) || std::str::from_utf8(bytes).is_err() {
        return Err(AvatarPackError::Invalid);
    }
    let pack: AvatarPack = serde_json::from_slice(bytes).map_err(|_| AvatarPackError::Invalid)?;
    let format = avatar_format(u64::from(pack.format_version)).ok_or(AvatarPackError::Invalid)?;
    if !crate::indexed_art::valid_text(&pack.label, crate::indexed_art::LABEL_LIMIT)
        || !crate::indexed_art::valid_text(&pack.credit, crate::indexed_art::CREDIT_LIMIT)
        || !crate::indexed_art::valid_license(&pack.license)
        || !crate::indexed_art::valid_palette_with_limit(&pack.palette, format.palette_limit)
        || !(1..=PACK_AVATAR_LIMIT).contains(&pack.avatars.len())
    {
        return Err(AvatarPackError::Invalid);
    }
    let mut keys = HashSet::with_capacity(pack.avatars.len());
    let mut cell_count = 0usize;
    for avatar in &pack.avatars {
        if !crate::indexed_art::valid_key(&avatar.key)
            || !keys.insert(avatar.key.as_str())
            || !crate::indexed_art::valid_text(&avatar.label, crate::indexed_art::LABEL_LIMIT)
        {
            return Err(AvatarPackError::Invalid);
        }
        let count = crate::indexed_art::validate_encoded_raster(
            &avatar.pixels,
            pack.palette.len(),
            Some(format.width),
            Some(format.height),
            format.height,
            format.width * format.height,
            format.index_width,
        )
        .ok_or(AvatarPackError::Invalid)?;
        if !avatar
            .pixels
            .iter()
            .any(|row| row.bytes().any(|byte| byte != b'0'))
        {
            return Err(AvatarPackError::Invalid);
        }
        cell_count = cell_count
            .checked_add(count)
            .filter(|value| *value <= PACK_CELL_LIMIT)
            .ok_or(AvatarPackError::Invalid)?;
    }
    Ok(ValidatedAvatarPack {
        digest: framed_digest(bytes, pack.format_version),
        bytes: bytes.to_vec(),
        pack,
        cell_count,
    })
}

pub fn framed_digest(bytes: &[u8], version: u8) -> String {
    format!(
        "sha256:{}",
        tmt_core::content_digest::framed_sha256(
            if version == 2 {
                DETAIL_DIGEST_DOMAIN
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

pub fn parse_avatar_reference(value: &str) -> Option<(&str, &str)> {
    crate::office_art_reference::parse_office_art_reference(value)
}

pub fn command_pack_input(pack: &ValidatedAvatarPack) -> Value {
    json!({"bytes":STANDARD.encode(pack.bytes())})
}
