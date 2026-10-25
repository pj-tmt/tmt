//! Where a built-in-host member's terminal stream is: a read-only lookup for a view such as
//! the Ops Terminal tab. It opens no connection and starts, wakes or types nothing; the
//! stream itself is `contracts/terminal-stream-v1.md`.
use super::{Fault, Request, invalid};
use crate::{
    delivery,
    host::{Host, OperationOptions},
    storage::Storage,
};
use serde::Deserialize;
use serde_json::json;
use tmt_core::{
    binding::{BindingEvidence, evaluate_binding},
    endpoint::ServerEvidence,
    host::HostKind,
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
    Ok(Request::TerminalStream {
        identity_id: input.identity_id,
    })
}

fn unavailable() -> Fault {
    Fault::new(
        "TERMINAL_STREAM_UNAVAILABLE",
        "The identity has no live member on the built-in host.",
    )
}

pub(super) fn stream(storage: &mut Storage, id: &str) -> Result<Vec<u8>, Fault> {
    let entry = delivery::current(storage, id)
        .map_err(|_| Fault::unavailable())?
        .ok_or_else(|| Fault::new("NAME_NOT_FOUND", "The selected identity was not found."))?;
    let binding = entry.binding.as_ref().ok_or_else(unavailable)?;
    if binding.server.host != HostKind::pty() {
        return Err(unavailable());
    }
    // Presence only, as for `pane.screen`: an ended agent still has a stream to read, but a
    // member whose supervisor is gone does not.
    let probe = Host::for_server(&binding.server)
        .probe(
            &binding.server,
            OperationOptions {
                deadline: None,
                pane_ids: Some(std::slice::from_ref(&binding.pane_id)),
            },
        )
        .map_err(|_| Fault::unavailable())?;
    match evaluate_binding(&entry, &probe) {
        BindingEvidence::Active(_) => resource(id, &binding.server, &binding.pane_id),
        BindingEvidence::EndpointLost
        | BindingEvidence::MarkerMismatch
        | BindingEvidence::Unknown => Err(unavailable()),
    }
}

fn resource(id: &str, server: &ServerEvidence, pane: &str) -> Result<Vec<u8>, Fault> {
    serde_json::to_vec(&json!({
        "identityId": id,
        "host": server.host.as_str(),
        "socket": server.socket_path,
        "pane": pane,
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

    #[test]
    fn requests_admit_only_an_exact_canonical_uuid() {
        let id = "11111111-1111-4111-8111-111111111111";
        let request = serde_json::json!({"identityId": id}).to_string();
        assert!(matches!(
            decode(request.as_bytes()).unwrap(),
            Request::TerminalStream { identity_id } if identity_id == id
        ));
        for input in [
            serde_json::json!({}),
            serde_json::json!({"identityId": "Worker"}),
            serde_json::json!({"identityId": id, "name": "Worker"}),
            serde_json::json!({"identityId": 7}),
        ] {
            assert_eq!(
                decode(input.to_string().as_bytes()).err().unwrap().code(),
                "API_INPUT_INVALID",
                "{input}"
            );
        }
    }

    #[test]
    fn the_resource_names_the_socket_and_pane_and_nothing_else() {
        let server = ServerEvidence {
            host: HostKind::pty(),
            server_id: "server".into(),
            socket_path: "/data/pty/pty_a1.sock".into(),
            server_pid: 7,
            server_start_time: "start".into(),
        };
        let body = resource("11111111-1111-4111-8111-111111111111", &server, "pty_a1").unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            serde_json::json!({
                "identityId": "11111111-1111-4111-8111-111111111111",
                "host": "pty",
                "socket": "/data/pty/pty_a1.sock",
                "pane": "pty_a1",
            })
        );
    }

    #[test]
    fn an_unknown_or_retired_identity_is_not_found_and_an_unbound_one_is_unavailable() {
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
                stream(&mut storage, id).unwrap_err().code(),
                "NAME_NOT_FOUND"
            );
        }
        assert_eq!(
            stream(&mut storage, &unbound.id).unwrap_err().code(),
            "TERMINAL_STREAM_UNAVAILABLE"
        );
    }
}
