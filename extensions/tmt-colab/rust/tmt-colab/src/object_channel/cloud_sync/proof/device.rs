//! Fixture devices that behave like paired browsers: they register through the serve's own
//! registration and then sync over its Unix socket, so every update reaches the serve by the
//! path a real browser's does.
use crate::{
    keyring::{Keyring, Layout},
    registration::Registration,
    store::Store,
};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};
use tmt_colab_model::{certificate, framing, object, values, wrap};
use tungstenite::{Message, WebSocket, protocol::Role};

/// The seed of the Remote-paired owner browser whose key vouches for the fixture devices.
const PAIRED: u8 = 9;
const UPGRADE: &str = "Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: colab-sync-v1\r\n";

pub(super) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}
fn context(id: &str) -> String {
    json!({"deviceId":id,"kind":"browser","origin":"http://127.0.0.1:1","name":"proof",
        "publicKey":values::encode_binary(SigningKey::from_bytes(&[PAIRED;32]).verifying_key().as_bytes()),
        "owner":true,"grantRevision":1})
    .to_string()
}
fn encryption(seed: u8) -> [u8; 32] {
    wrap::RecipientKey::from_seed(&[seed; 32])
        .unwrap()
        .public_key()
}
fn registration_body(id: &str, seed: u8) -> Vec<u8> {
    let issued = now();
    let cert = |purpose: &str, key: &[u8; 32]| {
        let input = framing::frame(&[
            b"tmt-ext-cert-v1",
            b"colab",
            purpose.as_bytes(),
            key,
            issued.to_string().as_bytes(),
        ])
        .unwrap();
        json!({"publicKey":values::encode_binary(key),"issuedAtMs":issued,
            "signature":values::encode_binary(&SigningKey::from_bytes(&[PAIRED;32]).sign(&input).to_bytes())})
    };
    let signing = SigningKey::from_bytes(&[seed; 32])
        .verifying_key()
        .to_bytes();
    serde_json::to_vec(
        &json!({"deviceId":id,"sign":cert("sign",&signing),"enc":cert("enc",&encryption(seed))}),
    )
    .unwrap()
}
/// Register devices (id and signing seed) in a stopped home, through the serve's own
/// registration, so each has a real certificate chain signed by the owner member.
pub(super) fn register(layout: &Layout, program: &Path, devices: &[(&str, u8)]) {
    let mut registration = Registration::with_decoder_config(
        Store::open(layout).unwrap(),
        Keyring::read(layout).unwrap(),
        crate::decoder::Config::new(program.to_owned()),
    )
    .unwrap();
    for (id, seed) in devices {
        registration
            .register(Some(&context(id)), &registration_body(id, *seed), now())
            .unwrap();
    }
}

/// One sealed update of a device's stream.
pub(super) struct Sealed {
    pub bytes: Vec<u8>,
    pub hash: [u8; 32],
}
pub(super) struct Writer<'a> {
    pub space: &'a str,
    pub page: &'a str,
    pub membership_revision: u64,
    pub secret: [u8; 32],
}
impl Writer<'_> {
    /// Sealed with any signing seed and epoch, so a forgery is built the same way as a good one.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn seal_as(
        &self,
        device: &str,
        namespace: &str,
        seq: u64,
        previous: [u8; 32],
        seed: u8,
        epoch: &str,
        payload: &[u8],
    ) -> Sealed {
        let envelope = object::seal(
            &object::Context {
                space: self.space.into(),
                page: self.page.into(),
                epoch: epoch.into(),
                kind: "update".into(),
                namespace: namespace.into(),
                author_device: device.into(),
                membership_revision: self.membership_revision.to_string(),
                stream_seq: seq.to_string(),
                prev_hash: previous,
            },
            &self.secret,
            &SigningKey::from_bytes(&[seed; 32]),
            payload,
        )
        .unwrap();
        Sealed {
            bytes: envelope.to_json().unwrap(),
            hash: envelope.hash().unwrap(),
        }
    }
}

