use super::*;
use crate::readers::test_support::{DEVICE, Fixture, LINK, PAGE, SEED};
use crate::transitions::{LinkAction, LinkSpec, OwnerAction, ShareMode};
use serde_json::{Value, json};
use tmt_colab_model::{certificate, link, values, wrap};
use tungstenite::{Message, WebSocket, protocol::Role};

#[test]
fn response_drains_input_sent_after_its_fin_until_peer_eof() {
    let (mut socket, mut peer) = UnixStream::pair().unwrap();
    peer.set_read_timeout(Some(limits::RESPONSE)).unwrap();
    peer.set_write_timeout(Some(limits::RESPONSE)).unwrap();
    socket.set_read_timeout(Some(limits::RESPONSE)).unwrap();
    let worker = thread::spawn(move || {
        response(&mut socket, 429, b"CAPACITY", false).unwrap();
        socket.set_nonblocking(false).unwrap();
        assert_eq!(socket.read(&mut [0; 1]).unwrap(), 0, "input not drained");
    });
    let mut reply = String::new();
    peer.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 429"));
    assert!(reply.ends_with("CAPACITY"));
    // The server has already sent FIN; the request tail is intentionally late.
    peer.write_all(&vec![b'x'; limits::HTTP_BODY_BYTES])
        .unwrap();
    peer.shutdown(std::net::Shutdown::Write).unwrap();
    worker.join().unwrap();
}

#[test]
fn websocket_keys_are_exactly_sixteen_base64_bytes() {
    // RFC 6455 section 1.3 sample key.
    assert!(websocket_key("dGhlIHNhbXBsZSBub25jZQ=="));
    assert!(websocket_key("AAAAAAAAAAAAAAAAAAAAAA=="));
    for invalid in [
        "",
        "dGhlIHNhbXBsZSBub25jZQ",
        "dGhlIHNhbXBsZSBub25jZQ=",
        "dGhlIHNhbXBsZSBub25jZR==",
        "dGhlIHNhbXBsZSBub25jZQ===",
        "dGhlIHNhbXBsZSBub25j-Q==",
        "dGhlIHNhbXBsZSBub25jZQ==\r",
        "AAAAAAAAAAAAAAAAAAAAAAAA",
    ] {
        assert!(!websocket_key(invalid), "{invalid:?}");
    }
}

const FRESH_DEVICE: &str = "30000000-0000-4000-8000-000000000002";
const REPLACEMENT_LINK: &str = "20000000-0000-4000-8000-000000000002";
const REPLACEMENT_SEED: [u8; 32] = [29; 32];

