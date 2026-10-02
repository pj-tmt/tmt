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
    sync::{Access, Admission, Code, Connection, Progress, Server, SyncScope},
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
}
impl Admission for Policy {
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
        let store = Store::open(&layout).unwrap();
        store.create_page(PAGE).unwrap();
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
    peer.0
        .send(Message::Text(frame.to_string().into()))
        .unwrap();
    assert_eq!(peer.1.poll(), Progress::Advanced);
}
fn receive(peer: &mut Peer) -> Value {
    let message = peer.0.read().unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
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
fn catchup_and_nonempty_cursors_are_explicitly_deferred() {
    let f = Fixture::new();
    let mut peer = f.peer(ALICE);
    send(
        &mut peer,
        f.frame("hello", json!({"device":ALICE,"cursors":[]})),
    );
    error(&mut peer, "RESYNC_REQUIRED");
    let cursor = json!({"streamId":ALICE,"namespace":"content","seq":"0","envelopeHash":values::encode_binary(&[0;32])});
    send(&mut peer, f.frame("subscribe", json!({"cursors":[cursor]})));
    error(&mut peer, "RESYNC_REQUIRED");
    send(&mut peer, f.frame("ack", json!({"cursors":[]})));
    assert_eq!(peer.1.poll(), Progress::Pending);
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
