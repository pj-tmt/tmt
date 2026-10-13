//! Frozen field/query names are extension declaration data, never provider projections.
use super::{DeclarationError, Reason, Result, field_name, members, optional_members, refuse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncMapping {
    pub fields: Vec<String>,
    pub space_field: String,
    pub uid_field: Option<String>,
    pub query: Option<SyncQueryMapping>,
    pub write_modes: Option<Vec<String>>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncQueryMapping {
    pub scope_fields: Vec<String>,
    pub stream_field: String,
    pub seq_field: String,
    pub order: String,
    pub discovery: Option<SyncDiscoveryMapping>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncDiscoveryMapping {
    pub name: String,
    pub scope_fields: Vec<String>,
    pub seq_equals: Option<String>,
}
pub(super) fn parse(value: &Value) -> Result<SyncMapping> {
    let row = optional_members(
        value,
        &["fields", "spaceField", "uidField", "query"],
        &["writeModes"],
    )?;
    if row.get("writeModes").is_some_and(|v| !v.is_array()) {
        return refuse(Reason::SyncMapping);
    }
    if !row["query"].is_null() {
        let query = optional_members(
            &row["query"],
            &["scopeFields", "streamField", "seqField", "order"],
            &["discovery"],
        )?;
        if let Some(discovery) = query.get("discovery") {
            members(discovery, &["name", "scopeFields", "seqEquals"])?;
        }
    }
    let mapping: SyncMapping =
        serde_json::from_value(value.clone()).map_err(|_| DeclarationError {
            resource: None,
            reason: Reason::SyncMapping,
        })?;
    let fields: BTreeSet<_> = mapping.fields.iter().collect();
    let declared = |name: &String| fields.contains(name) && field_name(name);
    if fields.is_empty()
        || fields.len() > 64
        || fields.len() != mapping.fields.len()
        || !mapping.fields.iter().all(|name| field_name(name))
        || !declared(&mapping.space_field)
        || mapping
            .uid_field
            .as_ref()
            .is_some_and(|name| !declared(name) || name == &mapping.space_field)
    {
        return refuse(Reason::SyncMapping);
    }
    if let Some(query) = &mapping.query {
        let scope: BTreeSet<_> = query.scope_fields.iter().collect();
        if scope.is_empty()
            || scope.len() > 8
            || scope.len() != query.scope_fields.len()
            || !query.scope_fields.iter().all(declared)
            || !declared(&query.stream_field)
            || !declared(&query.seq_field)
            || query.order != "asc"
            || query.stream_field == query.seq_field
            || query.stream_field == mapping.space_field
            || mapping
                .uid_field
                .as_ref()
                .is_some_and(|uid| uid == &query.stream_field || uid == &query.seq_field)
            || !scope.contains(&mapping.space_field)
            || !scope.contains(&query.stream_field)
            || scope.contains(&query.seq_field)
        {
            return refuse(Reason::SyncMapping);
        }
    }
    if let Some(modes) = &mapping.write_modes {
        let unique: BTreeSet<_> = modes.iter().collect();
        if modes.is_empty()
            || unique.len() != modes.len()
            || !modes
                .iter()
                .all(|s| ["create", "put", "delete"].contains(&s.as_str()))
        {
            return refuse(Reason::SyncMapping);
        }
    }
    if let Some(discovery) = mapping.query.as_ref().and_then(|q| q.discovery.as_ref()) {
        let query = mapping.query.as_ref().expect("discovery has query");
        let actual: BTreeSet<_> = discovery.scope_fields.iter().collect();
        let expected: BTreeSet<_> = query
            .scope_fields
            .iter()
            .filter(|name| *name != &query.stream_field)
            .collect();
        if !field_name(&discovery.name)
            || discovery.name == "stream"
            || actual != expected
            || actual.len() != discovery.scope_fields.len()
            || discovery.seq_equals.as_ref().is_some_and(|s| {
                s.parse::<u64>()
                    .map_or(true, |n| n > ((1 << 53) - 1) || n.to_string() != *s)
            })
        {
            return refuse(Reason::SyncMapping);
        }
    }
    Ok(mapping)
}
