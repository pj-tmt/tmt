//! An owner home for tests: a real Store and keyring in a private directory, the owner's
//! membership log, the owner member's wrap and page secret, and certified devices that
//! really sign. Two homes made from one another share an owner key and so one space.
use crate::{
    keyring::{Keyring, Layout},
    object_channel::cloud_sync::PageRef,
    store::{
        Envelope, Namespace, Store, StreamScope,
        owner::{Device, Mutation, Recipient},
    },
};
use ed25519_dalek::SigningKey;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_colab_model::{certificate, object, values, wrap};

pub(crate) const PAGE: &str = "10000000-0000-4000-8000-000000000001";
pub(crate) const SECRET: [u8; 32] = [11; 32];
const OP: &str = "40000000-0000-4000-8000-000000000001";

/// A device certified by the owner member. `registered` devices belong to this home; the others
/// have a chain here but were registered on another machine.
#[derive(Clone, Copy)]
pub(crate) struct DeviceSpec {
    pub id: &'static str,
    /// The seed byte of the device's signing key.
    pub signing: u8,
    pub registered: bool,
}

pub(crate) struct Home {
    pub root: PathBuf,
    pub layout: Layout,
    pub key: Keyring,
    /// The test's own writer, like the serve's other handlers: same database, another handle.
    pub store: Store,
    devices: Vec<DeviceSpec>,
    /// Latest sequence and hash per stream, which the next update and a cut chain from.
    heads: Vec<(String, u64, [u8; 32])>,
}
/// The owner member exactly as the product derives it, so the fold accepts the log.
fn recipient(key: &Keyring) -> Recipient {
    let member = key.management_member().unwrap();
    Recipient {
        kind: "member".into(),
        id: member.id,
        role: Some("editor".into()),
        signing_key: member.signing_key,
        encryption_key: member.encryption_key,
        pages: vec![],
        revoked: false,
    }
}
fn device(key: &Keyring, issuer: [u8; 32], spec: &DeviceSpec) -> Device {
    let signing = SigningKey::from_bytes(&[spec.signing; 32])
        .verifying_key()
        .to_bytes();
    let enc = wrap::RecipientKey::from_seed(&[10; 32])
        .unwrap()
        .public_key();
    let member = recipient(key);
    let certificate = certificate::Certificate {
        space: &key.space_id,
        issuer_kind: "member",
        issuer_id: &member.id,
        device_id: spec.id,
        signing_key: &signing,
        encryption_key: &enc,
        membership_revision: "1",
        issued_at: 1,
        expires_at: 100,
    };
    let cert = certificate::input(&certificate).unwrap();
    let signature = key.sign_device_certificate(&certificate).unwrap();
    Device {
        chain: serde_json::to_vec(&serde_json::json!({"version":1,
            "issuerStatement":values::encode_binary(&issuer),"deviceCertificate":values::encode_binary(&cert),
            "issuerSignature":values::encode_binary(&signature)}))
        .unwrap(),
        revoked: false,
    }
}
impl Home {
    /// A home with the given devices. With `seed_from` it shares that home's owner key.
    pub(crate) fn new(devices: &[DeviceSpec], seed_from: Option<&Home>) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-2452-home-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        if let Some(other) = seed_from {
            let seed = fs::read(other.layout.directory.join("owner.key")).unwrap();
            let path = layout.directory.join("owner.key");
            fs::write(&path, seed).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let key = Keyring::open(&layout).unwrap();
        let mut store = Store::open(&layout).unwrap();
        store.create_page(PAGE).unwrap();
        let statement = key
            .sign_statement(
                None,
                "member.add",
                &serde_json::to_vec(&serde_json::json!({
                    "memberId":recipient(&key).id,"role":"editor",
                    "signKey":values::encode_binary(&recipient(&key).signing_key),
                    "encKey":values::encode_binary(&recipient(&key).encryption_key),"pages":[]
                }))
                .unwrap(),
            )
            .unwrap();
        let wrapped = key
            .seal_wrap(
                &wrap::Header {
                    space: key.space_id.clone(),
                    page: PAGE.into(),
                    epoch: "1".into(),
                    recipient_kind: "member".into(),
                    recipient_id: recipient(&key).id,
                    recipient_key: recipient(&key).encryption_key,
                    signer_key: key.owner_public(),
                    membership_revision: "1".into(),
                },
                &SECRET,
            )
            .unwrap();
        store
            .owner_transaction(
                &key.space_id,
                &key.owner_public(),
                Mutation {
                    operation_id: OP,
                    digest: [7; 32],
                    expected_revision: 0,
                },
                |tx| {
                    tx.append_statement(&statement)?;
                    tx.put_recipient(&recipient(&key))?;
                    for spec in devices {
                        tx.put_device(&device(&key, statement.hash()?, spec))?;
                        if spec.registered {
                            tx.put_registration(spec.id, b"binding", 1)?;
                        }
                    }
                    tx.put_epoch_secret(PAGE, 1, &SECRET)?;
                    tx.put_wrap(&wrapped)?;
                    Ok(Vec::new())
                },
            )
            .unwrap();
        Self {
            root,
            layout,
            key,
            store,
            devices: devices.to_vec(),
            heads: Vec::new(),
        }
    }
    pub(crate) fn page(&self, epoch: u64) -> PageRef {
        PageRef {
            space: self.key.space_id.clone(),
            page: PAGE.into(),
            epoch,
        }
    }
    /// The exact chain bytes this home holds for a device.
    pub(crate) fn chain(&self, device: &str) -> Vec<u8> {
        self.store
            .owner_read(&self.key.space_id, &self.key.owner_public(), |tx| {
                tx.author_chain(device)
            })
            .unwrap()
    }
    /// The exact bytes of the owner log's statements.
    pub(crate) fn statements(&self) -> Vec<Vec<u8>> {
        self.store
            .owner_read(&self.key.space_id, &self.key.owner_public(), |tx| {
                tx.log()?.iter().map(|s| Ok(s.to_json()?)).collect()
            })
            .unwrap()
    }
    pub(crate) fn head(&self, stream: &str) -> (u64, [u8; 32]) {
        self.heads
            .iter()
            .find(|(id, ..)| id == stream)
            .map_or((0, [0; 32]), |(_, seq, hash)| (*seq, *hash))
    }
    pub(crate) fn seal(
        &self,
        stream: &str,
        kind: &str,
        namespace: &str,
        seq: u64,
        previous: [u8; 32],
    ) -> object::Envelope {
        let spec = self.devices.iter().find(|spec| spec.id == stream).unwrap();
        self.seal_as(stream, kind, namespace, seq, previous, spec.signing, "1")
    }
    /// Sealed with any signing seed and epoch, for originals that must be refused.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn seal_as(
        &self,
        stream: &str,
        kind: &str,
        namespace: &str,
        seq: u64,
        previous: [u8; 32],
        signing: u8,
        epoch: &str,
    ) -> object::Envelope {
        object::seal(
            &object::Context {
                space: self.key.space_id.clone(),
                page: PAGE.into(),
                epoch: epoch.into(),
                kind: kind.into(),
                namespace: namespace.into(),
                author_device: stream.into(),
                membership_revision: "1".into(),
                stream_seq: seq.to_string(),
                prev_hash: previous,
            },
            &SECRET,
            &SigningKey::from_bytes(&[signing; 32]),
            format!("{kind}-{namespace}-{seq}").as_bytes(),
        )
        .unwrap()
    }
    /// Another owner statement, so this home's log is longer than a peer's.
    pub(crate) fn retention(&mut self, days: u64, operation: &str) {
        let revision = self
            .store
            .owner_head(&self.key.space_id, &self.key.owner_public())
            .unwrap()
            .unwrap()
            .revision;
        let key = &self.key;
        self.store
            .owner_transaction(
                &key.space_id,
                &key.owner_public(),
                Mutation {
                    operation_id: operation,
                    digest: [days as u8; 32],
                    expected_revision: revision,
                },
                |tx| {
                    tx.append_statement(&key.sign_statement(
                        tx.head(),
                        "retention.set",
                        &serde_json::to_vec(&serde_json::json!({"pageId":PAGE,"days":days}))?,
                    )?)?;
                    Ok(Vec::new())
                },
            )
            .unwrap();
    }
    /// The next update of a stream, alternating namespaces the way real writers do.
    pub(crate) fn append(&mut self, stream: &str) -> Vec<u8> {
        let (last, previous) = self.head(stream);
        let seq = last + 1;
        let (namespace, name) = if seq.is_multiple_of(2) {
            (Namespace::Own, "own")
        } else {
            (Namespace::Content, "content")
        };
        let envelope = self.seal(stream, "update", name, seq, previous);
        let bytes = envelope.to_json().unwrap();
        let hash = envelope.hash().unwrap();
        self.store
            .append(&Envelope {
                scope: StreamScope {
                    page: PAGE,
                    epoch: 1,
                    stream,
                },
                namespace,
                seq,
                hash,
                previous,
                bytes: &bytes,
            })
            .unwrap();
        self.heads.retain(|(id, ..)| id != stream);
        self.heads.push((stream.into(), seq, hash));
        bytes
    }
    /// Both namespaces' checkpoints at `seq`, which prunes the covered prefix.
    pub(crate) fn pair(&mut self, stream: &str, seq: u64) -> [Vec<u8>; 2] {
        let (_, previous) = self.head(stream);
        let mut out = [Vec::new(), Vec::new()];
        for (index, (namespace, name)) in [(Namespace::Content, "content"), (Namespace::Own, "own")]
            .into_iter()
            .enumerate()
        {
            let envelope = self.seal(stream, "checkpoint", name, seq, previous);
            let bytes = envelope.to_json().unwrap();
            self.store
                .checkpoint(&Envelope {
                    scope: StreamScope {
                        page: PAGE,
                        epoch: 1,
                        stream,
                    },
                    namespace,
                    seq,
                    hash: envelope.hash().unwrap(),
                    previous,
                    bytes: &bytes,
                })
                .unwrap();
            out[index] = bytes;
        }
        out
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
