//! Exact-identity visible pane screen: a read-only diagnostic view, never a
//! reply, completion evidence or request attribution.
use super::{Fault, Request, invalid};
use crate::{
    delivery,
    host::{Host, OperationOptions, SCREEN_MAX_BYTES, ScreenFormat},
    storage::Storage,
};
use serde::Deserialize;
use serde_json::json;
use tmt_core::{
    binding::{BindingEvidence, evaluate_binding},
    request::RequestEndpoint,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    identity_id: String,
}

pub(super) fn decode(input: &[u8]) -> Result<Request, Fault> {
    let input: Input = serde_json::from_slice(input).map_err(|_| invalid())?;
    if !tmt_core::dispatch::canonical_id(&input.identity_id) {
        return Err(invalid());
    }
    Ok(Request::PaneScreen {
        identity_id: input.identity_id,
    })
}

pub(super) fn screen(storage: &mut Storage, id: &str) -> Result<Vec<u8>, Fault> {
    let entry = delivery::current(storage, id)
        .map_err(|_| Fault::unavailable())?
        .ok_or_else(|| Fault::new("NAME_NOT_FOUND", "The selected identity was not found."))?;
    let no_pane = || Fault::new("PANE_NOT_FOUND", "The identity has no live pane.");
    let Some(binding) = entry.binding.as_ref() else {
        return Err(no_pane());
    };
    let host = Host::for_server(&binding.server);
    // Presence only: an ended runtime still has a screen worth reading, and a
    // read never refreshes the session state.
    let probe = host
        .probe(
            &binding.server,
            OperationOptions {
                deadline: None,
                pane_ids: Some(std::slice::from_ref(&binding.pane_id)),
            },
        )
        .map_err(|_| Fault::unavailable())?;
    match evaluate_binding(&entry, &probe) {
        BindingEvidence::Active(_) => {}
        BindingEvidence::EndpointLost => return Err(no_pane()),
        BindingEvidence::MarkerMismatch | BindingEvidence::Unknown => {
            return Err(Fault::unavailable());
        }
    }
    let endpoint = RequestEndpoint {
        server: binding.server.clone(),
        pane_id: binding.pane_id.clone(),
        pane_pid: binding.pane_pid,
    };
    let (format, text) = host.screen(&endpoint).map_err(|_| Fault::unavailable())?;
    resource(id, binding.server.host.as_str(), format, &text)
}

/// Over the cap fails whole: a clipped screen would misdraw its last lines.
fn resource(id: &str, host: &str, format: ScreenFormat, text: &str) -> Result<Vec<u8>, Fault> {
    if text.len() > SCREEN_MAX_BYTES {
        return Err(Fault::unavailable());
    }
    serde_json::to_vec(&json!({
        "identityId": id,
        "host": host,
        "format": match format {
            ScreenFormat::Ansi => "ansi",
            ScreenFormat::Plain => "plain",
        },
        "text": text,
    }))
    .map_err(|_| Fault::unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDirectory;
    use serde_json::Value;
    use tmt_core::{
        binding::BindingRepository,
        identity::{Lifetime, create_or_resolve},
    };

    fn vector() -> Value {
        serde_json::from_str(include_str!("../../../../../contracts/pane-screen-v1.json")).unwrap()
    }

    #[test]
    fn requests_admit_only_an_exact_canonical_uuid() {
        let vector = vector();
        assert!(matches!(
            decode(vector["request"]["input"].to_string().as_bytes()).unwrap(),
            Request::PaneScreen { identity_id } if identity_id == vector["identityId"]
        ));
        for input in vector["invalidInputs"].as_array().unwrap() {
            assert_eq!(
                decode(input.to_string().as_bytes()).err().unwrap().code(),
                "API_INPUT_INVALID",
                "{input}"
            );
        }
    }

    #[test]
    fn resources_match_the_shared_vector_and_the_cap_fails_whole() {
        let vector = vector();
        let id = vector["identityId"].as_str().unwrap();
        for (host, format) in [("tmux", ScreenFormat::Ansi), ("herdr", ScreenFormat::Plain)] {
            let expected = vector["resources"]
                .as_object()
                .unwrap()
                .values()
                .find(|resource| resource["host"] == host)
                .unwrap();
            let body = resource(id, host, format, expected["text"].as_str().unwrap()).unwrap();
            assert_eq!(&serde_json::from_slice::<Value>(&body).unwrap(), expected);
        }
        let cap = usize::try_from(vector["limits"]["utf8TextBytes"].as_u64().unwrap()).unwrap();
        assert_eq!(cap, SCREEN_MAX_BYTES);
        assert!(resource(id, "tmux", ScreenFormat::Ansi, &"x".repeat(cap)).is_ok());
        let error = resource(id, "tmux", ScreenFormat::Ansi, &"x".repeat(cap + 1)).unwrap_err();
        assert_eq!(error.code(), vector["limits"]["overCapCode"]);
    }

    #[test]
    fn an_unknown_or_retired_identity_is_not_found_and_an_unbound_one_has_no_pane() {
        let directory = TestDirectory::new();
        let mut storage = Storage::open(directory.path.join("fixture.db")).unwrap();
        let unbound = create_or_resolve(&mut storage, "Unbound", Lifetime::Saved)
            .unwrap()
            .identity;
        let retired = create_or_resolve(&mut storage, "Retired", Lifetime::Saved)
            .unwrap()
            .identity;
        storage
            .with_binding_transaction(|rows| rows.retire_identity(&retired, false))
            .unwrap();
        let unknown = "22222222-2222-4222-8222-222222222222";
        for id in [unknown, retired.id.as_str()] {
            assert_eq!(
                screen(&mut storage, id).unwrap_err().code(),
                "NAME_NOT_FOUND"
            );
        }
        assert_eq!(
            screen(&mut storage, &unbound.id).unwrap_err().code(),
            "PANE_NOT_FOUND"
        );
    }
}
