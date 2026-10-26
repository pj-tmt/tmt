//! The only core access: fixed public API calls through the invoking executable.
use crate::Result;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
use tmt_invoke::Request;

pub fn data_root(stop: &AtomicBool) -> Result<PathBuf> {
    let executable = tmt_invoke::invoking_tmt()?;
    let args = ["api".into()];
    let input = serde_json::to_vec(&json!({"version":1,"operation":"storage.root","input":{}}))?;
    let output = tmt_invoke::invoke(
        Request {
            program: &executable,
            args: &args,
            input: &input,
            deadline: Instant::now() + Duration::from_secs(5),
            max_stream_bytes: 64 * 1024,
            launch: Default::default(),
        },
        Some(stop),
    )?;
    if !output.status.success() {
        return Err(
            "Core storage.root is unavailable; use a core supporting that operation.".into(),
        );
    }
    root_from_reply(&output.stdout)
}

pub fn root_from_reply(bytes: &[u8]) -> Result<PathBuf> {
    let value: Value = serde_json::from_slice(bytes)?;
    let root = value["dataRoot"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Core storage.root did not report dataRoot.")?;
    let root = PathBuf::from(root);
    if !root.is_absolute() {
        return Err(crate::keyring::StateFault::RootNotAbsolute.into());
    }
    Ok(root)
}

/// One best-effort public caller snapshot: a display-only name and an optional
/// canonical ID for the creation preference. Neither grants identity or authority.
#[derive(Clone, Debug)]
pub struct CallerSnapshot {
    pub name: String,
    pub agent_id: Option<String>,
}
pub fn publisher_agent() -> Option<String> {
    caller_snapshot().map(|caller| caller.name)
}
pub fn caller_snapshot() -> Option<CallerSnapshot> {
    let output = identity_reply(None)?;
    output
        .status
        .success()
        .then(|| caller_from_reply(&output.stdout))
        .flatten()
}

/// A selected reference is descriptive, never publication authority. Only an explicit public
/// not-found reply establishes absence; timeouts and malformed replies stay unverified.
pub enum IdentityLookup {
    Found(CallerSnapshot),
    NotFound,
    Unavailable,
}
pub fn identity_lookup(selector: &str) -> IdentityLookup {
    let Some(output) = identity_reply(Some(selector)) else {
        return IdentityLookup::Unavailable;
    };
    if output.status.success() {
        return caller_from_reply(&output.stdout)
            .filter(|caller| caller.agent_id.is_some())
            .filter(|caller| {
                tmt_colab_model::values::core_id(selector).is_err()
                    || caller.agent_id.as_deref() == Some(selector)
            })
            .map(IdentityLookup::Found)
            .unwrap_or(IdentityLookup::Unavailable);
    }
    let value: Value = serde_json::from_slice(&output.stdout).unwrap_or(Value::Null);
    match value["error"]["code"].as_str() {
        Some("NAME_NOT_FOUND" | "IDENTITY_NOT_FOUND") => IdentityLookup::NotFound,
        _ => IdentityLookup::Unavailable,
    }
}
fn identity_reply(selector: Option<&str>) -> Option<tmt_invoke::Output> {
    let executable = tmt_invoke::invoking_tmt().ok()?;
    let mut args = vec!["identity".into(), "show".into(), "--json".into()];
    if let Some(selector) = selector {
        args.extend(["--".into(), selector.into()]);
    }
    tmt_invoke::invoke(
        Request {
            program: &executable,
            args: &args,
            input: &[],
            deadline: Instant::now() + Duration::from_secs(1),
            max_stream_bytes: 64 * 1024,
            launch: Default::default(),
        },
        None,
    )
    .ok()
}
fn caller_from_reply(bytes: &[u8]) -> Option<CallerSnapshot> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let name = value.get("identity")?.get("name")?.as_str()?;
    crate::decoder::valid_publisher_agent(name).then(|| CallerSnapshot {
        name: name.to_owned(),
        agent_id: value["identity"]["id"]
            .as_str()
            .filter(|id| tmt_colab_model::values::core_id(id).is_ok())
            .map(str::to_owned),
    })
}

