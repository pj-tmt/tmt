//! Data-only World definitions and instances. Decoding does not grant host access.

use crate::office_block::{BLOCK_SIZE, INPUT_LIMIT, LocalBlockLayout, PropPlacement};
use crate::office_extension::{ExtensionAttachment, ResourceBinding};
use serde::{Deserialize, Serialize};

pub mod preflight;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionError {
    InvalidInput,
    InvalidDefinition,
    InvalidInstance,
    DefinitionMismatch,
    BindingMismatch,
    PlacementInvalid,
}

impl std::fmt::Display for ExtensionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidInput => "Expected two bounded UTF-8 JSON documents.",
            Self::InvalidDefinition => "The extension definition is invalid or unsupported.",
            Self::InvalidInstance => "The extension instance is invalid or unsupported.",
            Self::DefinitionMismatch => "The instance references a different definition ID.",
            Self::BindingMismatch => {
                "The instance binding does not match the definition's resource kind."
            }
            Self::PlacementInvalid => {
                "The rotated artwork footprint extends outside the 32-tile grid."
            }
        })
    }
}
impl std::error::Error for ExtensionError {}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionDefinition {
    pub format_version: u8,
    pub world_api_version: u8,
    pub id: String,
    pub label: String,
    pub appearance: ExtensionAppearance,
    pub action: Action,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionAppearance {
    pub prop: String,
    pub footprint: crate::codec::office_prop::Footprint,
}

impl ExtensionAppearance {
    pub(crate) fn placement(&self, x: u8, y: u8, rotation: u8) -> PropPlacement {
        PropPlacement {
            prop: self.prop.clone(),
            footprint_width: self.footprint.width,
            footprint_height: self.footprint.height,
            x: i32::from(x),
            y: i32::from(y),
            rotation,
            customization: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Action {
    pub id: ActionId,
    pub label: String,
    pub capability: Capability,
    pub resource_kind: ResourceKind,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ActionId {
    #[serde(rename = "open")]
    Open,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Capability {
    #[serde(rename = "discussion.open")]
    DiscussionOpen,
    #[serde(rename = "whiteboard.open")]
    WhiteboardOpen,
    #[serde(rename = "broadcast.open")]
    BroadcastOpen,
    #[serde(rename = "link.open")]
    LinkOpen,
    #[serde(rename = "notebook.open")]
    NotebookOpen,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ResourceKind {
    #[serde(rename = "office-board")]
    OfficeBoard,
    #[serde(rename = "whiteboard")]
    Whiteboard,
    #[serde(rename = "office-broadcast")]
    OfficeBroadcast,
    #[serde(rename = "external-link")]
    ExternalLink,
    #[serde(rename = "notebook")]
    Notebook,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionInstance {
    pub format_version: u8,
    pub id: String,
    pub definition: String,
    pub space: Space,
    pub x: u8,
    pub y: u8,
    pub rotation: u8,
    pub binding: ExtensionResourceBinding,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Space {
    #[serde(rename = "commons")]
    Commons,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ExtensionResourceBinding {
    #[serde(rename = "notebook")]
    Notebook {
        #[serde(rename = "identityId")]
        identity_id: String,
    },
    #[serde(rename = "external-link")]
    ExternalLink { url: String },
    #[serde(rename = "office-broadcast")]
    OfficeBroadcast {},
    #[serde(rename = "office-board")]
    OfficeBoard {
        #[serde(
            rename = "roomId",
            default,
            deserialize_with = "present_room_id",
            skip_serializing_if = "Option::is_none"
        )]
        room_id: Option<String>,
    },
    #[serde(rename = "whiteboard")]
    Whiteboard {
        #[serde(rename = "documentId")]
        document_id: String,
    },
}

fn present_room_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

impl From<ExtensionResourceBinding> for ResourceBinding {
    fn from(value: ExtensionResourceBinding) -> Self {
        match value {
            ExtensionResourceBinding::Notebook { identity_id } => Self::Notebook { identity_id },
            ExtensionResourceBinding::ExternalLink { url } => Self::ExternalLink { url },
            ExtensionResourceBinding::OfficeBoard { room_id } => Self::OfficeBoard { room_id },
            ExtensionResourceBinding::OfficeBroadcast {} => Self::OfficeBroadcast,
            ExtensionResourceBinding::Whiteboard { document_id } => {
                Self::Whiteboard { document_id }
            }
        }
    }
}

impl From<&ResourceBinding> for ExtensionResourceBinding {
    fn from(value: &ResourceBinding) -> Self {
        match value {
            ResourceBinding::Notebook { identity_id } => Self::Notebook {
                identity_id: identity_id.clone(),
            },
            ResourceBinding::ExternalLink { url } => Self::ExternalLink { url: url.clone() },
            ResourceBinding::OfficeBoard { room_id } => Self::OfficeBoard {
                room_id: room_id.clone(),
            },
            ResourceBinding::OfficeBroadcast => Self::OfficeBroadcast {},
            ResourceBinding::Whiteboard { document_id } => Self::Whiteboard {
                document_id: document_id.clone(),
            },
        }
    }
}

impl ResourceKind {
    fn accepts(&self, binding: &ResourceBinding) -> bool {
        matches!(
            (self, binding),
            (Self::OfficeBoard, ResourceBinding::OfficeBoard { .. })
                | (Self::Whiteboard, ResourceBinding::Whiteboard { .. })
                | (Self::OfficeBroadcast, ResourceBinding::OfficeBroadcast)
                | (Self::ExternalLink, ResourceBinding::ExternalLink { .. })
                | (Self::Notebook, ResourceBinding::Notebook { .. })
        )
    }
}

/// Bundled declarations, not a runtime loader or resource store.
pub fn bundled_definitions() -> &'static [ExtensionDefinition; 5] {
    static DEFINITIONS: std::sync::OnceLock<[ExtensionDefinition; 5]> = std::sync::OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        [
            include_bytes!("../../../../../../contracts/office/discussion-extension-v1.json")
                .as_slice(),
            include_bytes!("../../../../../../contracts/office/whiteboard-extension-v1.json")
                .as_slice(),
            include_bytes!("../../../../../../contracts/office/broadcaster-extension-v1.json")
                .as_slice(),
            include_bytes!("../../../../../../contracts/office/link-extension-v1.json").as_slice(),
            include_bytes!("../../../../../../contracts/office/notebook-extension-v1.json")
                .as_slice(),
        ]
        .map(|bytes| decode_definition(bytes).expect("admitted bundled extension"))
    })
}

pub(crate) fn bundled_instances() -> &'static [ExtensionInstance; 3] {
    static INSTANCES: std::sync::OnceLock<[ExtensionInstance; 3]> = std::sync::OnceLock::new();
    INSTANCES.get_or_init(|| {
        [
            include_bytes!("../../../../../../contracts/office/lobby-extension-v1.json").as_slice(),
            include_bytes!("../../../../../../contracts/office/lobby-whiteboard-v1.json")
                .as_slice(),
            include_bytes!("../../../../../../contracts/office/lobby-broadcaster-v1.json")
                .as_slice(),
        ]
        .map(|bytes| decode_instance(bytes).expect("admitted bundled extension instance"))
    })
}

/// Missing definitions remain inert references; known definitions cannot change resource kind.
pub fn validate_attachment(attachment: &ExtensionAttachment) -> Result<(), ExtensionError> {
    if !attachment.is_valid() {
        return Err(ExtensionError::InvalidInstance);
    }
    if bundled_definitions()
        .iter()
        .find(|definition| definition.id == attachment.definition)
        .is_some_and(|definition| !definition.action.resource_kind.accepts(&attachment.binding))
    {
        return Err(ExtensionError::BindingMismatch);
    }
    Ok(())
}

pub fn decode_definition(bytes: &[u8]) -> Result<ExtensionDefinition, ExtensionError> {
    let invalid = || ExtensionError::InvalidDefinition;
    if bytes.len() > INPUT_LIMIT {
        return Err(invalid());
    }
    let input: ExtensionDefinition = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if input.format_version != 1
        || input.world_api_version != 1
        || !crate::indexed_art::valid_key(&input.id)
        || !crate::indexed_art::valid_text(&input.label, crate::indexed_art::LABEL_LIMIT)
        || !crate::indexed_art::valid_text(&input.action.label, crate::indexed_art::LABEL_LIMIT)
        || !matches!(
            (&input.action.capability, &input.action.resource_kind),
            (Capability::DiscussionOpen, ResourceKind::OfficeBoard)
                | (Capability::WhiteboardOpen, ResourceKind::Whiteboard)
                | (Capability::BroadcastOpen, ResourceKind::OfficeBroadcast)
                | (Capability::LinkOpen, ResourceKind::ExternalLink)
                | (Capability::NotebookOpen, ResourceKind::Notebook)
        )
        || LocalBlockLayout::new(vec![input.appearance.placement(0, 0, 0)]).is_err()
    {
        return Err(invalid());
    }
    Ok(input)
}

pub fn decode_instance(bytes: &[u8]) -> Result<ExtensionInstance, ExtensionError> {
    let invalid = || ExtensionError::InvalidInstance;
    if bytes.len() > INPUT_LIMIT {
        return Err(invalid());
    }
    let input: ExtensionInstance = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if !ResourceBinding::from(input.binding.clone()).is_valid() {
        return Err(invalid());
    }
    if input.format_version != 1
        || !crate::indexed_art::valid_key(&input.id)
        || !crate::indexed_art::valid_key(&input.definition)
        || input.x >= BLOCK_SIZE
        || input.y >= BLOCK_SIZE
        || input.rotation > 3
    {
        return Err(invalid());
    }
    Ok(input)
}

/// Structural composition only: neither catalog availability nor host grants are checked.
pub fn validate_pair(
    definition: &[u8],
    instance: &[u8],
) -> Result<(ExtensionDefinition, ExtensionInstance), ExtensionError> {
    let definition = decode_definition(definition)?;
    let instance = decode_instance(instance)?;
    if definition.id != instance.definition {
        return Err(ExtensionError::DefinitionMismatch);
    }
    if !definition
        .action
        .resource_kind
        .accepts(&instance.binding.clone().into())
    {
        return Err(ExtensionError::BindingMismatch);
    }
    LocalBlockLayout::new(vec![definition.appearance.placement(
        instance.x,
        instance.y,
        instance.rotation,
    )])
    .map_err(|_| ExtensionError::PlacementInvalid)?;
    Ok((definition, instance))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn notebook_bindings_share_structural_admission_without_granting_access() {
        let vectors: Value = serde_json::from_slice(include_bytes!(
            "../../../../../../contracts/office/notebook-binding-vectors.json"
        ))
        .unwrap();
        for case in vectors.as_array().unwrap() {
            let decoded =
                serde_json::from_value::<ExtensionResourceBinding>(case["binding"].clone());
            let valid = decoded
                .as_ref()
                .is_ok_and(|binding| ResourceBinding::from(binding.clone()).is_valid());
            assert_eq!(valid, case["valid"].as_bool().unwrap(), "{}", case["name"]);
            if valid {
                let binding: ResourceBinding = decoded.unwrap().into();
                assert_eq!(
                    serde_json::to_value(ExtensionResourceBinding::from(&binding)).unwrap(),
                    case["binding"]
                );
                assert!(
                    validate_attachment(&ExtensionAttachment {
                        definition: "tmt-notebook".into(),
                        binding: binding.clone()
                    })
                    .is_ok()
                );
                assert!(
                    validate_attachment(&ExtensionAttachment {
                        definition: "tmt-link".into(),
                        binding
                    })
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn shared_extension_vectors_and_defaults_are_admitted_exactly() {
        let vectors: Value = serde_json::from_slice(include_bytes!(
            "../../../../../../contracts/office/extension-vectors.json"
        ))
        .unwrap();
        for case in vectors["definitions"].as_array().unwrap() {
            let bytes = serde_json::to_vec(&case["value"]).unwrap();
            let result = decode_definition(&bytes);
            assert_eq!(
                result.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            if let Ok(value) = result {
                assert_eq!(serde_json::to_value(value).unwrap(), case["value"]);
            }
        }
        for case in vectors["instances"].as_array().unwrap() {
            let bytes = serde_json::to_vec(&case["value"]).unwrap();
            let result = decode_instance(&bytes);
            assert_eq!(
                result.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            if let Ok(value) = result {
                assert_eq!(serde_json::to_value(value).unwrap(), case["value"]);
            }
        }
        for (definition_bytes, instance_bytes) in [
            (
                include_bytes!("../../../../../../contracts/office/discussion-extension-v1.json")
                    .as_slice(),
                include_bytes!("../../../../../../contracts/office/lobby-extension-v1.json")
                    .as_slice(),
            ),
            (
                include_bytes!("../../../../../../contracts/office/whiteboard-extension-v1.json")
                    .as_slice(),
                include_bytes!("../../../../../../contracts/office/lobby-whiteboard-v1.json")
                    .as_slice(),
            ),
            (
                include_bytes!("../../../../../../contracts/office/broadcaster-extension-v1.json")
                    .as_slice(),
                include_bytes!("../../../../../../contracts/office/lobby-broadcaster-v1.json")
                    .as_slice(),
            ),
        ] {
            let definition = decode_definition(definition_bytes).unwrap();
            let instance = decode_instance(instance_bytes).unwrap();
            assert_eq!(instance.definition, definition.id);
            let (digest, key) = definition.appearance.prop.split_once('/').unwrap();
            let pack = crate::codec::office_prop::builtin_by_digest(digest).unwrap();
            assert_eq!(
                pack.prop(key).unwrap().footprint,
                definition.appearance.footprint
            );
        }
    }

    #[test]
    fn external_links_share_core_admission_and_strict_attachment_fields() {
        let definition =
            include_bytes!("../../../../../../contracts/office/link-extension-v1.json");
        let vectors: Value = serde_json::from_slice(include_bytes!(
            "../../../../../../contracts/office/external-link-vectors.json"
        ))
        .unwrap();
        for case in vectors.as_array().unwrap() {
            let instance = serde_json::json!({ "formatVersion": 1, "id": "web-plaque",
                "definition": "tmt-link", "space": "commons", "x": 0, "y": 0, "rotation": 0,
                "binding": { "kind": "external-link", "url": case["url"] } });
            let result = validate_pair(definition, &serde_json::to_vec(&instance).unwrap());
            assert_eq!(
                result.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            if let Ok((_, instance)) = result {
                assert_eq!(
                    serde_json::to_value(instance.binding).unwrap(),
                    serde_json::json!({"kind":"external-link","url":case["url"]})
                );
            }
        }
        let valid = r#"{"kind":"external-link","url":"https://example.com"}"#;
        assert!(serde_json::from_str::<ExtensionResourceBinding>(valid).is_ok());
        for invalid in [
            r#"{"kind":"external-link","url":"https://example.com","url":"javascript:test"}"#,
            r#"{"kind":"external-link","url":"https://example.com","command":"echo test"}"#,
        ] {
            assert!(serde_json::from_str::<ExtensionResourceBinding>(invalid).is_err());
        }
    }

    #[test]
    fn raw_duplicate_fields_and_oversized_documents_are_rejected() {
        let definition = std::str::from_utf8(include_bytes!(
            "../../../../../../contracts/office/discussion-extension-v1.json"
        ))
        .unwrap();
        for (original, duplicate) in [
            (
                "\"formatVersion\": 1",
                "\"formatVersion\": 1, \"formatVersion\": 1",
            ),
            (
                "\"capability\": \"discussion.open\"",
                "\"capability\": \"discussion.open\", \"capability\": \"discussion.open\"",
            ),
        ] {
            let invalid = definition.replacen(original, duplicate, 1);
            assert_ne!(invalid, definition);
            assert!(decode_definition(invalid.as_bytes()).is_err());
        }
        let oversized = vec![b' '; INPUT_LIMIT + 1];
        assert!(decode_definition(&oversized).is_err());
        assert!(decode_instance(&oversized).is_err());
        let instance = std::str::from_utf8(include_bytes!(
            "../../../../../../contracts/office/lobby-whiteboard-v1.json"
        ))
        .unwrap();
        for (original, duplicate) in [
            (
                "\"kind\": \"whiteboard\"",
                "\"kind\": \"whiteboard\", \"kind\": \"whiteboard\"",
            ),
            (
                "\"documentId\": \"lobby\"",
                "\"documentId\": \"lobby\", \"documentId\": \"lobby\"",
            ),
        ] {
            let invalid = instance.replacen(original, duplicate, 1);
            assert_ne!(invalid, instance);
            assert!(decode_instance(invalid.as_bytes()).is_err());
        }
    }
}
