//! Pure domain validation and storage codec for Office block layouts.

/// The block is a square room measured in tiles.
pub const BLOCK_SIZE: u8 = 32;
/// The maximum number of ordered furniture objects in a layout.
pub const OBJECT_LIMIT: usize = 16;
/// Directional furniture may occupy up to half of a room along either axis.
pub const PROP_FOOTPRINT_LIMIT: u8 = 16;
/// The largest revision exactly representable by a JavaScript number.
pub const MAX_REVISION: u64 = tmt_core::limits::MAX_JS_SAFE_INTEGER;
/// The maximum input size reserved for bounded block documents.
pub const INPUT_LIMIT: usize = 65_536;
/// Canonical local layouts, including bounded text, fit this SQLite byte budget.
pub const STORED_LAYOUT_LIMIT: usize = 8192;
pub const PROP_TEXT_LIMIT: usize = 64;
pub const BUILTIN_PROP_PACK_DIGEST: &str =
    "sha256:5aa6a2d239d7111586abc06be799b2a1ec2ca46619752a90ae08a13e414afb6a";

/// The fixed furniture catalog shared by the browser and native adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FurnitureAsset {
    Desk,
    Chair,
    Plant,
    Rug,
}

impl FurnitureAsset {
    /// The catalog order is part of the domain API for consumers that build a menu.
    pub const ALL: [Self; 4] = [Self::Desk, Self::Chair, Self::Plant, Self::Rug];

    /// Parse the public, lower-case asset name exactly.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|asset| asset.name() == value)
    }

    /// The public asset name used by readable views and commands.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Desk => "desk",
            Self::Chair => "chair",
            Self::Plant => "plant",
            Self::Rug => "rug",
        }
    }

    /// The single-character storage token for this asset.
    pub const fn token(self) -> char {
        match self {
            Self::Desk => 'd',
            Self::Chair => 'c',
            Self::Plant => 'p',
            Self::Rug => 'r',
        }
    }

    /// The unrotated footprint in tiles, as width then height.
    pub const fn dimensions(self) -> (u8, u8) {
        match self {
            Self::Desk => (4, 2),
            Self::Chair | Self::Plant => (2, 2),
            Self::Rug => (6, 4),
        }
    }

    fn from_token(value: u8) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|asset| asset.token() as u8 == value)
    }
}

/// One ordered decoration in a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Furniture {
    pub asset: FurnitureAsset,
    pub x: u8,
    pub y: u8,
    pub rotation: u8,
}

impl Furniture {
    /// Return the rotated footprint in tiles, as width then height.
    pub const fn dimensions(self) -> (u8, u8) {
        let (width, height) = self.asset.dimensions();
        if self.rotation % 2 == 1 {
            (height, width)
        } else {
            (width, height)
        }
    }

    fn validate(self) -> Result<(), LayoutError> {
        if self.rotation > 3 {
            return Err(LayoutError::InvalidRotation);
        }
        let (width, height) = self.dimensions();
        if self.x > BLOCK_SIZE.saturating_sub(width) || self.y > BLOCK_SIZE.saturating_sub(height) {
            return Err(LayoutError::OutOfBounds);
        }
        Ok(())
    }
}

/// A validated, ordered block layout. Overlap is intentionally preserved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockLayout {
    objects: Vec<Furniture>,
}

impl BlockLayout {
    /// Validate an ordered list without sorting, deduplicating, or rejecting overlap.
    pub fn new(objects: Vec<Furniture>) -> Result<Self, LayoutError> {
        if objects.len() > OBJECT_LIMIT {
            return Err(LayoutError::TooManyObjects);
        }
        objects.iter().copied().try_for_each(Furniture::validate)?;
        Ok(Self { objects })
    }

    /// Borrow the validated objects in their original paint order.
    pub fn objects(&self) -> &[Furniture] {
        &self.objects
    }

    /// Encode the layout as four-character storage tokens.
    pub fn encode(&self) -> Vec<String> {
        self.objects
            .iter()
            .map(|object| {
                format!(
                    "{}{}{}{}",
                    object.asset.token(),
                    object.rotation,
                    encode_digit(object.x),
                    encode_digit(object.y)
                )
            })
            .collect()
    }

