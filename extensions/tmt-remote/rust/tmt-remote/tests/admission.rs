//! Real private storage and strict signatures; no core effects are enabled in A.
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tmt_remote::{
    admission::{BindingAction, MessagePermit, MessageRefusal},
    authority::{DispatchMode, PermittedAgents},
    canonical::{self, Envelope},
    crypto,
    pairing::now_ms,
    session::DoorSessions,
    state::{Layout, MachineKey, Serving},
    store::{DEFAULT_SCOPES, Grant, Store, uuid_v4},
    transport::{LoopbackTransport, Transport},
    wire::{SignedMessage, strict_json},
};
struct AdmissionRoot(PathBuf);
impl AdmissionRoot {
    fn new() -> Self {
        Self(PathBuf::from(format!("/tmp/t1055-{}", uuid_v4().unwrap())))
    }
}
impl Drop for AdmissionRoot {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
struct OwnerDoor {
    sessions: Arc<DoorSessions>,
    store: Arc<Mutex<Store>>,
    key: SigningKey,
    grant: Grant,
    machine: String,
    window: String,
    public: [u8; 32],
    _serving: Serving,
    _root: AdmissionRoot,
}
impl OwnerDoor {
    fn new() -> Self {
        Self::with_scopes(DEFAULT_SCOPES.iter().map(|scope| (*scope).into()).collect())
    }
    fn with_scopes(scopes: Vec<String>) -> Self {
        Self::with_policy(scopes, "direct")
    }
    fn with_policy(scopes: Vec<String>, mode: &str) -> Self {
        Self::with_limits(scopes, mode, "all".into(), None)
    }
    fn with_limits(scopes: Vec<String>, mode: &str, agents: String, expiry: Option<u64>) -> Self {
        let root = AdmissionRoot::new();
        let layout = Layout::open(&root.0).unwrap();
        let serving = layout.serve_lock().unwrap();
        let machine_key = MachineKey::open(&layout).unwrap();
        let public = machine_key.public();
        let mut store = Store::open(&serving).unwrap();
        let machine = store.machine().unwrap();
        let window = uuid_v4().unwrap();
        let key = SigningKey::from_bytes(&[47; 32]);
        let grant = Grant {
            client_id: uuid_v4().unwrap(),
            public_key: key.verifying_key().to_bytes(),
            kind: "cli".into(),
            origin: "cli".into(),
            name: "Test device".into(),
            agents,
            scopes,
            mode: mode.into(),
            issued_at_ms: now_ms().unwrap(),
            expires_at_ms: expiry,
            revision: 1,
            disabled: false,
        };
        store.insert_grant(&grant).unwrap();
        let store = Arc::new(Mutex::new(store));
        let sessions = Arc::new(DoorSessions::new(
            machine.id.clone(),
            window.clone(),
            "http://127.0.0.1:32100".into(),
            format!("{}/x/", machine.route_prefix),
            machine_key,
            Arc::clone(&store),
            Duration::from_secs(43200),
        ));
        Self {
            sessions,
            store,
            key,
            grant,
            machine: machine.id,
            window,
            public,
            _serving: serving,
            _root: root,
        }
    }
    fn wire(&self, session: &str, sequence: &str, operation: &str, payload: &[u8]) -> Value {
        let id = uuid_v4().unwrap();
        let now = now_ms().unwrap();
        let kind = if matches!(operation, "session.open" | "subscribe" | "ack") {
            "control"
        } else {
            "request"
        };
        let mut wire = json!({"version":1,"profile":"local-v1","kind":kind,"id":id,"correlationId":null,"machineId":self.machine,"windowId":self.window,"clientId":self.grant.client_id,"sessionId":session,"sequence":sequence,"timestampMs":now,"origin":"cli","operation":operation,"payload":canonical::base64url(payload)});
        self.resign(&mut wire);
        wire
    }
    fn resign(&self, wire: &mut Value) {
        let payload = canonical::base64url_decode(wire["payload"].as_str().unwrap()).unwrap();
        let signature = self
            .key
            .sign(&canonical::envelope(&test_envelope(wire, &payload)).unwrap());
        wire["signature"] = json!(canonical::base64url(&signature.to_bytes()));
    }
    fn open(&self) -> String {
        let payload = json!({"clientNonce":uuid_v4().unwrap().replace('-',"")}).to_string();
        let wire = self.wire("new", "0", "session.open", payload.as_bytes());
        let opened = self
            .sessions
            .open(None, &serde_json::to_vec(&wire).unwrap())
            .unwrap();
        assert!(opened.cookie.is_none());
        let reply: Value = serde_json::from_slice(&opened.response).unwrap();
        self.verify_reply(&reply, &wire);
        assert_eq!(reply["sequence"], "1");
        reply["sessionId"].as_str().unwrap().into()
    }
    fn verify_reply(&self, reply: &Value, request: &Value) -> Value {
        let text = |key: &str| reply[key].as_str().unwrap();
        assert_eq!(reply["correlationId"], request["id"]);
        assert_eq!(reply["operation"], request["operation"]);
        assert_eq!(reply["clientId"], request["clientId"]);
        assert_eq!(reply["machineId"], self.machine);
        let payload = canonical::base64url_decode(text("payload")).unwrap();
        crypto::verify_signature(
            &self.public,
            &canonical::envelope(&test_envelope(reply, &payload)).unwrap(),
            &canonical::base64url_bytes(text("signature"), 64).unwrap(),
        )
        .unwrap();
        strict_json(&payload).unwrap()
    }
    fn admit(&self, wire: &Value) -> Result<MessagePermit, MessageRefusal> {
        self.sessions.admit(
            BindingAction::Append,
            None,
            &serde_json::to_vec(wire).unwrap(),
            1024,
        )
    }
}
// The fixture constructs signing bytes independently of production wire decoding.
fn test_envelope<'a>(wire: &'a Value, payload: &'a [u8]) -> Envelope<'a> {
    let text = |key: &str| wire[key].as_str().unwrap();
    Envelope {
        kind: text("kind"),
        id: text("id"),
        correlation_id: wire["correlationId"].as_str(),
        machine_id: text("machineId"),
        window_id: text("windowId"),
        client_id: text("clientId"),
        session_id: text("sessionId"),
        sequence: text("sequence"),
        timestamp_ms: wire["timestampMs"].as_u64().unwrap(),
        origin: text("origin"),
        operation: text("operation"),
        payload,
    }
}
fn signed_code(result: Result<MessagePermit, MessageRefusal>) -> String {
    let response = match result {
        Err(MessageRefusal::Signed(response)) => response,
        _ => panic!("expected signed refusal"),
    };
    let reply: Value = serde_json::from_slice(&response).unwrap();
    let payload = canonical::base64url_decode(reply["payload"].as_str().unwrap()).unwrap();
    strict_json(&payload).unwrap()["error"]["code"]
        .as_str()
        .unwrap()
        .into()
}
#[test]
fn exact_payload_and_duplicate_member_admission() {
    let owner = OwnerDoor::new();
    let session = owner.open();
    let payload = br#"{ "note": "exact whitespace" }"#;
    let wire = owner.wire(&session, "1", "capabilities", payload);
    let bytes = serde_json::to_vec(&wire).unwrap();
    assert_eq!(
        SignedMessage::decode(&bytes, 1024).unwrap().payload,
        payload
    );
    let text = String::from_utf8(bytes).unwrap();
    let duplicate = text.replacen("{", "{\"version\":1,", 1);
    assert!(SignedMessage::decode(duplicate.as_bytes(), 1024).is_none());
    let missing = text.replace("\"correlationId\":null,", "");
    assert!(SignedMessage::decode(missing.as_bytes(), 1024).is_none());
    for bad in [
        br#"{"a":1,"a":2}"#.as_slice(),
        br#"{"nested":[{"a":1,"\u0061":2}]}"#,
        br#"{"text":"\ud800"}"#,
        b"{} {}",
        b"\xef\xbb\xbf{}",
    ] {
        let wire = owner.wire(&session, "1", "capabilities", bad);
        assert!(SignedMessage::decode(&serde_json::to_vec(&wire).unwrap(), 1024).is_none());
    }
    assert!(SignedMessage::decode(text.as_bytes(), payload.len() - 1).is_none());
    let mut unknown = wire.clone();
    unknown["identity"] = json!("caller");
    assert!(SignedMessage::decode(&serde_json::to_vec(&unknown).unwrap(), 1024).is_none());
}
#[test]
fn closed_transport_consumes_once_and_machine_responses_increase() {
    let owner = OwnerDoor::new();
    let session = owner.open();
    let transport = LoopbackTransport::new(Arc::clone(&owner.sessions), 1024);
    let first = owner.wire(&session, "1", "capabilities", b"{}");
    let send = |wire: &Value| -> Value {
        serde_json::from_slice(
            &transport
                .append(None, &serde_json::to_vec(wire).unwrap())
                .unwrap(),
        )
        .unwrap()
    };
    let response = send(&first);
    assert_eq!(response["sequence"], "2");
    assert_eq!(
        owner.verify_reply(&response, &first)["error"]["code"],
        "REMOTE_CLOSED"
    );
    let replay = send(&first);
    assert_eq!(replay["sequence"], "3");
    assert_eq!(
        owner.verify_reply(&replay, &first)["error"]["code"],
        "REMOTE_REPLAY"
    );
    let next = owner.wire(&session, "2", "agents.list", b"{}");
    let response = send(&next);
    assert_eq!(response["sequence"], "4");
    assert_eq!(
        owner.verify_reply(&response, &next)["error"]["code"],
        "REMOTE_CLOSED"
    );
    assert!(
        !owner._root.0.join("tmux-team.db").exists(),
        "A must not create core state"
    );
}
#[test]
fn bad_signature_audience_time_and_origin_do_not_spend_sequence() {
    let owner = OwnerDoor::new();
    let session = owner.open();
    let good = owner.wire(&session, "1", "capabilities", b"{}");
    for field in [
        "signature",
        "payload",
        "machineId",
        "windowId",
        "origin",
        "timestampMs",
    ] {
        let mut wire = good.clone();
        wire[field] = match field {
            "signature" => json!(canonical::base64url(&[0; 64])),
            "payload" => json!(canonical::base64url(b"{ }")),
            "origin" => json!("http://127.0.0.1:32100"),
            "timestampMs" => json!(1),
            _ => json!(uuid_v4().unwrap()),
        };
        if !matches!(field, "signature" | "payload") {
            owner.resign(&mut wire);
        }
        assert!(
            matches!(owner.admit(&wire), Err(MessageRefusal::Unauthenticated)),
            "{field}"
        );
    }
    assert!(matches!(
        owner.sessions.admit(
            BindingAction::Append,
            Some("https://evil.example"),
            &serde_json::to_vec(&good).unwrap(),
            1024
        ),
        Err(MessageRefusal::Unauthenticated)
    ));
    assert!(owner.admit(&good).is_ok());
}
#[test]
fn single_in_flight_and_out_of_order_refuse_without_spending_next() {
    let owner = OwnerDoor::new();
    let session = owner.open();
    assert_eq!(
        signed_code(owner.admit(&owner.wire(&session, "2", "capabilities", b"{}"))),
        "REMOTE_REPLAY"
    );
    let first = owner
        .admit(&owner.wire(&session, "1", "capabilities", b"{}"))
        .ok()
        .unwrap();
    let next = owner.wire(&session, "2", "capabilities", b"{}");
    assert_eq!(signed_code(owner.admit(&next)), "REMOTE_REPLAY");
    first.revalidate().unwrap();
    drop(first);
    assert!(owner.admit(&next).is_ok());
}
#[test]
fn new_session_and_revocation_end_already_admitted_authority() {
    let owner = OwnerDoor::new();
    let old = owner.open();
    let permit = owner
        .admit(&owner.wire(&old, "1", "capabilities", b"{}"))
        .ok()
        .unwrap();
    let new = owner.open();
    assert_eq!(permit.revalidate().unwrap_err().code, "REMOTE_CLOSED");
    drop(permit);
    assert!(matches!(
        owner.admit(&owner.wire(&old, "2", "capabilities", b"{}")),
        Err(MessageRefusal::Unauthenticated)
    ));
    let permit = owner
        .admit(&owner.wire(&new, "1", "capabilities", b"{}"))
        .ok()
        .unwrap();
    owner
        .store
        .lock()
        .unwrap()
        .revoke(&owner.grant.client_id)
        .unwrap();
    assert_eq!(permit.revalidate().unwrap_err().code, "REMOTE_CLOSED");
    assert!(matches!(
        owner.admit(&owner.wire(&new, "2", "capabilities", b"{}")),
        Err(MessageRefusal::Unauthenticated)
    ));
}
#[test]
fn typed_grant_refuses_malformed_expanded_or_expired_authority() {
    let owner = OwnerDoor::new();
    let mut grant = owner.grant.clone();
    let agent = "00000000-0000-7000-0000-000000000001";
    grant.agents = json!([agent]).to_string();
    grant.mode = "hold".into();
    grant.expires_at_ms = Some(100);
    let authority = grant.authority().unwrap();
    assert_eq!(authority.mode, DispatchMode::Hold);
    assert_eq!(
        authority.agents,
        PermittedAgents::Selected(vec![agent.into()])
    );
    assert!(authority.permits(agent));
    assert!(!authority.permits("00000000-0000-7000-0000-000000000002"));
    assert!(grant.live_at(99));
    assert!(!grant.live_at(100));
    for agents in [
        json!([agent, agent]).to_string(),
        json!(["00000000-0000-0000-0000-000000000000"]).to_string(),
        json!(["bad"]).to_string(),
        "future".into(),
    ] {
        grant.agents = agents;
        assert!(grant.authority().is_err());
    }
    grant = owner.grant.clone();
    grant.scopes.push("run".into());
    assert!(grant.authority().is_err());
    grant = owner.grant.clone();
    grant.scopes.swap(0, 1);
    assert!(grant.authority().is_err());
}
#[test]
fn session_counters_survive_reopen_and_exhaustion_never_wraps() {
    let owner = OwnerDoor::new();
    let grant = owner.grant.clone();
    let root = AdmissionRoot::new();
    let layout = Layout::open(&root.0).unwrap();
    let serving = layout.serve_lock().unwrap();
    let session = uuid_v4().unwrap();
    let window = uuid_v4().unwrap();
    let now = now_ms().unwrap();
    let consume = |store: &mut Store, sequence| {
        store.consume_sequence(&grant.client_id, &session, &window, sequence, now)
    };
    let response = |store: &mut Store| store.response_sequence(&grant.client_id, &session, &window);
    {
        let mut store = Store::open(&serving).unwrap();
        store.insert_grant(&grant).unwrap();
        store.start_session(&grant, &session, &window, now).unwrap();
        consume(&mut store, 1).unwrap();
        assert_eq!(response(&mut store).unwrap(), 2);
    }
    let mut store = Store::open(&serving).unwrap();
    assert_eq!(consume(&mut store, 1).unwrap_err().code, "REMOTE_REPLAY");
    consume(&mut store, 2).unwrap();
    assert_eq!(response(&mut store).unwrap(), 3);
    assert_eq!(
        consume(&mut store, u64::MAX).unwrap_err().code,
        "REMOTE_REPLAY"
    );
    consume(&mut store, 3).unwrap();
    drop(store);
    // Test-only state preparation independently reaches the u64 boundary.
    // Store remains the sole product opener, and the oracle closes before reopen.
    let oracle = rusqlite::Connection::open(layout.directory.join("remote.db")).unwrap();
    oracle
        .execute(
            "UPDATE sessions SET next_client_sequence=?1,next_server_sequence=?1",
            [u64::MAX.to_string()],
        )
        .unwrap();
    drop(oracle);
    let mut store = Store::open(&serving).unwrap();
    consume(&mut store, u64::MAX).unwrap();
    assert_eq!(
        consume(&mut store, u64::MAX).unwrap_err().code,
        "REMOTE_REPLAY"
    );
    assert_eq!(consume(&mut store, 1).unwrap_err().code, "REMOTE_REPLAY");
    assert_eq!(response(&mut store).unwrap(), u64::MAX);
    assert_eq!(response(&mut store).unwrap_err().code, "REMOTE_CLOSED");
}

#[test]
fn concurrent_duplicate_has_one_admission_winner() {
    let owner = OwnerDoor::new();
    let session = owner.open();
    let wire = serde_json::to_vec(&owner.wire(&session, "1", "capabilities", b"{}")).unwrap();
    let ready = Arc::new(std::sync::Barrier::new(3));
    let finished = Arc::new(std::sync::Barrier::new(3));
    let workers = (0..2)
        .map(|_| {
            let sessions = Arc::clone(&owner.sessions);
            let wire = wire.clone();
            let ready = Arc::clone(&ready);
            let finished = Arc::clone(&finished);
            std::thread::spawn(move || {
                ready.wait();
                let result = sessions.admit(BindingAction::Append, None, &wire, 1024);
                let accepted = result.is_ok();
                finished.wait();
                drop(result);
                accepted
            })
        })
        .collect::<Vec<_>>();
    ready.wait();
    finished.wait();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|accepted| *accepted)
            .count(),
        1
    );
    assert_eq!(
        signed_code(owner.admit(&owner.wire(&session, "1", "capabilities", b"{}"))),
        "REMOTE_REPLAY"
    );
}