struct ReaderMount {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    path: std::path::PathBuf,
}
impl ReaderMount {
    fn start(f: &mut Fixture) -> Self {
        let layout = Layout::open(&f.root).unwrap();
        let mount = MountSocket::bind(&layout, &f.key.space_id, Tunnels::PRODUCT)
            .unwrap()
            .with_registration(&layout, f.service.clone())
            .unwrap();
        f.server = mount.sync.as_ref().unwrap().clone();
        let path = mount.path.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let worker = thread::spawn(move || mount.run(&flag).unwrap());
        Self {
            stop,
            worker: Some(worker),
            path,
        }
    }
    fn socket(&self) -> UnixStream {
        let socket = UnixStream::connect(&self.path).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
    }
    fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let mut socket = self.socket();
        let body = body.to_string();
        let event = if path == EVENTS {
            "tmt-device-event: 1\r\n"
        } else {
            ""
        };
        socket
            .write_all(
                format!(
                    "POST {path} HTTP/1.1\r\n{event}Content-Length: {}\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        let status = response.split_whitespace().nth(1).unwrap().parse().unwrap();
        let body = response.split_once("\r\n\r\n").unwrap().1;
        (
            status,
            serde_json::from_str(body).unwrap_or_else(|_| json!(body)),
        )
    }
    fn challenge(&self, f: &Fixture, chain: Option<&[u8]>) -> (u16, Value) {
        let mut body = json!({"kind":if chain.is_some() {"link"} else {"public"},"space":f.key.space_id,"page":PAGE});
        if let Some(chain) = chain {
            body["chain"] = json!(values::encode_binary(chain));
        }
        self.post(crate::readers::CHALLENGE_PATH, body)
    }
    fn session(&self, f: &Fixture, link: bool) -> Value {
        let (status, challenge) = self.challenge(f, link.then_some(f.chain.as_slice()));
        assert_eq!(status, 200, "{challenge}");
        let (status, session) = self.post(
            crate::readers::SESSION_PATH,
            f.exchange_body(&challenge, link),
        );
        assert_eq!(status, 200, "{session}");
        session
    }
    fn upgrade(&self, session: &Value) -> std::result::Result<WebSocket<UnixStream>, u16> {
        let token = session["token"].as_str().unwrap();
        let mut socket = self.socket();
        socket.write_all(format!("GET /sync HTTP/1.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: colab-reader-v1.{token}, colab-sync-v1\r\n\r\n").as_bytes()).unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            assert!(head.len() < limits::HEADER_BYTES);
            let mut b = [0];
            socket.read_exact(&mut b).unwrap();
            head.push(b[0]);
        }
        let head = String::from_utf8(head).unwrap();
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        assert!(!head.contains(token));
        if status != 101 {
            return Err(status);
        }
        assert!(head.contains("Sec-WebSocket-Protocol: colab-sync-v1\r\n"));
        Ok(WebSocket::from_raw_socket(socket, Role::Client, None))
    }
}
impl Drop for ReaderMount {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
        assert!(!self.path.exists(), "reader mount leaked its socket");
    }
}
fn reader_frame(session: &Value, kind: &str, fields: Value) -> Value {
    let mut frame = json!({"version":1,"type":kind,"space":session["space"],"page":session["page"],"epoch":session["epoch"]});
    for (key, value) in fields.as_object().unwrap() {
        frame[key] = value.clone();
    }
    frame
}
fn send_reader(peer: &mut WebSocket<UnixStream>, frame: Value) {
    peer.send(Message::Text(frame.to_string().into())).unwrap();
}
fn read_reader(peer: &mut WebSocket<UnixStream>) -> Value {
    serde_json::from_str(peer.read().unwrap().to_text().unwrap()).unwrap()
}
fn bootstrap_reader(peer: &mut WebSocket<UnixStream>, session: &Value) -> Vec<Value> {
    send_reader(
        peer,
        reader_frame(
            session,
            "hello",
            json!({"device":session["principal"],"membershipRevision":"0","cursors":[]}),
        ),
    );
    let mut pages = Vec::new();
    for _ in 0..16 {
        let page = read_reader(peer);
        assert_eq!(page["type"], "catchup", "{page}");
        let done = page["more"] == false;
        pages.push(page);
        send_reader(peer, reader_frame(session, "ack", json!({"cursors":[]})));
        if done {
            return pages;
        }
    }
    panic!("reader catchup did not complete within its fixture bound");
}
fn reader_cut_off(peer: &mut WebSocket<UnixStream>) {
    // Only EOF/reset/close is success; timeout or application data always fails.
    match peer.read() {
        Ok(Message::Close(_)) => {}
        Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {}
        Err(tungstenite::Error::Protocol(
            tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
        )) => {}
        Err(tungstenite::Error::Io(e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionReset
            ) => {}
        other => panic!("reader not cut off: {other:?}"),
    }
}
fn fresh_old_chain(f: &Fixture) -> Vec<u8> {
    let chain = certificate::Chain::from_json(&f.chain).unwrap();
    let mut cert = chain.certificate().unwrap();
    cert.device_id = FRESH_DEVICE;
    cert.expires_at = 2_000_000;
    let keys = link::Keys::derive(&SEED, &f.key.space_id, LINK).unwrap();
    let mut wire: Value = serde_json::from_slice(&f.chain).unwrap();
    wire["deviceCertificate"] = json!(values::encode_binary(&certificate::input(&cert).unwrap()));
    wire["issuerSignature"] = json!(values::encode_binary(&keys.certify(&cert).unwrap()));
    serde_json::to_vec(&wire).unwrap()
}
fn verify_link_wraps(f: &Fixture, pages: &[Value], id: &str, seed: &[u8; 32]) {
    let keys = link::Keys::derive(seed, &f.key.space_id, id).unwrap();
    let scope = f.scope();
    let expected = f
        .store
        .owner_read(&f.key.space_id, &f.key.owner_public(), |tx| {
            Ok(tx
                .epoch_secret(PAGE, values::decimal(&scope.epoch, false)?)?
                .unwrap())
        })
        .unwrap();
    let mut current = false;
    for encoded in pages.iter().filter_map(|p| p["wraps"].as_array()).flatten() {
        let e =
            wrap::Envelope::from_json(&values::binary(encoded.as_str().unwrap(), 2048).unwrap())
                .unwrap();
        let h = e.header().unwrap();
        assert_eq!(h.recipient_kind, "link");
        assert_eq!(h.recipient_id, id);
        let key = wrap::open(&e, &h, keys.recipient(), &f.key.owner_public()).unwrap();
        if h.epoch == scope.epoch {
            assert_eq!(key, expected);
            current = true;
        }
    }
    // Public epochs publish their key in signed policy instead of fresh wraps.
    let mut head = None;
    for encoded in pages.iter().flat_map(|p| {
        p["membershipHead"]["statements"]
            .as_array()
            .or_else(|| p["membership"]["statements"].as_array())
            .into_iter()
            .flatten()
    }) {
        let statement = tmt_colab_model::statement::Envelope::from_json(
            &values::binary(encoded.as_str().unwrap(), 64 * 1024).unwrap(),
        )
        .unwrap();
        let verified = statement
            .verify_next(&f.key.space_id, &f.key.owner_public(), head.as_ref())
            .unwrap();
        if let tmt_colab_model::payload::Payload::PageShare(p) = &verified.payload
            && p.page_id == PAGE
            && p.epoch == scope.epoch
            && let Some(keys) = &p.published_keys
        {
            for key in keys.as_slice() {
                if key.epoch == scope.epoch {
                    assert_eq!(values::binary(&key.key, 32).unwrap(), expected);
                    current = true;
                }
            }
        }
        head = Some(verified.head);
    }
    assert!(
        current,
        "reader received no usable current-epoch key for {id} at {}",
        scope.epoch
    );
}
fn no_removed_link_wraps(f: &Fixture, old_epoch: &str) {
    let scope = f.scope();
    assert!(
        values::decimal(&scope.epoch, false).unwrap() > values::decimal(old_epoch, false).unwrap()
    );
    let epoch = values::decimal(&scope.epoch, false).unwrap();
    let (old_key, new_key) = f
        .store
        .owner_read(&f.key.space_id, &f.key.owner_public(), |tx| {
            Ok((
                tx.epoch_secret(PAGE, values::decimal(old_epoch, false)?)?
                    .unwrap(),
                tx.epoch_secret(PAGE, epoch)?.unwrap(),
            ))
        })
        .unwrap();
    assert_ne!(
        new_key, old_key,
        "narrowing/removal reused the exposed page key"
    );
    let baseline = f.store.baseline(PAGE, epoch).unwrap().unwrap();
    let envelope = tmt_colab_model::object::Envelope::from_json(&baseline.envelope).unwrap();
    let header = tmt_colab_model::object::Header::decode(envelope.header()).unwrap();
    let signer = f.key.management_member().unwrap().signing_key;
    assert!(tmt_colab_model::object::open(&envelope, &header.context, &new_key, &signer).is_ok());
    assert!(
        tmt_colab_model::object::open(&envelope, &header.context, &old_key, &signer).is_err(),
        "retained old key opened the private epoch baseline"
    );
    let db = rusqlite::Connection::open(f.root.join("colab/space.db")).unwrap();
    let mut query = db
        .prepare("SELECT envelope FROM wraps WHERE page=?1 AND epoch=?2")
        .unwrap();
    let rows = query
        .query_map(
            rusqlite::params![
                PAGE,
                format!("{:020}", values::decimal(&scope.epoch, false).unwrap())
            ],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(!rows.is_empty());
    let keys = link::Keys::derive(&SEED, &f.key.space_id, LINK).unwrap();
    for bytes in rows {
        let e = wrap::Envelope::from_json(&bytes).unwrap();
        let h = e.header().unwrap();
        assert!(!(h.recipient_kind == "link" && h.recipient_id == LINK));
        assert!(!(h.recipient_kind == "device" && h.recipient_id == DEVICE));
        assert!(wrap::open(&e, &h, keys.recipient(), &f.key.owner_public()).is_err());
    }
    let revoked = f
        .store
        .owner_read(&f.key.space_id, &f.key.owner_public(), |tx| {
            Ok(tx.recipient("link", LINK)?.unwrap().revoked)
        })
        .unwrap();
    assert!(revoked);
}

#[derive(Clone, Copy, Debug)]
enum ReaderChange {
    Private,
    Link,
    Remove,
    Reset,
    Rotate,
    Revoke,
    Delete,
    Expire,
    CertificateExpire,
}
fn change_reader(f: &mut Fixture, mount: &ReaderMount, change: ReaderChange) {
    match change {
        ReaderChange::Private => f.share(ShareMode::Private),
        ReaderChange::Link => f.share(ShareMode::Link),
        ReaderChange::Remove => f.apply(OwnerAction::Link(LinkAction::Remove { link_id: LINK })),
        ReaderChange::Reset => f.apply(OwnerAction::Link(LinkAction::Reset {
            link_id: LINK,
            replacement: Some(LinkSpec {
                id: REPLACEMENT_LINK,
                seed: &REPLACEMENT_SEED,
                role: "viewer",
                pages: vec![PAGE.into()],
            }),
        })),
        ReaderChange::Rotate => f.apply(OwnerAction::EpochAdvance { page: PAGE }),
        ReaderChange::Revoke => {
            assert_eq!(
                mount
                    .post(
                        EVENTS,
                        json!({"type":"device.revoked","deviceId":DEVICE,"grantRevision":1})
                    )
                    .0,
                200
            );
        }
        ReaderChange::Delete => f.apply(OwnerAction::Delete { page: PAGE }),
        ReaderChange::Expire => f.clock.store(601_000, Ordering::SeqCst),
        ReaderChange::CertificateExpire => f.clock.store(3000, Ordering::SeqCst),
    }
}
fn reader_fixture(public: bool, short_cert: bool) -> Fixture {
    let mut f = Fixture::new();
    f.share(ShareMode::Link);
    f.add_link();
    if public {
        f.share(ShareMode::Public);
    }
    if short_cert {
        let chain = certificate::Chain::from_json(&f.chain).unwrap();
        let mut cert = chain.certificate().unwrap();
        cert.expires_at = 3000;
        let keys = link::Keys::derive(&SEED, &f.key.space_id, LINK).unwrap();
        let mut wire: Value = serde_json::from_slice(&f.chain).unwrap();
        wire["deviceCertificate"] =
            json!(values::encode_binary(&certificate::input(&cert).unwrap()));
        wire["issuerSignature"] = json!(values::encode_binary(&keys.certify(&cert).unwrap()));
        f.chain = serde_json::to_vec(&wire).unwrap();
    }
    f
}

/// Real HTTP admission, mounted subscriptions and the root transition sync lock.
#[test]
fn mounted_reader_narrowing_reset_removal_rotation_revocation_and_expiry_twice() {
    for (public, change) in [
        (false, ReaderChange::Private),
        (true, ReaderChange::Private),
        (true, ReaderChange::Link),
        (false, ReaderChange::Remove),
        (false, ReaderChange::Reset),
        (false, ReaderChange::Rotate),
        (true, ReaderChange::Rotate),
        (false, ReaderChange::Revoke),
        (false, ReaderChange::Expire),
        (false, ReaderChange::CertificateExpire),
    ] {
        for _ in 0..2 {
            let mut f = reader_fixture(public, matches!(change, ReaderChange::CertificateExpire));
            let mount = ReaderMount::start(&mut f);
            let session = mount.session(&f, true);
            let mut peer = mount.upgrade(&session).unwrap();
            let pages = bootstrap_reader(&mut peer, &session);
            verify_link_wraps(&f, &pages, LINK, &SEED);
            let unused = mount.session(&f, true);
            let mut public_peer = public.then(|| {
                let s = mount.session(&f, false);
                let mut p = mount.upgrade(&s).unwrap();
                for page in bootstrap_reader(&mut p, &s) {
                    assert!(page["wraps"].as_array().is_none_or(|w| w.is_empty()));
                }
                p
            });
            change_reader(&mut f, &mount, change);
            reader_cut_off(&mut peer);
            if let Some(p) = &mut public_peer {
                reader_cut_off(p);
            }
            assert_eq!(
                mount.upgrade(&unused).err(),
                Some(403),
                "unused old ticket survived {change:?}"
            );
            let removed = matches!(
                change,
                ReaderChange::Private
                    | ReaderChange::Link
                    | ReaderChange::Remove
                    | ReaderChange::Reset
            );
            if removed
                || matches!(
                    change,
                    ReaderChange::Revoke | ReaderChange::CertificateExpire
                )
            {
                assert_eq!(mount.challenge(&f, Some(&f.chain)).0, 403);
            }
            if removed {
                // Even a freshly certified device from the retained old seed is excluded.
                assert_eq!(mount.challenge(&f, Some(&fresh_old_chain(&f))).0, 403);
                no_removed_link_wraps(&f, session["epoch"].as_str().unwrap());
                assert_eq!(mount.challenge(&f, None).0, 403);
            }
            if matches!(change, ReaderChange::Rotate | ReaderChange::Revoke) {
                if matches!(change, ReaderChange::Revoke) {
                    f.chain = fresh_old_chain(&f);
                }
                let fresh = mount.session(&f, true);
                let mut p = mount.upgrade(&fresh).unwrap();
                verify_link_wraps(&f, &bootstrap_reader(&mut p, &fresh), LINK, &SEED);
                // Rotation alone does not revoke a surviving bearer link.
                if public {
                    let s = mount.session(&f, false);
                    let mut p = mount.upgrade(&s).unwrap();
                    bootstrap_reader(&mut p, &s);
                }
            }
            if matches!(change, ReaderChange::Reset) {
                f.chain = f.certify_link(REPLACEMENT_LINK, &REPLACEMENT_SEED, FRESH_DEVICE);
                let fresh = mount.session(&f, true);
                let mut p = mount.upgrade(&fresh).unwrap();
                verify_link_wraps(
                    &f,
                    &bootstrap_reader(&mut p, &fresh),
                    REPLACEMENT_LINK,
                    &REPLACEMENT_SEED,
                );
            }
            if matches!(change, ReaderChange::CertificateExpire) {
                f.chain = fresh_old_chain(&f);
                let fresh = mount.session(&f, true);
                let mut p = mount.upgrade(&fresh).unwrap();
                verify_link_wraps(&f, &bootstrap_reader(&mut p, &fresh), LINK, &SEED);
            }
            if matches!(change, ReaderChange::Expire) {
                let fresh = mount.session(&f, true);
                let mut p = mount.upgrade(&fresh).unwrap();
                bootstrap_reader(&mut p, &fresh);
            }
        }
    }
}

#[test]
fn mounted_archived_readers_keep_reads_and_delete_cuts_off_both_kinds_twice() {
    for _ in 0..2 {
        let mut f = reader_fixture(true, false);
        let mount = ReaderMount::start(&mut f);
        let link = mount.session(&f, true);
        let public = mount.session(&f, false);
        let mut readers = [
            mount.upgrade(&link).unwrap(),
            mount.upgrade(&public).unwrap(),
        ];
        for (p, s) in readers.iter_mut().zip([&link, &public]) {
            bootstrap_reader(p, s);
        }
        f.apply(OwnerAction::Archive { page: PAGE });
        for (p, s) in readers.iter_mut().zip([&link, &public]) {
            p.send(Message::Ping(vec![7].into())).unwrap();
            assert!(matches!(p.read().unwrap(), Message::Pong(_)));
            // A new subscription still proves admission after the archive commit.
            let fresh = mount.session(&f, s == &link);
            let mut p = mount.upgrade(&fresh).unwrap();
            bootstrap_reader(&mut p, &fresh);
        }
        f.apply(OwnerAction::Delete { page: PAGE });
        for p in &mut readers {
            reader_cut_off(p);
        }
        assert_eq!(mount.challenge(&f, Some(&f.chain)).0, 403);
        assert_eq!(mount.challenge(&f, Some(&fresh_old_chain(&f))).0, 403);
        assert_eq!(mount.challenge(&f, None).0, 403);
        let db = rusqlite::Connection::open(f.root.join("colab/space.db")).unwrap();
        for table in ["receipts", "baselines", "wraps", "epoch_secrets"] {
            assert_eq!(
                db.query_row(
                    &format!("SELECT count(*) FROM {table} WHERE page=?"),
                    [PAGE],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        }
        assert!(
            db.query_row("SELECT count(*) FROM pages WHERE page=?", [PAGE], |r| r
                .get::<_, i64>(0))
                .unwrap()
                > 0
        );
    }
}

#[test]
fn mounted_prehello_readers_close_on_policy_device_epoch_and_time_changes_twice() {
    for (link, change) in [
        (true, ReaderChange::Private),
        (false, ReaderChange::Private),
        (false, ReaderChange::Link),
        (true, ReaderChange::Remove),
        (true, ReaderChange::Reset),
        (true, ReaderChange::Rotate),
        (true, ReaderChange::Revoke),
        (false, ReaderChange::Delete),
        (true, ReaderChange::Expire),
        (false, ReaderChange::Expire),
        (true, ReaderChange::CertificateExpire),
    ] {
        for _ in 0..2 {
            let mut f = reader_fixture(!link, matches!(change, ReaderChange::CertificateExpire));
            let mount = ReaderMount::start(&mut f);
            let s = mount.session(&f, link);
            let mut peer = mount.upgrade(&s).unwrap();
            // No hello is sent. Recheck must use the admitted session scope.
            change_reader(&mut f, &mount, change);
            // A new challenge reclaims unused entries, never drops active fencing.
            let _ = mount.challenge(&f, None);
            reader_cut_off(&mut peer);
        }
    }
}

#[test]
fn mounted_challenge_expiry_and_policy_recheck_before_exchange() {
    for (narrow, link) in [(false, false), (false, true), (true, false), (true, true)] {
        for _ in 0..2 {
            let mut f = reader_fixture(true, false);
            let mount = ReaderMount::start(&mut f);
            let (status, c) = mount.challenge(&f, link.then_some(f.chain.as_slice()));
            assert_eq!(status, 200);
            if narrow {
                f.share(ShareMode::Private);
            } else {
                f.clock.store(61_000, Ordering::SeqCst);
            }
            let (status, error) =
                mount.post(crate::readers::SESSION_PATH, f.exchange_body(&c, link));
            assert_eq!(status, 403);
            assert_eq!(error["code"], if narrow { "DENIED" } else { "EXPIRED" });
            assert_eq!(
                mount
                    .post(crate::readers::SESSION_PATH, f.exchange_body(&c, link))
                    .1["code"],
                "DENIED"
            );
        }
    }
}

struct BlockedReaderStream {
    stream: UnixStream,
    blocked: Arc<AtomicBool>,
    attempted: Arc<std::sync::atomic::AtomicUsize>,
    written: Arc<std::sync::atomic::AtomicUsize>,
    budget: Arc<std::sync::atomic::AtomicUsize>,
}
impl Read for BlockedReaderStream {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.stream.read(bytes)
    }
}
impl Write for BlockedReaderStream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.blocked.load(Ordering::SeqCst) || self.budget.load(Ordering::SeqCst) == 0 {
            self.attempted.fetch_add(bytes.len(), Ordering::SeqCst);
            return Err(std::io::ErrorKind::WouldBlock.into());
        }
        let remaining = self.budget.load(Ordering::SeqCst);
        let written = self.stream.write(&bytes[..bytes.len().min(remaining)])?;
        self.budget.fetch_sub(written, Ordering::SeqCst);
        self.written.fetch_add(written, Ordering::SeqCst);
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}
fn append_reader_fixture_object(f: &mut Fixture, size: usize) -> Vec<u8> {
    use ed25519_dalek::SigningKey;
    use tmt_colab_model::object;
    use yrs::{Doc, Map, ReadTxn, StateVector, Text, Transact};
    // Valid production decoder input, so root rotation verifies this exact tail.
    let doc = Doc::with_client_id(91);
    doc.get_or_insert_text("html")
        .insert(&mut doc.transact_mut(), 0, &"x".repeat(size));
    doc.get_or_insert_map("meta")
        .insert(&mut doc.transact_mut(), "title", "reader transfer");
    let update = doc
        .transact()
        .encode_state_as_update_v1(&StateVector::default());
    let scope = f.scope();
    let epoch = values::decimal(&scope.epoch, false).unwrap();
    let (revision, secret) = f
        .store
        .owner_read(&f.key.space_id, &f.key.owner_public(), |tx| {
            Ok((
                tx.head().unwrap().revision,
                tx.epoch_secret(PAGE, epoch)?.unwrap(),
            ))
        })
        .unwrap();
    let context = object::Context {
        space: scope.space,
        page: PAGE.into(),
        epoch: scope.epoch,
        kind: "update".into(),
        namespace: "content".into(),
        author_device: DEVICE.into(),
        membership_revision: revision.to_string(),
        stream_seq: "1".into(),
        prev_hash: [0; 32],
    };
    let envelope = object::seal(
        &context,
        &secret,
        &SigningKey::from_bytes(&[19; 32]),
        &update,
    )
    .unwrap();
    let bytes = envelope.to_json().unwrap();
    assert!(bytes.len() > limits::CHUNK_BYTES);
    assert_eq!(
        f.store
            .append(&crate::store::Envelope {
                scope: crate::store::StreamScope {
                    page: PAGE,
                    epoch,
                    stream: DEVICE
                },
                namespace: crate::store::Namespace::Content,
                seq: 1,
                hash: envelope.hash().unwrap(),
                previous: [0; 32],
                bytes: &bytes,
            })
            .unwrap(),
        crate::store::Accepted::New
    );
    bytes
}
fn read_controlled_reader(
    peer: &mut WebSocket<UnixStream>,
    connection: &mut crate::sync::Connection<
        BlockedReaderStream,
        crate::registration::OwnerAdmission,
    >,
) -> Value {
    peer.get_mut().set_nonblocking(true).unwrap();
    for _ in 0..64 {
        match peer.read() {
            Ok(message) => {
                peer.get_mut().set_nonblocking(false).unwrap();
                return serde_json::from_str(message.to_text().unwrap()).unwrap();
            }
            Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert_ne!(connection.poll(), Progress::Closed);
            }
            other => panic!("controlled reader receive: {other:?}"),
        }
    }
    panic!("controlled reader frame did not complete within bounded duplex turns");
}

/// Mounted HTTP issues the real reader capability; the existing generic sync
/// transport seam controls WouldBlock deterministically on real duplex sockets.
/// This tests buffered bytes/continuations, not operating-system buffer sizes.
#[test]
fn mounted_blocked_readers_drop_frames_and_chunk_continuations_twice() {
    for change in [
        ReaderChange::Private,
        ReaderChange::Revoke,
        ReaderChange::Expire,
    ] {
        for chunk in [false, true] {
            for _ in 0..2 {
                let mut f = reader_fixture(false, false);
                let mount = ReaderMount::start(&mut f);
                let session = mount.session(&f, true);
                let mut principal = None;
                f.server
                    .update_admission(|_| {
                        principal = Some(f.upgrade(&session).unwrap());
                    })
                    .unwrap();
                let principal = principal.unwrap();
                let object = chunk.then(|| append_reader_fixture_object(&mut f, 96 * 1024));
                let (client, stream) = UnixStream::pair().unwrap();
                stream.set_nonblocking(true).unwrap();
                client
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let blocked = Arc::new(AtomicBool::new(!chunk));
                let attempted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let written = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                let budget = Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));
                let mut connection = f
                    .server
                    .connect(
                        BlockedReaderStream {
                            stream,
                            blocked: blocked.clone(),
                            attempted: attempted.clone(),
                            written: written.clone(),
                            budget: budget.clone(),
                        },
                        principal.clone(),
                    )
                    .unwrap();
                let mut peer = WebSocket::from_raw_socket(client, Role::Client, None);
                send_reader(
                    &mut peer,
                    reader_frame(
                        &session,
                        "hello",
                        json!({"device":principal,"membershipRevision":"0","cursors":[]}),
                    ),
                );
                if chunk {
                    let mut referenced = false;
                    for _ in 0..8 {
                        assert_eq!(connection.poll(), Progress::Advanced);
                        let page = read_controlled_reader(&mut peer, &mut connection);
                        send_reader(
                            &mut peer,
                            reader_frame(&session, "ack", json!({"cursors":[]})),
                        );
                        if page["streams"].as_array().is_some_and(|streams| {
                            streams.iter().any(|s| {
                                s["tail"].as_array().is_some_and(|tail| {
                                    tail.iter().any(|o| o["envelope"].get("objectId").is_some())
                                })
                            })
                        }) {
                            referenced = true;
                            break;
                        }
                    }
                    assert!(
                        referenced,
                        "no large-object reference before chunk transfer"
                    );
                    let bytes = object.as_ref().unwrap();
                    let envelope = tmt_colab_model::object::Envelope::from_json(bytes).unwrap();
                    let header =
                        tmt_colab_model::object::Header::decode(envelope.header()).unwrap();
                    let expected = reader_frame(&session, "chunk", json!({
                        "objectId":header.object_id,"envelopeHash":values::encode_binary(&envelope.hash().unwrap()),
                        "index":0,"count":bytes.len().div_ceil(limits::CHUNK_BYTES),
                        "bytes":values::encode_binary(&bytes[..limits::CHUNK_BYTES])
                    })).to_string();
                    // RFC 6455 unmasked frame with a 16-bit payload length.
                    assert!((126..65536).contains(&expected.len()));
                    budget.store(expected.len() + 4, Ordering::SeqCst);
                    assert_eq!(connection.poll(), Progress::Advanced);
                    let first = read_controlled_reader(&mut peer, &mut connection);
                    assert_eq!(first["type"], "chunk");
                    assert_eq!(first["index"], 0);
                    assert_eq!(
                        values::binary(first["bytes"].as_str().unwrap(), limits::CHUNK_BYTES)
                            .unwrap(),
                        bytes[..limits::CHUNK_BYTES]
                    );
                    assert!(first["count"].as_u64().unwrap() > 2);
                    send_reader(
                        &mut peer,
                        reader_frame(&session, "ack", json!({"cursors":[]})),
                    );
                    blocked.store(true, Ordering::SeqCst);
                }
                let before = written.load(Ordering::SeqCst);
                assert_ne!(connection.poll(), Progress::Closed);
                assert!(
                    attempted.load(Ordering::SeqCst) > 0,
                    "write was never blocked"
                );
                assert_eq!(written.load(Ordering::SeqCst), before);
                change_reader(&mut f, &mount, change);
                blocked.store(false, Ordering::SeqCst);
                budget.store(usize::MAX, Ordering::SeqCst);
                assert_eq!(connection.poll(), Progress::Closed);
                assert_eq!(
                    written.load(Ordering::SeqCst),
                    before,
                    "buffered ciphertext escaped after {change:?}"
                );
                reader_cut_off(&mut peer);
                drop(connection);
                f.server
                    .update_admission(|a| a.0.lock().unwrap().release_reader(&principal))
                    .unwrap();
                assert_eq!(
                    f.read(&principal, &f.scope()),
                    Err(crate::sync::Code::Denied)
                );
            }
        }
    }
}