/// A device connected to a serve's socket with the forwarded owner context.
pub(super) struct Peer {
    ws: WebSocket<UnixStream>,
    id: String,
    space: String,
    page: String,
    epoch: String,
    /// Frames read while waiting for something else, kept in order for the next reader.
    pending: std::collections::VecDeque<Value>,
}
impl Peer {
    pub(super) fn connect(socket: &Path, id: &str, space: &str, page: &str, epoch: &str) -> Self {
        let mut stream = UnixStream::connect(socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        let request = format!(
            "GET /sync HTTP/1.1\r\nHost: 127.0.0.1:1\r\ntmt-device-context: {}\r\n{UPGRADE}\r\n",
            context(id)
        );
        stream.write_all(request.as_bytes()).unwrap();
        let mut head = Vec::new();
        let mut byte = [0; 1];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        let head = String::from_utf8(head).unwrap();
        assert!(head.starts_with("HTTP/1.1 101"), "{head}");
        Self {
            ws: WebSocket::from_raw_socket(stream, Role::Client, None),
            id: id.into(),
            space: space.into(),
            page: page.into(),
            epoch: epoch.into(),
            pending: Default::default(),
        }
    }
    fn frame(&self, kind: &str, fields: Value) -> Value {
        let mut frame =
            json!({"version":1,"type":kind,"space":self.space,"page":self.page,"epoch":self.epoch});
        frame
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        frame
    }
    fn send(&mut self, kind: &str, fields: Value) {
        let frame = self.frame(kind, fields);
        self.ws
            .send(Message::Text(frame.to_string().into()))
            .unwrap();
    }
    pub(super) fn receive(&mut self) -> Value {
        if let Some(frame) = self.pending.pop_front() {
            return frame;
        }
        self.read()
    }
    fn read(&mut self) -> Value {
        loop {
            // The serve's keepalive pings are not frames of the protocol.
            if let Message::Text(text) = self.ws.read().unwrap() {
                return serde_json::from_str(text.as_str()).unwrap();
            }
        }
    }
    /// Hello, then take every catch-up page this device is owed, acknowledging each. Opening a
    /// page this way is what makes it the serve's active page.
    pub(super) fn hello(&mut self, membership_revision: u64) -> Vec<Value> {
        let id = self.id.clone();
        self.send(
            "hello",
            json!({"membershipRevision":membership_revision.to_string(),"device":id,"cursors":[]}),
        );
        let mut pages = Vec::new();
        loop {
            let page = self.receive();
            if page["type"] != "catchup" {
                pages.push(page);
                continue;
            }
            let more = page["more"].as_bool().unwrap_or(false);
            pages.push(page);
            self.send("ack", json!({"cursors":[]}));
            if !more {
                return pages;
            }
        }
    }
    /// Follow the page live, so what the serve stores from anywhere is announced to this device.
    pub(super) fn subscribe(&mut self) {
        self.send("subscribe", json!({"cursors":[]}));
    }
    /// The next frame about a stream's update, acknowledging any catch-up page on the way.
    pub(super) fn next_update(&mut self, stream: &str) -> Value {
        loop {
            let frame = self.receive();
            match frame["type"].as_str() {
                Some("catchup") => self.send("ack", json!({"cursors":[]})),
                Some("broadcast") if frame["streamId"] == stream => return frame,
                _ => {}
            }
        }
    }
    /// Append one sealed update and wait for its receipt (or the serve's refusal).
    pub(super) fn append(&mut self, seq: u64, sealed: &Sealed) -> Value {
        let id = self.id.clone();
        self.send(
            "append",
            json!({"streamId":id,"seq":seq.to_string(),
                "envelopeHash":values::encode_binary(&sealed.hash),
                "envelope":values::encode_binary(&sealed.bytes)}),
        );
        loop {
            let frame = self.read();
            if frame["type"] == "receipt" || frame["type"] == "error" {
                return frame;
            }
            self.pending.push_back(frame);
        }
    }
}

/// A member of the page and the Firebase identity that writes for them.
pub(super) struct Member {
    pub id: &'static str,
    pub signing_seed: u8,
    pub encryption_seed: u8,
}
impl Member {
    pub(super) fn signing_key(&self) -> String {
        values::encode_binary(
            SigningKey::from_bytes(&[self.signing_seed; 32])
                .verifying_key()
                .as_bytes(),
        )
    }
    pub(super) fn encryption_key(&self) -> String {
        values::encode_binary(&encryption(self.encryption_seed))
    }
    /// The chain of a device this member certifies, as the member's own client would hold it:
    /// the certificate is signed by the member's key under the statement that added them.
    pub(super) fn chain(
        &self,
        space: &str,
        statement_hash: [u8; 32],
        membership_revision: u64,
        device: &str,
        device_seed: u8,
    ) -> Vec<u8> {
        let signing = SigningKey::from_bytes(&[device_seed; 32])
            .verifying_key()
            .to_bytes();
        let revision = membership_revision.to_string();
        let issued = now();
        let certificate = certificate::Certificate {
            space,
            issuer_kind: "member",
            issuer_id: self.id,
            device_id: device,
            signing_key: &signing,
            encryption_key: &encryption(device_seed),
            membership_revision: &revision,
            issued_at: issued - 60_000,
            expires_at: issued + 30 * 86_400_000,
        };
        let input = certificate::input(&certificate).unwrap();
        let signature = SigningKey::from_bytes(&[self.signing_seed; 32]).sign(&input);
        serde_json::to_vec(&json!({"version":1,
            "issuerStatement":values::encode_binary(&statement_hash),
            "deviceCertificate":values::encode_binary(&input),
            "issuerSignature":values::encode_binary(&signature.to_bytes())}))
        .unwrap()
    }
}