#[test]
fn unsupported_operations_and_route_mismatch_leave_client_sequence_unchanged() {
    let owner = OwnerDoor::new();
    let session = owner.open();
    let unsupported = owner.wire(&session, "1", "run", b"{}");
    assert_eq!(
        signed_code(owner.admit(&unsupported)),
        "REMOTE_INPUT_INVALID"
    );
    let good = owner.wire(&session, "1", "capabilities", b"{}");
    assert_eq!(
        signed_code(owner.sessions.admit(
            BindingAction::Subscribe,
            None,
            &serde_json::to_vec(&good).unwrap(),
            1024
        )),
        "REMOTE_INPUT_INVALID"
    );
    assert!(owner.admit(&good).is_ok());
}

#[test]
fn removed_or_empty_scopes_refuse_without_spending_sequence() {
    let narrowed = DEFAULT_SCOPES
        .iter()
        .filter(|scope| **scope != "agents.read")
        .map(|scope| (*scope).into())
        .collect();
    for scopes in [narrowed, Vec::new()] {
        let owner = OwnerDoor::with_scopes(scopes);
        let session = owner.open();
        assert_eq!(
            signed_code(owner.admit(&owner.wire(&session, "1", "agents.list", b"{}"))),
            "REMOTE_SCOPE_DENIED"
        );
        assert!(
            owner
                .admit(&owner.wire(&session, "1", "capabilities", b"{}"))
                .is_ok()
        );
    }
}
#[test]
fn strict_payload_admission_preserves_native_json_number_values() {
    let value = strict_json(br#"{"integer":18446744073709551615,"fraction":1.5}"#).unwrap();
    assert_eq!(value["integer"].as_u64(), Some(u64::MAX));
    assert_eq!(value["fraction"].as_f64(), Some(1.5));
}

#[path = "admission/journal.rs"]
mod journal;

#[path = "admission/operations.rs"]
mod operations;
