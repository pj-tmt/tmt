//! Frozen field/query names are extension declaration data, never provider projections.
use super::{DeclarationError, Reason, Result, field_name, members, refuse};
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
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncQueryMapping {
    pub scope_fields: Vec<String>,
    pub stream_field: String,
    pub seq_field: String,
    pub order: String,
}
pub(super) fn parse(value: &Value) -> Result<SyncMapping> {
    let row = members(value, &["fields", "spaceField", "uidField", "query"])?;
    if !row["query"].is_null() {
        members(
            &row["query"],
            &["scopeFields", "streamField", "seqField", "order"],
        )?;
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
            || !scope.contains(&mapping.space_field)
            || !scope.contains(&query.stream_field)
            || scope.contains(&query.seq_field)
        {
            return refuse(Reason::SyncMapping);
        }
    }
    Ok(mapping)
}
