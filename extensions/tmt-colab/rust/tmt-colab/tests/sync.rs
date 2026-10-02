//! Real duplex sockets with deterministic explicit server turns: no worker/sleep loops.
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tmt_colab::{
    keyring::Layout,
    store::{Store, StreamScope},
    sync::{Access, Admission, CatchupContext, Code, Connection, Progress, Server, SyncScope},
};
use tmt_colab_model::{crypto, object, values};
use tungstenite::{Message, WebSocket, protocol::Role};
const PAGE: &str = "00000000-0000-4000-8000-000000000001";
const ALICE: &str = "00000000-0000-4000-8000-000000000002";
const BOB: &str = "00000000-0000-4000-8000-000000000003";
struct Policy {
    space: String,
    devices: HashMap<String, [u8; 32]>,
    epoch: String,
    denied: Option<Code>,
    baseline: Option<Vec<u8>>,
    owner: [u8; 32],
}
impl Admission for Policy {
    fn catchup_context(
        &self,
        _: &str,
        scope: &SyncScope,
        store: &Store,
    ) -> Result<CatchupContext, Code> {
        Ok(CatchupContext {
            membership_head: store
                .owner_head(&scope.space, &self.owner)
                .map_err(|_| Code::Invalid)?
                .ok_or(Code::Denied)?,
            baseline: self.baseline.clone(),
        })
    }
    fn authorize(
        &self,
        principal: &str,
        scope: &SyncScope,
        access: Access<'_>,
    ) -> Result<[u8; 32], Code> {
        if let Some(code) = self.denied {
            return Err(code);
        }
        if scope.space != self.space || scope.page != PAGE {
            return Err(Code::Denied);
        }
        if scope.epoch != self.epoch {
            return Err(Code::StaleEpoch);
        }
        let key = *self.devices.get(principal).ok_or(Code::Denied)?;
        if let Access::Append(c) = access
            && (c.membership_revision != "1" || (principal == BOB && c.namespace == "content"))
        {
            return Err(Code::Denied);
        }
        Ok(key)
    }
}
struct Fixture {
    root: PathBuf,
    server: Server<Policy>,
    space: String,
    alice: SigningKey,
    bob: SigningKey,
}
type Peer = (WebSocket<UnixStream>, Connection<UnixStream, Policy>);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-1156-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let alice = SigningKey::from_bytes(&[1; 32]);
        let bob = SigningKey::from_bytes(&[2; 32]);
        let space = crypto::space_id(alice.verifying_key().as_bytes()).unwrap();
        let layout = Layout::open(&root).unwrap();
        let mut store = Store::open(&layout).unwrap();
        store.create_page(PAGE).unwrap();
        let initial = tmt_colab_model::statement::sign(&space,None,"member.add",&serde_json::to_vec(&json!({
            "memberId":ALICE,"role":"editor","signKey":values::encode_binary(alice.verifying_key().as_bytes()),
            "encKey":values::encode_binary(&[6;32]),"pages":[PAGE],
        })).unwrap(), &alice).unwrap();
        store
            .owner_transaction(
                &space,
                alice.verifying_key().as_bytes(),
                tmt_colab::store::owner::Mutation {
                    operation_id: PAGE,
                    digest: [7; 32],
                    expected_revision: 0,
                },
                |tx| {
                    tx.append_statement(&initial)?;
                    Ok(Vec::new())
                },
            )
            .unwrap();
        let devices = [
            (ALICE.into(), alice.verifying_key().to_bytes()),
            (BOB.into(), bob.verifying_key().to_bytes()),
        ]
        .into();
        let server = Server::new(
            store,
            Policy {
                space: space.clone(),
                devices,
                epoch: "1".into(),
                denied: None,
                baseline: None,
                owner: alice.verifying_key().to_bytes(),
            },
        );
        Self {
            root,
            server,
            space,
            alice,
            bob,
        }
    }
    fn peer(&self, id: &str) -> Peer {
        let (client, server) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        server.set_nonblocking(true).unwrap();
        (
            WebSocket::from_raw_socket(client, Role::Client, None),
            self.server.connect(server, id.into()).unwrap(),
        )
    }
    fn frame(&self, kind: &str, fields: Value) -> Value {
        let mut v = json!({"type":kind,"version":1,"space":self.space,"page":PAGE,"epoch":"1"});
        v.as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        v
    }
    fn append(
        &self,
        id: &str,
        seq: u64,
        previous: [u8; 32],
        namespace: &str,
        size: usize,
    ) -> (Value, [u8; 32], Vec<u8>) {
        let context = object::Context {
            space: self.space.clone(),
            page: PAGE.into(),
            epoch: "1".into(),
            kind: "update".into(),
            namespace: namespace.into(),
            author_device: id.into(),
            membership_revision: "1".into(),
            stream_seq: seq.to_string(),
            prev_hash: previous,
        };
        let envelope = object::seal(
            &context,
            &[9; 32],
            if id == ALICE { &self.alice } else { &self.bob },
            &vec![7; size],
        )
        .unwrap();
        let hash = envelope.hash().unwrap();
        let bytes = envelope.to_json().unwrap();
        (self.frame("append", json!({"streamId":id,"seq":seq.to_string(),"envelopeHash":values::encode_binary(&hash),"envelope":values::encode_binary(&bytes)})), hash, bytes)
    }
    fn payload(&self, id: &str, seq: u64) -> Option<Vec<u8>> {
        Store::open(&Layout::open(&self.root).unwrap())
            .unwrap()
            .payload(
                StreamScope {
                    page: PAGE,
                    epoch: 1,
                    stream: id,
                },
                seq,
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
fn send(peer: &mut Peer, frame: Value) {
    // Large frames exceed a Unix socket's kernel buffer. Drive both endpoints
    // without threads/sleeps, retaining tungstenite's buffered bytes on WouldBlock.
    peer.0.get_mut().set_nonblocking(true).unwrap();
    let mut flushed = match peer.0.send(Message::Text(frame.to_string().into())) {
        Ok(()) => true,
        Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => false,
        Err(e) => panic!("send: {e}"),
    };
    let mut advanced = false;
    for _ in 0..32 {
        advanced |= peer.1.poll() == Progress::Advanced;
        if flushed {
            assert!(advanced);
            return;
        }
        match peer.0.flush() {
            Ok(()) => flushed = true,
            Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("flush: {e}"),
        }
    }
    panic!("send did not finish in bounded duplex turns");
}
fn receive(peer: &mut Peer) -> Value {
    peer.0.get_mut().set_nonblocking(true).unwrap();
    for _ in 0..32 {
        match peer.0.read() {
            Ok(message) => return serde_json::from_str(message.to_text().unwrap()).unwrap(),
            Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert_ne!(peer.1.poll(), Progress::Closed);
            }
            Err(e) => panic!("receive: {e}"),
        }
    }
    panic!("receive did not finish in bounded duplex turns");
}
fn error(peer: &mut Peer, code: &str) {
    let v = receive(peer);
    assert_eq!(v["type"], "error");
    assert_eq!(v["code"], code);
}
fn closed(peer: &mut Peer, code: &str) {
    let message = peer.0.read().unwrap();
    let Message::Close(Some(close)) = message else {
        panic!("expected close, got {message:?}")
    };
    assert_eq!(close.reason, code);
    assert_eq!(peer.1.poll(), Progress::Closed);
}
#[test]
fn two_clients_broadcast_exact_retry_and_durable_bytes() {
    let f = Fixture::new();
    let mut alice = f.peer(ALICE);
    let mut bob = f.peer(BOB);
    send(&mut bob, f.frame("subscribe", json!({"cursors":[]})));
    let (frame, _, bytes) = f.append(ALICE, 1, [0; 32], "content", 20);
    send(&mut alice, frame.clone());
    let receipt = receive(&mut alice);
    assert_eq!(receipt["type"], "receipt");
    assert_eq!(bob.1.poll(), Progress::Advanced);
    let broadcast = receive(&mut bob);
    assert_eq!(broadcast["type"], "broadcast");
    assert_eq!(
        values::binary(broadcast["envelope"].as_str().unwrap(), 65536).unwrap(),
        bytes
    );
    send(&mut alice, frame);
    assert_eq!(receive(&mut alice), receipt);
    assert_eq!(bob.1.poll(), Progress::Pending);
    assert_eq!(f.payload(ALICE, 1), Some(bytes));
    send(&mut alice, f.frame("subscribe", json!({"cursors":[]})));
    let (frame, _, _) = f.append(BOB, 1, [0; 32], "own", 20);
    send(&mut bob, frame);
    assert_eq!(receive(&mut bob)["type"], "receipt");
    assert_eq!(alice.1.poll(), Progress::Advanced);
    assert_eq!(receive(&mut alice)["streamId"], BOB);
}
#[test]
fn gap_conflict_role_signature_and_epoch_denials_have_no_new_payload() {
    let f = Fixture::new();
    let mut a = f.peer(ALICE);
    let mut b = f.peer(BOB);
    let (gap, _, _) = f.append(ALICE, 2, [1; 32], "content", 4);
    send(&mut a, gap);
    error(&mut a, "GAP");
    let (denied, _, _) = f.append(BOB, 1, [0; 32], "content", 4);
    send(&mut b, denied);
    error(&mut b, "DENIED");
    let (mut bad, _, _) = f.append(ALICE, 1, [0; 32], "content", 4);
    let mut bytes = values::binary(bad["envelope"].as_str().unwrap(), 65536).unwrap();
    let mut envelope: Value = serde_json::from_slice(&bytes).unwrap();
    envelope["signature"] = json!(values::encode_binary(&[0; 64]));
    bytes = serde_json::to_vec(&envelope).unwrap();
    let decoded = object::Envelope::from_json(&bytes).unwrap();
    bad["envelopeHash"] = json!(values::encode_binary(&decoded.hash().unwrap()));
    bad["envelope"] = json!(values::encode_binary(&bytes));
    send(&mut a, bad);
    error(&mut a, "INVALID");
    assert_eq!(f.payload(ALICE, 1), None);
    assert_eq!(f.payload(BOB, 1), None);
    let (frame, head, _) = f.append(ALICE, 1, [0; 32], "content", 4);
    send(&mut a, frame.clone());
    receive(&mut a);
    let (fork, _, _) = f.append(ALICE, 1, [0; 32], "content", 5);
    send(&mut a, fork);
    error(&mut a, "CONFLICT");
    send(&mut a, frame);
    assert_eq!(receive(&mut a)["type"], "receipt");
    let (next, _, _) = f.append(ALICE, 2, head, "content", 4);
    send(&mut a, next);
    error(&mut a, "CONFLICT");
    assert_eq!(f.payload(ALICE, 2), None);
    let mut fresh = f.peer(ALICE);
    let mut stale = f.frame("subscribe", json!({"cursors":[]}));
    stale["epoch"] = json!("2");
    send(&mut fresh, stale);
    error(&mut fresh, "STALE_EPOCH");
}
#[test]
fn slow_subscriber_closes_without_losing_receipts_and_awareness_is_ephemeral() {
    let f = Fixture::new();
    let mut a = f.peer(ALICE);
    let mut b = f.peer(BOB);
    send(&mut b, f.frame("subscribe", json!({"cursors":[]})));
    let mut hash = [0; 32];
    for n in 1..=9 {
        let (frame, next, _) = f.append(ALICE, n, hash, "content", 4);
        send(&mut a, frame);
        assert_eq!(receive(&mut a)["seq"], n.to_string());
        hash = next;
    }
    assert_eq!(b.1.poll(), Progress::Closed);
    closed(&mut b, "RESYNC_REQUIRED");
    assert!(f.payload(ALICE, 9).is_some());
    let mut b = f.peer(BOB);
    send(&mut b, f.frame("subscribe", json!({"cursors":[]})));
    send(
        &mut a,
        f.frame(
            "awareness",
            json!({"device":ALICE,"data":values::encode_binary(b"present")}),
        ),
    );
    assert_eq!(b.1.poll(), Progress::Advanced);
    assert_eq!(receive(&mut b)["type"], "awareness");
    assert_eq!(f.payload(ALICE, 10), None);
    drop(b);
    let mut b = f.peer(BOB);
    send(&mut b, f.frame("subscribe", json!({"cursors":[]})));
    assert_eq!(b.1.poll(), Progress::Pending);
}
#[test]
fn revocation_drops_queued_ciphertext_and_expiry_is_distinct() {
    let f = Fixture::new();
    let mut a = f.peer(ALICE);
    let mut b = f.peer(BOB);
    send(&mut b, f.frame("subscribe", json!({"cursors":[]})));
    let (frame, _, _) = f.append(ALICE, 1, [0; 32], "content", 4);
    send(&mut a, frame);
    receive(&mut a);
    f.server
        .update_admission(|p| {
            p.devices.remove(BOB);
        })
        .unwrap();
    assert_eq!(b.1.poll(), Progress::Closed);
    closed(&mut b, "DENIED");
    f.server
        .update_admission(|p| p.denied = Some(Code::Expired))
        .unwrap();
    assert_eq!(a.1.poll(), Progress::Closed);
    closed(&mut a, "EXPIRED");
}
#[test]
fn strict_frames_and_connection_capacity() {
    let f = Fixture::new();
    let valid = f.frame("subscribe", json!({"cursors":[]})).to_string();
    for text in [
        valid.replacen("{", "{\"version\":1,", 1),
        valid.replacen("{", "{\"extra\":0,", 1),
        valid.replace("\"epoch\":\"1\"", "\"epoch\":1"),
        valid.replace("\"epoch\":\"1\"", "\"epoch\":\"01\""),
        "[]".into(),
    ] {
        let mut peer = f.peer(ALICE);
        peer.0.send(Message::Text(text.into())).unwrap();
        assert_eq!(peer.1.poll(), Progress::Closed);
        closed(&mut peer, "INVALID");
    }
    let peers: Vec<_> = (0..16).map(|_| f.peer(ALICE)).collect();
    let (_, stream) = UnixStream::pair().unwrap();
    assert!(matches!(
        f.server.connect(stream, ALICE.into()),
        Err(Code::Capacity)
    ));
    drop(peers);
    let _reusable = f.peer(ALICE);
}
#[test]
fn oversized_frame_rejects_from_header_without_allocating_body() {
    let f = Fixture::new();
    let (mut raw, stream) = UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    let mut server = f.server.connect(stream, ALICE.into()).unwrap();
    let mut header = vec![0x81, 0xff];
    header.extend_from_slice(&65537u64.to_be_bytes());
    header.extend_from_slice(&[1, 2, 3, 4]);
    raw.write_all(&header).unwrap();
    assert_eq!(server.poll(), Progress::Closed);
    assert_eq!(f.payload(ALICE, 1), None);
}
#[test]
fn empty_catchup_has_metadata_then_atomic_live_subscription() {
    let f = Fixture::new();
    let mut peer = f.peer(ALICE);
    send(
        &mut peer,
        f.frame("hello", json!({"device":ALICE,"cursors":[]})),
    );
    let first = receive(&mut peer);
    assert_eq!(first["type"], "catchup");
    assert_eq!(first["membershipHead"]["revision"], "1");
    assert_eq!(first["baseline"], Value::Null);
    assert_eq!(first["more"], true);
    assert_eq!(peer.1.poll(), Progress::Advanced);
    let last = receive(&mut peer);
    assert_eq!(last["more"], false);
    assert!(last.get("membershipHead").is_none());
    let cursor = json!({"streamId":ALICE,"namespace":"content","seq":"0","envelopeHash":values::encode_binary(&[0;32])});
    send(&mut peer, f.frame("ack", json!({"cursors":[cursor]})));
    assert_eq!(peer.1.poll(), Progress::Pending);
    send(
        &mut peer,
        f.frame("hello", json!({"device":ALICE,"cursors":[]})),
    );
    error(&mut peer, "INVALID");
}

#[test]
fn page_capacity_returns_error_and_preserves_original_receipt() {
    let f = Fixture::new();
    let mut a = f.peer(ALICE);
    let (first, hash, _) = f.append(ALICE, 1, [0; 32], "content", 4);
    send(&mut a, first.clone());
    let receipt = receive(&mut a);
    // Fill the existing storage quota without a long series of network writes.
    let db = rusqlite::Connection::open(f.root.join("colab/space.db")).unwrap();
    db.execute(
        "UPDATE receipts SET payload=zeroblob(?)",
        [tmt_colab::limits::PAGE_BYTES as i64],
    )
    .unwrap();
    let (next, _, _) = f.append(ALICE, 2, hash, "content", 4);
    send(&mut a, next);
    error(&mut a, "CAPACITY");
    assert_eq!(f.payload(ALICE, 2), None);
    send(&mut a, first);
    assert_eq!(receive(&mut a), receipt);
}

struct GatedStream {
    stream: UnixStream,
    blocked: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl std::io::Read for GatedStream {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        std::io::Read::read(&mut self.stream, b)
    }
}
impl Write for GatedStream {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.blocked.load(Ordering::Relaxed) {
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        self.stream.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[test]
fn blocked_frame_counts_toward_queue_cap_and_is_not_flushed_after_revocation() {
    for revoke in [false, true] {
        let f = Fixture::new();
        let mut a = f.peer(ALICE);
        let (client, stream) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let blocked = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut server = f
            .server
            .connect(
                GatedStream {
                    stream,
                    blocked: blocked.clone(),
                },
                BOB.into(),
            )
            .unwrap();
        let mut client = WebSocket::from_raw_socket(client, Role::Client, None);
        client
            .send(Message::Text(
                f.frame("subscribe", json!({"cursors":[]}))
                    .to_string()
                    .into(),
            ))
            .unwrap();
        assert_eq!(server.poll(), Progress::Advanced);
        blocked.store(true, Ordering::Relaxed);
        let (frame, mut hash, _) = f.append(ALICE, 1, [0; 32], "content", 4);
        send(&mut a, frame);
        receive(&mut a);
        assert_eq!(server.poll(), Progress::Advanced);
        if revoke {
            f.server
                .update_admission(|p| {
                    p.devices.remove(BOB);
                })
                .unwrap();
        } else {
            for n in 2..=9 {
                let (frame, next, _) = f.append(ALICE, n, hash, "content", 4);
                send(&mut a, frame);
                receive(&mut a);
                hash = next;
            }
        }
        blocked.store(false, Ordering::Relaxed);
        assert_eq!(server.poll(), Progress::Closed);
        // Abort a stalled transport instead of flushing ciphertext ahead of a close.
        assert!(matches!(
            client.read(),
            Err(tungstenite::Error::Protocol(
                tungstenite::error::ProtocolError::ResetWithoutClosingHandshake
            ))
        ));
    }
}

fn transfer(f: &Fixture, frame: Value, bytes: &[u8]) -> (Value, Vec<Value>) {
    let envelope = object::Envelope::from_json(bytes).unwrap();
    let header = object::Header::decode(envelope.header()).unwrap();
    let hash = values::encode_binary(&envelope.hash().unwrap());
    let mut reference = frame;
    reference["envelope"] = json!({"objectId":header.object_id});
    let count = bytes.len().div_ceil(tmt_colab::limits::CHUNK_BYTES);
    let chunks = bytes.chunks(tmt_colab::limits::CHUNK_BYTES).enumerate().map(|(index, chunk)|
        f.frame("chunk", json!({"objectId":header.object_id,"envelopeHash":hash,"index":index,"count":count,"bytes":values::encode_binary(chunk)}))).collect();
    (reference, chunks)
}
fn assemble(peer: &mut Peer, expected: &[u8]) {
    let count = expected.len().div_ceil(tmt_colab::limits::CHUNK_BYTES);
    let mut bytes = Vec::new();
    let mut identity = None;
    for index in 0..count {
        assert_ne!(peer.1.poll(), Progress::Closed);
        let frame = receive(peer);
        assert_eq!(frame["type"], "chunk");
        assert_eq!(frame["index"], index);
        assert_eq!(frame["count"], count);
        let current = (frame["objectId"].clone(), frame["envelopeHash"].clone());
        assert_eq!(identity.get_or_insert(current.clone()), &current);
        bytes.extend(
            values::binary(
                frame["bytes"].as_str().unwrap(),
                tmt_colab::limits::CHUNK_BYTES,
            )
            .unwrap(),
        );
        assert!(frame.to_string().len() <= tmt_colab::limits::WS_FRAME_BYTES);
    }
    assert_eq!(bytes, expected);
    let envelope = object::Envelope::from_json(&bytes).unwrap();
    assert_eq!(
        values::encode_binary(&envelope.hash().unwrap()),
        identity.unwrap().1
    );
}
#[test]
fn large_chunk_append_applies_only_after_completion_and_lazy_broadcast_retries() {
    let f = Fixture::new();
    let mut a = f.peer(ALICE);
    let mut b = f.peer(BOB);
    send(&mut b, f.frame("subscribe", json!({"cursors":[]})));
    let (frame, _, bytes) = f.append(ALICE, 1, [0; 32], "content", 256 * 1024);
    let (reference, chunks) = transfer(&f, frame, &bytes);
    assert!(chunks.len() > tmt_colab::limits::SEND_QUEUE_FRAMES);
    send(&mut a, reference.clone());
    for chunk in &chunks[..chunks.len() - 1] {
        send(&mut a, chunk.clone());
        assert_eq!(f.payload(ALICE, 1), None);
        assert_eq!(b.1.poll(), Progress::Pending);
    }
    send(&mut a, chunks.last().unwrap().clone());
    let receipt = receive(&mut a);
    assert_eq!(receipt["type"], "receipt");
    assert_eq!(f.payload(ALICE, 1), Some(bytes.clone()));
    assert_eq!(b.1.poll(), Progress::Advanced);
    assert!(receive(&mut b)["envelope"]["objectId"].is_string());
    assemble(&mut b, &bytes);
    // Exact frozen replay does not create a second transfer/broadcast.
    send(&mut a, reference);
    for chunk in chunks {
        send(&mut a, chunk);
    }
    assert_eq!(receive(&mut a), receipt);
    assert_eq!(b.1.poll(), Progress::Pending);
    let mut reconnect = f.peer(BOB);
    send(
        &mut reconnect,
        f.frame("hello", json!({"device":BOB,"cursors":[]})),
    );
    receive(&mut reconnect);
    assert_eq!(reconnect.1.poll(), Progress::Advanced);
    let page = receive(&mut reconnect);
    assert!(page["streams"][0]["tail"][0]["envelope"]["objectId"].is_string());
    assemble(&mut reconnect, &bytes);
    assert_ne!(reconnect.1.poll(), Progress::Closed);
    assert_eq!(receive(&mut reconnect)["more"], false);
}
#[test]
fn catchup_more_pages_than_queue_includes_appends_to_visited_namespaces_then_live() {
    let f = Fixture::new();
    let mut a = f.peer(ALICE);
    let mut hash = [0; 32];
    for seq in 1..=10 {
        let (frame, next, _) = f.append(
            ALICE,
            seq,
            hash,
            if seq % 2 == 1 { "content" } else { "own" },
            4,
        );
        send(&mut a, frame);
        receive(&mut a);
        hash = next;
    }
    let mut b = f.peer(BOB);
    send(&mut b, f.frame("hello", json!({"device":BOB,"cursors":[]})));
    receive(&mut b);
    let mut seen = Vec::new();
    for page in 0..11 {
        assert_eq!(b.1.poll(), Progress::Advanced);
        let result = receive(&mut b);
        assert_eq!(result["more"], true);
        assert!(result.get("membershipHead").is_none());
        seen.push(
            result["streams"][0]["tail"][0]["seq"]
                .as_str()
                .unwrap()
                .parse::<u64>()
                .unwrap(),
        );
        if page == 5 {
            // Content was exhausted before the first own page. Revisit it.
            let (frame, next, _) = f.append(ALICE, 11, hash, "content", 4);
            send(&mut a, frame);
            receive(&mut a);
            hash = next;
        }
    }
    seen.sort_unstable();
    assert_eq!(seen, (1..=11).collect::<Vec<_>>());
    assert_eq!(b.1.poll(), Progress::Advanced);
    assert_eq!(receive(&mut b)["more"], false);
    let (frame, _, _) = f.append(ALICE, 12, hash, "own", 4);
    send(&mut a, frame);
    receive(&mut a);
    assert_eq!(b.1.poll(), Progress::Advanced);
    let live = receive(&mut b);
    assert_eq!(live["type"], "broadcast");
    assert_eq!(live["seq"], "12");
}
#[test]
fn unknown_pruned_and_wrong_namespace_cursors_resync_and_zero_bootstraps_checkpoint() {
    let f = Fixture::new();
    let mut a = f.peer(ALICE);
    let (first, hash, _) = f.append(ALICE, 1, [0; 32], "content", 4);
    send(&mut a, first);
    receive(&mut a);
    let (second, head, _) = f.append(ALICE, 2, hash, "own", 4);
    send(&mut a, second);
    receive(&mut a);
    let checkpoint = object::seal(
        &object::Context {
            space: f.space.clone(),
            page: PAGE.into(),
            epoch: "1".into(),
            kind: "checkpoint".into(),
            namespace: "content".into(),
            author_device: ALICE.into(),
            membership_revision: "1".into(),
            stream_seq: "2".into(),
            prev_hash: head,
        },
        &[9; 32],
        &f.alice,
        b"checkpoint",
    )
    .unwrap();
    let bytes = checkpoint.to_json().unwrap();
    Store::open(&Layout::open(&f.root).unwrap())
        .unwrap()
        .checkpoint(&tmt_colab::store::Envelope {
            scope: StreamScope {
                page: PAGE,
                epoch: 1,
                stream: ALICE,
            },
            namespace: tmt_colab::store::Namespace::Content,
            seq: 2,
            hash: checkpoint.hash().unwrap(),
            previous: head,
            bytes: &bytes,
        })
        .unwrap();
    for (seq, hash, ns) in [
        (1, hash, "content"),
        (2, head, "content"),
        (9, [8; 32], "own"),
    ] {
        let mut b = f.peer(BOB);
        let cursor = json!({"streamId":ALICE,"namespace":ns,"seq":seq.to_string(),"envelopeHash":values::encode_binary(&hash)});
        send(
            &mut b,
            f.frame("hello", json!({"device":BOB,"cursors":[cursor]})),
        );
        error(&mut b, "RESYNC_REQUIRED");
    }
    let mut b = f.peer(BOB);
    let cursor = json!({"streamId":ALICE,"namespace":"own","seq":"2","envelopeHash":values::encode_binary(&head)});
    send(&mut b, f.frame("subscribe", json!({"cursors":[cursor]})));
    receive(&mut b);
    assert_eq!(b.1.poll(), Progress::Advanced);
    let result = receive(&mut b);
    assert_eq!(result["streams"][0]["namespace"], "content");
    assert_eq!(
        values::binary(
            result["streams"][0]["checkpoint"]["envelope"]
                .as_str()
                .unwrap(),
            65536
        )
        .unwrap(),
        bytes
    );
    assert_eq!(b.1.poll(), Progress::Advanced);
    assert_eq!(receive(&mut b)["more"], false);
}
#[test]
fn invalid_chunk_order_identity_count_and_payload_discard_partial_state() {
    for mutation in [
        "index",
        "count",
        "objectId",
        "envelopeHash",
        "bytes",
        "count-cap",
        "count-zero",
    ] {
        let f = Fixture::new();
        let mut a = f.peer(ALICE);
        let (frame, _, bytes) = f.append(ALICE, 1, [0; 32], "content", 60000);
        let (reference, chunks) = transfer(&f, frame, &bytes);
        send(&mut a, reference.clone());
        send(&mut a, chunks[0].clone());
        let mut bad = chunks[1].clone();
        bad[if mutation.starts_with("count-") {
            "count"
        } else {
            mutation
        }] = match mutation {
            "count-cap" => json!(tmt_colab::limits::CHUNK_COUNT + 1),
            "count-zero" => json!(0),
            "index" => json!(0),
            "count" => json!(chunks.len() + 1),
            "objectId" => json!("a".repeat(64)),
            "envelopeHash" => json!(values::encode_binary(&[8; 32])),
            _ => json!(""),
        };
        send(&mut a, bad);
        error(&mut a, "INVALID");
        assert_eq!(f.payload(ALICE, 1), None);
        // Fresh transfer is possible only from zero after rejection.
        send(&mut a, reference);
        for chunk in chunks {
            send(&mut a, chunk);
        }
        assert_eq!(receive(&mut a)["type"], "receipt");
    }
}
#[test]
fn chunk_deadline_drop_revocation_and_signature_failure_never_persist_partial_bytes() {
    for mode in ["deadline", "drop", "revoke", "signature", "object-binding"] {
        let f = Fixture::new();
        let mut a = f.peer(ALICE);
        let (frame, _, mut bytes) = f.append(ALICE, 1, [0; 32], "content", 60000);
        let mut frame = frame;
        if mode == "signature" {
            let mut v: Value = serde_json::from_slice(&bytes).unwrap();
            v["signature"] = json!(values::encode_binary(&[0; 64]));
            bytes = serde_json::to_vec(&v).unwrap();
            frame["envelopeHash"] = json!(values::encode_binary(
                &object::Envelope::from_json(&bytes).unwrap().hash().unwrap()
            ));
        }
        let (mut reference, mut chunks) = transfer(&f, frame, &bytes);
        if mode == "object-binding" {
            reference["envelope"]["objectId"] = json!("a".repeat(64));
            for chunk in &mut chunks {
                chunk["objectId"] = json!("a".repeat(64));
            }
        }
        send(&mut a, reference);
        send(&mut a, chunks[0].clone());
        match mode {
            "deadline" => {
                assert_eq!(
                    a.1.poll_at(std::time::Instant::now() + Duration::from_secs(2)),
                    Progress::Closed
                );
                closed(&mut a, "INVALID");
            }
            "drop" => drop(a),
            "revoke" => {
                f.server
                    .update_admission(|p| {
                        p.devices.remove(ALICE);
                    })
                    .unwrap();
                assert_eq!(a.1.poll(), Progress::Closed);
                closed(&mut a, "DENIED");
            }
            _ => {
                for chunk in chunks.into_iter().skip(1) {
                    send(&mut a, chunk);
                }
                error(&mut a, "INVALID");
            }
        }
        assert_eq!(f.payload(ALICE, 1), None);
    }
}
#[test]
fn bootstrap_baseline_descriptor_is_exact_scoped_and_first_page_only() {
    let f = Fixture::new();
    let bytes = serde_json::to_vec(&json!({"pageId":PAGE,"epoch":"1","sourceDigest":values::encode_binary(&[1;32]),
        "baselineCommitment":values::encode_binary(&[2;32]),"title":"Example","objectEnvelopeHash":values::encode_binary(&[3;32]),"membershipRevision":"1"})).unwrap();
    f.server
        .update_admission(|p| p.baseline = Some(bytes.clone()))
        .unwrap();
    let mut b = f.peer(BOB);
    send(&mut b, f.frame("hello", json!({"device":BOB,"cursors":[]})));
    let result = receive(&mut b);
    assert_eq!(
        values::binary(result["baseline"].as_str().unwrap(), 8192).unwrap(),
        bytes
    );
    assert_eq!(b.1.poll(), Progress::Advanced);
    assert!(receive(&mut b).get("baseline").is_none());
    let mut wrong: Value = serde_json::from_slice(&bytes).unwrap();
    wrong["epoch"] = json!("2");
    f.server
        .update_admission(|p| p.baseline = Some(serde_json::to_vec(&wrong).unwrap()))
        .unwrap();
    let mut a = f.peer(ALICE);
    send(
        &mut a,
        f.frame("hello", json!({"device":ALICE,"cursors":[]})),
    );
    error(&mut a, "INVALID");
}