#[test]
fn mounted_failed_narrowing_keeps_reader_authority_and_retry_cuts_off_twice() {
    for _ in 0..2 {
        let mut f = reader_fixture(false, false);
        let mount = ReaderMount::start(&mut f);
        let session = mount.session(&f, true);
        let mut peer = mount.upgrade(&session).unwrap();
        bootstrap_reader(&mut peer, &session);
        let before = f.scope();
        let head = f
            .store
            .owner_head(&f.key.space_id, &f.key.owner_public())
            .unwrap()
            .unwrap();
        let db = rusqlite::Connection::open(f.root.join("colab/space.db")).unwrap();
        let wraps = db
            .query_row("SELECT count(*) FROM wraps", [], |r| r.get::<_, i64>(0))
            .unwrap();
        db.execute_batch("CREATE TRIGGER fail_reader_narrow BEFORE UPDATE ON recipients WHEN OLD.kind='link' BEGIN SELECT RAISE(ABORT,'injected narrow failure'); END;").unwrap();
        assert!(
            f.try_apply(OwnerAction::Share {
                page: PAGE,
                mode: ShareMode::Private,
                publication: crate::transitions::Publication::Loopback
            })
            .is_err()
        );
        assert_eq!(f.scope(), before);
        assert_eq!(
            f.store
                .owner_head(&f.key.space_id, &f.key.owner_public())
                .unwrap()
                .unwrap(),
            head
        );
        assert_eq!(
            db.query_row("SELECT count(*) FROM wraps", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            wraps
        );
        peer.send(Message::Ping(vec![9].into())).unwrap();
        assert!(matches!(peer.read().unwrap(), Message::Pong(_)));
        let fresh = mount.session(&f, true);
        let mut reader = mount.upgrade(&fresh).unwrap();
        verify_link_wraps(&f, &bootstrap_reader(&mut reader, &fresh), LINK, &SEED);
        db.execute_batch("DROP TRIGGER fail_reader_narrow").unwrap();
        f.share(ShareMode::Private);
        reader_cut_off(&mut peer);
        reader_cut_off(&mut reader);
        assert_eq!(mount.challenge(&f, Some(&f.chain)).0, 403);
    }
}
