//! Data-only attachments. A reference is not permission to invoke a host capability.

use crate::{
    office_art_reference::valid_office_art_key, office_whiteboard::document::valid_document_id,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceBinding {
    OfficeBoard { room_id: Option<String> },
    Whiteboard { document_id: String },
    Notebook { identity_id: String },
    OfficeBroadcast,
    ExternalLink { url: String },
}

impl ResourceBinding {
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Whiteboard { document_id } => valid_document_id(document_id),
            Self::Notebook { identity_id } => tmt_core::dispatch::canonical_id(identity_id),
            Self::ExternalLink { url } => valid_external_link(url),
            Self::OfficeBoard { room_id } => room_id
                .as_deref()
                .is_none_or(tmt_core::dispatch::canonical_id),
            Self::OfficeBroadcast => true,
        }
    }
}

/// A bounded, explicit web destination; parsing does not resolve DNS or fetch it.
pub fn valid_external_link(value: &str) -> bool {
    if value.len() > 2048
        || value.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || c == '\\'
                || matches!(c, '\u{feff}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
    {
        return false;
    }
    let Some((scheme, rest)) = value.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    url::Url::parse(value).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.has_host()
            && url.username().is_empty()
            && url.password().is_none()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_link_conformance_and_byte_limit() {
        let vectors: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../contracts/external-link-vectors.json"
        ))
        .unwrap();
        for case in vectors.as_array().unwrap() {
            assert_eq!(
                valid_external_link(case["url"].as_str().unwrap()),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
        let base = "https://example.com/";
        assert!(valid_external_link(&format!(
            "{base}{}",
            "a".repeat(2048 - base.len())
        )));
        assert!(!valid_external_link(&format!(
            "{base}{}",
            "a".repeat(2049 - base.len())
        )));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionAttachment {
    pub definition: String,
    pub binding: ResourceBinding,
}

impl ExtensionAttachment {
    pub fn is_valid(&self) -> bool {
        valid_office_art_key(&self.definition) && self.binding.is_valid()
    }
}