/// One optional public batch lookup for registered references. Failed or incomplete discovery is
/// unverified, not a not-found result; it cannot by itself refuse a source write.
pub fn reference_names(ids: &[String]) -> Option<crate::question_registry::Identities> {
    if ids.is_empty()
        || ids.len() > 256
        || ids
            .iter()
            .any(|id| tmt_colab_model::values::core_id(id).is_err())
    {
        return None;
    }
    let executable = tmt_invoke::invoking_tmt().ok()?;
    let input = serde_json::to_vec(
        &json!({"version":1,"operation":"references.resolve","input":{"identityIds":ids}}),
    )
    .ok()?;
    let output = tmt_invoke::invoke(
        Request {
            program: &executable,
            args: &["api".into()],
            input: &input,
            deadline: Instant::now() + Duration::from_secs(1),
            max_stream_bytes: 64 * 1024,
            launch: Default::default(),
        },
        None,
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    references_from_reply(&output.stdout, ids)
}

fn references_from_reply(
    bytes: &[u8],
    ids: &[String],
) -> Option<crate::question_registry::Identities> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let rows = value["identities"].as_array()?;
    if rows.len() != ids.len() {
        return None;
    }
    let mut known = crate::question_registry::Identities::new();
    for (row, id) in rows.iter().zip(ids) {
        if row["id"].as_str()? != id {
            return None;
        }
        let name = if row["found"].as_bool()? {
            let name = row["name"].as_str()?;
            if !crate::decoder::valid_publisher_agent(name) {
                return None;
            }
            Some(name.to_owned())
        } else {
            None
        };
        if known.insert(id.clone(), name).is_some() {
            return None;
        }
    }
    Some(known)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_lookup_distinguishes_not_found_from_incomplete_or_invalid_observation() {
        let id = "50000000-0000-1000-8000-000000000001".to_owned();
        let ids = vec![id.clone()];
        let found =
            serde_json::to_vec(&json!({"identities":[{"id":id,"found":true,"name":"agent"}]}))
                .unwrap();
        assert_eq!(
            references_from_reply(&found, &ids).unwrap()[&id].as_deref(),
            Some("agent")
        );
        let missing = serde_json::to_vec(&json!({"identities":[{"id":id,"found":false}]})).unwrap();
        assert_eq!(references_from_reply(&missing, &ids).unwrap()[&id], None);
        for reply in [
            b"{}".as_slice(),
            br#"{"identities":[]}"#,
            br#"{"identities":[{"id":"wrong","found":false}]}"#,
        ] {
            assert!(references_from_reply(reply, &ids).is_none());
        }
    }
    fn publisher_from_reply(bytes: &[u8]) -> Option<String> {
        caller_from_reply(bytes).map(|caller| caller.name)
    }
    #[test]
    fn caller_snapshot_keeps_the_canonical_id_separate_from_its_display_name() {
        let valid = caller_from_reply(
            br#"{"identity":{"name":"renamed-agent","id":"50000000-0000-1000-8000-000000000001"}}"#,
        )
        .unwrap();
        assert_eq!(valid.name, "renamed-agent");
        assert_eq!(
            valid.agent_id.as_deref(),
            Some("50000000-0000-1000-8000-000000000001")
        );
        for id in [
            "",
            "not-an-id",
            "00000000-0000-0000-0000-000000000000",
            "ABCDEF00-0000-1000-8000-000000000001",
        ] {
            let caller = caller_from_reply(
                &serde_json::to_vec(&json!({"identity":{"name":"display","id":id}})).unwrap(),
            )
            .unwrap();
            assert_eq!(caller.name, "display");
            assert_eq!(caller.agent_id, None);
        }
    }
    #[test]
    fn publisher_reply_is_optional_and_bounded() {
        assert_eq!(
            publisher_from_reply(br#"{"identity":{"name":"publisher"}}"#),
            Some("publisher".into())
        );
        for reply in [
            b"{}".as_slice(),
            br#"{"error":{"code":"IDENTITY_REQUIRED"}}"#,
            br#"{"identity":{"name":null}}"#,
            br#"{"identity":{"name":""}}"#,
            br#"{"identity":{"name":"bad\nname"}}"#,
            b"not json",
        ] {
            assert_eq!(publisher_from_reply(reply), None);
        }
        assert_eq!(
            publisher_from_reply(
                &serde_json::to_vec(&json!({"identity":{"name":"x".repeat(129)}})).unwrap()
            ),
            None
        );
    }
}
