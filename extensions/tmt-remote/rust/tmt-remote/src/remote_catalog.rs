//! Public references only. Importing or removing one never grants access or changes cloud state.
use crate::{
    error::RemoteError,
    state::{Layout, document_error, document_result},
    wire,
};
use serde::{Deserialize, Serialize};
use std::io::Read;
const FILE: &str = "remotes.json";
const BYTES: usize = crate::limits::REMOTE_CATALOG_BYTES;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteReference {
    pub name: String,
    pub kind: String,
    pub url: String,
    pub project: Option<String>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    version: u8,
    entries: Vec<RemoteReference>,
}
pub fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name != "local"
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
fn unsafe_catalog() -> RemoteError {
    RemoteError::new(
        "REMOTE_SELECTOR_UNAVAILABLE",
        "The remotes could not be read.",
    )
    .with_hint("Run tmt remote ls")
}
pub fn read(layout: &Layout) -> Result<Vec<RemoteReference>, RemoteError> {
    let Some(file) = layout.read_file(FILE)? else {
        return Ok(vec![]);
    };
    let mut bytes = vec![];
    file.take((BYTES + 1) as u64).read_to_end(&mut bytes)?;
    decode(&bytes)
}
fn decode(bytes: &[u8]) -> Result<Vec<RemoteReference>, RemoteError> {
    if bytes.len() > BYTES {
        return Err(unsafe_catalog());
    }
    let value = wire::strict_json(bytes).ok_or_else(unsafe_catalog)?;
    let document: Catalog = serde_json::from_value(value).map_err(|_| unsafe_catalog())?;
    let mut names = std::collections::BTreeSet::new();
    if document.version != 1
        || document
            .entries
            .iter()
            .any(|entry| !names.insert(&entry.name) || validate(entry).is_err())
    {
        return Err(unsafe_catalog());
    }
    Ok(document.entries)
}
fn validate(entry: &RemoteReference) -> Result<(), RemoteError> {
    if !name_ok(&entry.name) {
        return Err(unsafe_catalog());
    }
    if entry.kind == "firestore" {
        let project = crate::remote_link::project(&entry.url)?;
        if entry.project.as_deref() != Some(project.as_str()) {
            return Err(unsafe_catalog());
        }
    } else if entry.kind == "local" {
        let rest = entry
            .url
            .strip_prefix("http://")
            .or_else(|| entry.url.strip_prefix("https://"))
            .ok_or_else(unsafe_catalog)?;
        let (authority, path) = rest.split_once('/').ok_or_else(unsafe_catalog)?;
        if authority.is_empty()
            || !authority
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b':' | b'-'))
            || !path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_'))
            || !entry.url.ends_with('/')
            || entry.project.is_some()
            || entry.url.len() > 2048
        {
            return Err(unsafe_catalog());
        }
    } else {
        return Err(unsafe_catalog());
    }
    Ok(())
}
pub fn add(layout: &Layout, name: &str, url: &str) -> Result<RemoteReference, RemoteError> {
    let project = crate::remote_link::project(url).ok();
    let entry = RemoteReference {
        name: name.into(),
        kind: if project.is_some() {
            "firestore"
        } else {
            "local"
        }
        .into(),
        url: url.into(),
        project,
    };
    validate(&entry).map_err(|_| {
        RemoteError::new(
            "REMOTE_SELECTOR_INVALID",
            "The remote name or link is invalid",
        )
        .with_hint("Run tmt remote add <name> <remote-link>")
    })?;
    if crate::deploy_record::projects(layout)?
        .iter()
        .any(|project| project == name && entry.project.as_ref() != Some(project))
    {
        return Err(RemoteError::new(
            "REMOTE_SELECTOR_INVALID",
            "This remote name is already a different project's id.",
        )
        .with_hint("Run tmt remote ls"));
    }
    update(
        layout,
        &format!("tmt remote add {name} {url} --json"),
        |entries| {
            if entries.iter().any(|e| e.name == name) {
                return Err(RemoteError::new(
                    "REMOTE_SELECTOR_INVALID",
                    "A remote already has that name.",
                )
                .with_hint("Run tmt remote ls"));
            }
            entries.push(entry.clone());
            Ok(())
        },
    )?;
    Ok(entry)
}
pub fn remove(layout: &Layout, name: &str) -> Result<(), RemoteError> {
    update(layout, &format!("tmt remote rm {name} --json"), |entries| {
        let index = entries
            .iter()
            .position(|e| e.name == name)
            .ok_or_else(|| crate::remote_selector::missing(name))?;
        entries.remove(index);
        Ok(())
    })
}
fn update(
    layout: &Layout,
    retry: &str,
    change: impl FnOnce(&mut Vec<RemoteReference>) -> Result<(), RemoteError>,
) -> Result<(), RemoteError> {
    let document = tmt_extension_state::Document::new(
        &layout.directory,
        &layout.directory.join(FILE),
        BYTES as u64,
        retry,
    )
    .map_err(document_error)?;
    document_result(document.update(|current| {
        // Document's general permission policy is broader than Remote's private 0600 files.
        // Re-admit under the document lock before validating/changing its current bytes.
        if layout.read_file(FILE)?.is_some() != current.is_some() {
            return Err(unsafe_catalog());
        }
        let mut entries = current.map(decode).transpose()?.unwrap_or_default();
        change(&mut entries)?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let bytes = serde_json::to_vec(&Catalog {
            version: 1,
            entries,
        })
        .map_err(|_| unsafe_catalog())?;
        if bytes.len() > BYTES {
            return Err(unsafe_catalog());
        }
        Ok(bytes)
    }))
}

#[cfg(test)]
mod browser_fixture_tests {
    use super::*;
    #[test]
    fn browser_catalog_fixture_matches_native_public_reference_fields_and_names() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/browser-remotes.json")).unwrap();
        let catalog: Catalog = serde_json::from_value(fixture["catalog"].clone()).unwrap();
        for entry in &catalog.entries {
            validate(entry).unwrap();
        }
        assert_eq!(serde_json::to_value(catalog).unwrap(), fixture["catalog"]);
        for name in fixture["validNames"].as_array().unwrap() {
            assert!(name_ok(name.as_str().unwrap()));
        }
        for name in fixture["invalidNames"].as_array().unwrap() {
            assert!(!name_ok(name.as_str().unwrap()));
        }
        assert_eq!(fixture["maxNameBytes"], 63);
        assert!(name_ok(&"a".repeat(63)));
        assert!(!name_ok(&"a".repeat(64)));
    }
}
