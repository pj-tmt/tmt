//! Readable block input/output. Firestore envelopes belong to the remote adapter.

use crate::json_integer::whole;
use crate::office_block::BLOCK_SIZE;
use crate::office_block::BlockLayout;
use crate::office_block::Furniture;
use crate::office_block::FurnitureAsset;
use crate::office_block::INPUT_LIMIT;
use crate::office_block::LocalBlockLayout;
use crate::office_block::LocalBlockTarget;
use crate::office_block::MAX_REVISION;
use crate::office_block::OBJECT_LIMIT;
use crate::office_block::PropCustomization;
use crate::office_block::PropPlacement;
use crate::office_protocol::OfficeError;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::OnceLock;

/// Shipped defaults are projected, not materialized by observation.
pub fn default_local_layout(target: &LocalBlockTarget) -> LocalBlockLayout {
    match target {
        LocalBlockTarget::Identity(_) => {
            LocalBlockLayout::new(Vec::new()).expect("empty layout is valid")
        }
        LocalBlockTarget::Lobby => {
            static LOBBY: OnceLock<LocalBlockLayout> = OnceLock::new();
            LOBBY
                .get_or_init(|| {
                    decode_local_layout(include_bytes!(
                        "../../../../contracts/lobby-preset-v1.json"
                    ))
                    .expect("bundled lobby layout must be admitted")
                })
                .clone()
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutInput {
    objects: Vec<ObjectInput>,
}

impl LayoutInput {
    pub fn validate(self) -> Result<BlockLayout, OfficeError> {
        validated_objects(self.objects)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectInput {
    asset: String,
    x: u8,
    y: u8,
    rotation: u8,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LocalLayoutInput {
    Local(LocalLayoutVersionedInput),
    V1(LayoutInput),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalLayoutVersionedInput {
    version: u8,
    objects: Vec<PropPlacementInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PropPlacementInput {
    prop: String,
    footprint: FootprintInput,
    #[serde(deserialize_with = "whole")]
    x: i32,
    #[serde(deserialize_with = "whole")]
    y: i32,
    #[serde(deserialize_with = "whole")]
    rotation: u8,
    #[serde(default, deserialize_with = "present")]
    customization: Option<CustomizationInput>,
}

impl PropPlacementInput {
    pub(crate) fn into_placement(self) -> PropPlacement {
        PropPlacement {
            prop: self.prop,
            footprint_width: self.footprint.width,
            footprint_height: self.footprint.height,
            x: self.x,
            y: self.y,
            rotation: self.rotation,
            customization: self.customization.map(|value| PropCustomization {
                tint: value.tint,
                text: value.text,
            }),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomizationInput {
    #[serde(default, deserialize_with = "present")]
    tint: Option<String>,
    #[serde(default, deserialize_with = "present")]
    text: Option<String>,
}

fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FootprintInput {
    #[serde(deserialize_with = "whole")]
    width: u8,
    #[serde(deserialize_with = "whole")]
    height: u8,
}

fn validated_objects(objects: Vec<ObjectInput>) -> Result<BlockLayout, OfficeError> {
    let objects = objects
        .into_iter()
        .map(|object| {
            Ok(Furniture {
                asset: FurnitureAsset::parse(&object.asset).ok_or(OfficeError::LayoutInvalid)?,
                x: object.x,
                y: object.y,
                rotation: object.rotation,
            })
        })
        .collect::<Result<Vec<_>, OfficeError>>()?;
    BlockLayout::new(objects).map_err(|_| OfficeError::LayoutInvalid)
}

pub fn decode_layout(bytes: &[u8]) -> Result<BlockLayout, OfficeError> {
    if bytes.len() > INPUT_LIMIT {
        return Err(OfficeError::LayoutInvalid);
    }
    let input: LayoutInput =
        serde_json::from_slice(bytes).map_err(|_| OfficeError::LayoutInvalid)?;
    input.validate()
}

pub fn decode_local_layout(bytes: &[u8]) -> Result<LocalBlockLayout, OfficeError> {
    if bytes.len() > INPUT_LIMIT {
        return Err(OfficeError::LayoutInvalid);
    }
    match serde_json::from_slice::<LocalLayoutInput>(bytes)
        .map_err(|_| OfficeError::LayoutInvalid)?
    {
        LocalLayoutInput::V1(input) => input
            .validate()
            .map(|layout| LocalBlockLayout::from_legacy(&layout)),
        LocalLayoutInput::Local(input) if matches!(input.version, 2 | 3) => {
            if input.version == 2
                && input
                    .objects
                    .iter()
                    .any(|object| object.customization.is_some())
            {
                return Err(OfficeError::LayoutInvalid);
            }
            let objects = input
                .objects
                .into_iter()
                .map(PropPlacementInput::into_placement)
                .collect();
            LocalBlockLayout::new(objects).map_err(|_| OfficeError::LayoutInvalid)
        }
        LocalLayoutInput::Local(_) => Err(OfficeError::LayoutInvalid),
    }
}

pub fn local_layout_value(layout: &LocalBlockLayout) -> Value {
    json!({
        "version":if layout.objects().iter().any(|object| object.customization.is_some()) { 3 } else { 2 },
        "objects":layout.objects().iter().map(placement_value).collect::<Vec<_>>()
    })
}

pub(crate) fn placement_value(object: &PropPlacement) -> Value {
    let mut value = json!({
    "prop":object.prop,
    "footprint":{"width":object.footprint_width,"height":object.footprint_height},
    "x":object.x,
    "y":object.y,
    "rotation":object.rotation
    });
    if let Some(customization) = &object.customization {
        let mut fields = serde_json::Map::new();
        if let Some(tint) = &customization.tint {
            fields.insert("tint".into(), json!(tint));
        }
        if let Some(text) = &customization.text {
            fields.insert("text".into(), json!(text));
        }
        value["customization"] = Value::Object(fields);
    }
    value
}

pub fn layout_value(layout: &BlockLayout) -> Value {
    json!({"objects": layout.objects().iter().map(|object| json!({
        "asset": object.asset.name(), "x":object.x, "y":object.y,"rotation":object.rotation
    })).collect::<Vec<_>>()})
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockSnapshot {
    pub block_id: String,
    pub revision: u64,
    pub layout: BlockLayout,
}

impl BlockSnapshot {
    pub fn public_value(&self) -> Value {
        let mut value = self.wire_value();
        value["limits"] = json!({"size":BLOCK_SIZE,"objects":OBJECT_LIMIT});
        value["catalog"] = json!(
            FurnitureAsset::ALL
                .iter()
                .map(|asset| {
                    let (width, height) = asset.dimensions();
                    json!({"asset":asset.name(),"width":width,"height":height})
                })
                .collect::<Vec<_>>()
        );
        value
    }

    pub fn wire_value(&self) -> Value {
        let mut value = layout_value(&self.layout);
        value["blockId"] = json!(self.block_id);
        value["revision"] = json!(self.revision);
        value
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, OfficeError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Snapshot {
            block_id: String,
            revision: u64,
            objects: Vec<ObjectInput>,
        }
        let wire: Snapshot =
            serde_json::from_slice(bytes).map_err(|_| OfficeError::CredentialsInvalid)?;
        if uuid::Uuid::parse_str(&wire.block_id)
            .ok()
            .is_none_or(|id| id.to_string() != wire.block_id)
            || wire.revision > MAX_REVISION
        {
            return Err(OfficeError::CredentialsInvalid);
        }
        let layout =
            validated_objects(wire.objects).map_err(|_| OfficeError::CredentialsInvalid)?;
        if wire.revision == 0 && !layout.objects().is_empty() {
            return Err(OfficeError::CredentialsInvalid);
        }
        Ok(Self {
            block_id: wire.block_id,
            revision: wire.revision,
            layout,
        })
    }
}