    /// Decode and validate the ordered storage tokens.
    pub fn decode(tokens: &[String]) -> Result<Self, LayoutError> {
        if tokens.len() > OBJECT_LIMIT {
            return Err(LayoutError::TooManyObjects);
        }

        let objects = tokens
            .iter()
            .map(|token| {
                let bytes = token.as_bytes();
                if bytes.len() != 4 {
                    return Err(LayoutError::InvalidToken);
                }
                let asset =
                    FurnitureAsset::from_token(bytes[0]).ok_or(LayoutError::InvalidToken)?;
                let rotation = bytes[1]
                    .checked_sub(b'0')
                    .filter(|rotation| *rotation <= 3)
                    .ok_or(LayoutError::InvalidToken)?;
                let x = decode_digit(bytes[2]).ok_or(LayoutError::InvalidToken)?;
                let y = decode_digit(bytes[3]).ok_or(LayoutError::InvalidToken)?;
                Ok(Furniture {
                    asset,
                    x,
                    y,
                    rotation,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(objects)
    }
}

/// Optional local-v3 values; capability authorization belongs to prop resolution.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PropCustomization {
    pub tint: Option<String>,
    pub text: Option<String>,
}

impl PropCustomization {
    pub fn validate(&self) -> Result<(), LayoutError> {
        let valid_tint = self.tint.as_ref().is_none_or(|value| {
            let bytes = value.as_bytes();
            bytes.len() == 7
                && bytes[0] == b'#'
                && bytes[1..]
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        });
        let valid_text = self.text.as_ref().is_none_or(|value| {
            !value.trim().is_empty()
                && value.len() <= PROP_TEXT_LIMIT
                && value.chars().count() <= 24
                && !value.chars().any(|character| {
                    character.is_control()
                        || matches!(character, '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                })
        });
        if (self.tint.is_none() && self.text.is_none()) || !valid_tint || !valid_text {
            return Err(LayoutError::InvalidCustomization);
        }
        Ok(())
    }
}

/// One canonical immutable prop placement, with optional local customization.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PropPlacement {
    pub prop: String,
    pub footprint_width: u8,
    pub footprint_height: u8,
    pub x: i32,
    pub y: i32,
    pub rotation: u8,
    pub customization: Option<PropCustomization>,
}

impl PropPlacement {
    pub const fn dimensions(&self) -> (u8, u8) {
        if self.rotation % 2 == 1 {
            (self.footprint_height, self.footprint_width)
        } else {
            (self.footprint_width, self.footprint_height)
        }
    }

    /// Shared artwork rules; the enclosing layout owns its placement surface.
    pub fn validate_appearance(&self) -> Result<(), LayoutError> {
        if let Some(customization) = &self.customization {
            customization.validate()?;
        }
        if crate::office_art_reference::parse_office_art_reference(&self.prop).is_none()
            || !(1..=PROP_FOOTPRINT_LIMIT).contains(&self.footprint_width)
            || !(1..=PROP_FOOTPRINT_LIMIT).contains(&self.footprint_height)
        {
            return Err(LayoutError::InvalidReference);
        }
        if self.rotation > 3 {
            return Err(LayoutError::InvalidRotation);
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), LayoutError> {
        self.validate_appearance()?;
        let (width, height) = self.dimensions();
        if self.x < 0
            || self.y < 0
            || self.x > i32::from(BLOCK_SIZE.saturating_sub(width))
            || self.y > i32::from(BLOCK_SIZE.saturating_sub(height))
        {
            return Err(LayoutError::OutOfBounds);
        }
        Ok(())
    }
}

/// A local layout owner. The installation lobby is never a synthetic identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalBlockTarget {
    Identity(String),
    Lobby,
}

impl LocalBlockTarget {
    pub fn identity_id(&self) -> Option<&str> {
        match self {
            Self::Identity(id) => Some(id),
            Self::Lobby => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Identity(_) => "identity",
            Self::Lobby => "lobby",
        }
    }
}

/// Canonical local block layout. Overlap and paint order are intentional.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalBlockLayout {
    objects: Vec<PropPlacement>,
}

impl LocalBlockLayout {
    pub fn new(objects: Vec<PropPlacement>) -> Result<Self, LayoutError> {
        if objects.len() > OBJECT_LIMIT {
            return Err(LayoutError::TooManyObjects);
        }
        objects.iter().try_for_each(PropPlacement::validate)?;
        Ok(Self { objects })
    }

    pub fn objects(&self) -> &[PropPlacement] {
        &self.objects
    }

    pub fn from_legacy(layout: &BlockLayout) -> Self {
        let objects = layout
            .objects()
            .iter()
            .map(|item| {
                let (width, height) = item.asset.dimensions();
                PropPlacement {
                    prop: format!("{BUILTIN_PROP_PACK_DIGEST}/{}", item.asset.name()),
                    footprint_width: width,
                    footprint_height: height,
                    x: i32::from(item.x),
                    y: i32::from(item.y),
                    rotation: item.rotation,
                    customization: None,
                }
            })
            .collect();
        Self { objects }
    }
}

/// Why a proposed or stored layout is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    TooManyObjects,
    InvalidRotation,
    OutOfBounds,
    InvalidToken,
    InvalidReference,
    InvalidCustomization,
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::TooManyObjects => "Block layout has too many objects.",
            Self::InvalidRotation => "Furniture rotation must be between zero and three.",
            Self::OutOfBounds => "Furniture footprint must fit inside the block.",
            Self::InvalidToken => "Invalid stored furniture token.",
            Self::InvalidReference => "Invalid immutable Office prop reference or footprint.",
            Self::InvalidCustomization => "Invalid Office prop tint or display text.",
        })
    }
}

impl std::error::Error for LayoutError {}

fn encode_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=31 => (b'a' + value - 10) as char,
        _ => unreachable!("validated block coordinates are base-32 digits"),
    }
}

fn decode_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'v' => Some(value - b'a' + 10),
        _ => None,
    }
}
