//! `StoreLocal` over a real owner Store: real signed statements, chains, wraps and sealed
//! updates, so what it hands the publisher is what a reader would verify.
use super::*;
use crate::{
    keyring::{Keyring, Layout},
    store::{
        Envelope,
        owner::{Device, Mutation, Recipient},
    },
};
use ed25519_dalek::{Signer, SigningKey};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tmt_colab_model::{certificate, values, wrap};
use tmt_extension_objects::sync;

const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const MEMBER: &str = "20000000-0000-4000-8000-000000000001";
const DEVICE: &str = "30000000-0000-4000-8000-000000000001";
const FOREIGN: &str = "30000000-0000-4000-8000-000000000002";
const OP: &str = "40000000-0000-4000-8000-000000000001";
const UID: &str = "owner-uid";
const NOW: u64 = 1_790_000_000_000;

fn recipient() -> Recipient {
    Recipient {
        kind: "member".into(),
        id: MEMBER.into(),
        role: Some("editor".into()),
        signing_key: SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes(),
        encryption_key: wrap::RecipientKey::from_seed(&[8; 32])
            .unwrap()
            .public_key(),
        pages: vec![PAGE.into()],
        revoked: false,
    }
}
fn device(key: &Keyring, issuer: [u8; 32], id: &str) -> Device {
    let signing = SigningKey::from_bytes(&[9; 32]).verifying_key().to_bytes();
    let enc = wrap::RecipientKey::from_seed(&[10; 32])
        .unwrap()
        .public_key();
    let cert = certificate::input(&certificate::Certificate {
        space: &key.space_id,
        issuer_kind: "member",
        issuer_id: MEMBER,
        device_id: id,
        signing_key: &signing,
        encryption_key: &enc,
        membership_revision: "1",
        issued_at: 1,
        expires_at: 100,
    })
    .unwrap();
    let signature = SigningKey::from_bytes(&[7; 32]).sign(&cert).to_bytes();
    Device {
        chain: serde_json::to_vec(&serde_json::json!({"version":1,
            "issuerStatement":values::encode_binary(&issuer),"deviceCertificate":values::encode_binary(&cert),
            "issuerSignature":values::encode_binary(&signature)}))
        .unwrap(),
        revoked: false,
    }
}

struct Fixture {
    root: PathBuf,
    key: Keyring,
    local: StoreLocal,
    /// Latest sequence and hash per stream, which the next update and a cut chain from.
    heads: Vec<(String, u64, [u8; 32])>,
    /// A second connection to the same Store: the test's own writer, like the serve's other
    /// handlers, which share the database but not the worker's handle.
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "tmt-2452-local-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let layout = Layout::open(&root).unwrap();
        let key = Keyring::open(&layout).unwrap();
        let mut store = Store::open(&layout).unwrap();
        store.create_page(PAGE).unwrap();
        let statement = key
            .sign_statement(
                None,
                "member.add",
                &serde_json::to_vec(&serde_json::json!({
                    "memberId":MEMBER,"role":"editor",
                    "signKey":values::encode_binary(&recipient().signing_key),
                    "encKey":values::encode_binary(&recipient().encryption_key),"pages":[PAGE]
                }))
                .unwrap(),
            )
            .unwrap();
        let header = wrap::Header {
            space: key.space_id.clone(),
            page: PAGE.into(),
            epoch: "1".into(),
            recipient_kind: "member".into(),
            recipient_id: MEMBER.into(),
            recipient_key: recipient().encryption_key,
            signer_key: key.owner_public(),
            membership_revision: "1".into(),
        };
        let wrapped = key.seal_wrap(&header, &[11; 32]).unwrap();
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
                    tx.put_recipient(&recipient())?;
                    tx.put_device(&device(&key, statement.hash()?, DEVICE))?;
                    tx.put_device(&device(&key, statement.hash()?, FOREIGN))?;
                    // Only this machine registered DEVICE; FOREIGN's records arrived from elsewhere.
                    tx.put_registration(DEVICE, b"binding", 1)?;
                    tx.put_epoch_secret(PAGE, 1, &[11; 32])?;
                    tx.put_wrap(&wrapped)?;
                    Ok(Vec::new())
                },
            )
            .unwrap();
        let local = StoreLocal::new(
            Store::open(&layout).unwrap(),
            key.owner_public(),
            key.space_id.clone(),
            Bound {
                project: "fixture".into(),
                declaration_digest: "b".repeat(64),
                uid: UID.into(),
            },
            Arc::new(|| NOW),
        );
        Self {
            root,
            key,
            local,
            heads: Vec::new(),
            store,
        }
    }
    fn page(&self, epoch: u64) -> PageRef {
        PageRef {
            space: self.key.space_id.clone(),
            page: PAGE.into(),
            epoch,
        }
    }
    fn head(&self, stream: &str) -> (u64, [u8; 32]) {
        self.heads
            .iter()
            .find(|(id, ..)| id == stream)
            .map_or((0, [0; 32]), |(_, seq, hash)| (*seq, *hash))
    }
    fn sealed(
        &self,
        stream: &str,
        kind: &str,
        namespace: &str,
        seq: u64,
        previous: [u8; 32],
    ) -> object::Envelope {
        object::seal(
            &object::Context {
                space: self.key.space_id.clone(),
                page: PAGE.into(),
                epoch: "1".into(),
                kind: kind.into(),
                namespace: namespace.into(),
                author_device: stream.into(),
                membership_revision: "1".into(),
                stream_seq: seq.to_string(),
                prev_hash: previous,
            },
            &[3; 32],
            &SigningKey::from_bytes(&[5; 32]),
            format!("{kind}-{namespace}-{seq}").as_bytes(),
        )
        .unwrap()
    }
    /// The next update of a stream, alternating namespaces the way real writers do.
    fn append(&mut self, stream: &str) -> Vec<u8> {
        let (last, previous) = self.head(stream);
        let seq = last + 1;
        let (namespace, name) = if seq.is_multiple_of(2) {
            (Namespace::Own, "own")
        } else {
            (Namespace::Content, "content")
        };
        let envelope = self.sealed(stream, "update", name, seq, previous);
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
    fn pair(&mut self, stream: &str, seq: u64) -> [Vec<u8>; 2] {
        let (_, previous) = self.head(stream);
        let mut out = [Vec::new(), Vec::new()];
        for (index, (namespace, name)) in [(Namespace::Content, "content"), (Namespace::Own, "own")]
            .into_iter()
            .enumerate()
        {
            let envelope = self.sealed(stream, "checkpoint", name, seq, previous);
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
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn resources(rows: &[Row]) -> Vec<&str> {
    rows.iter()
        .map(|row| match row {
            Row::Create(document) | Row::Put(document) => document.target.resource.as_str(),
        })
        .collect()
}
fn documents(rows: &[Row], resource: &str) -> Vec<sync::Document> {
    rows.iter()
        .map(|row| match row {
            Row::Create(document) | Row::Put(document) => document,
        })
        .filter(|document| document.target.resource == resource)
        .cloned()
        .collect()
}
fn update_seq(data: &Data) -> u64 {
    match &data.confirm {
        Confirm::Update { seq, .. } => *seq,
        Confirm::Checkpoint { .. } => panic!("expected an update"),
    }
}
fn stuck(result: Result<impl Sized, Fault>) -> bool {
    matches!(result, Err(Fault::Stuck(_)))
}

#[test]
fn the_preamble_is_the_owner_rows_then_each_signed_original_exactly() {
    let mut fixture = Fixture::new();
    let page = fixture.page(1);
    // A chain is published for a stream that has data and was registered here.
    fixture.append(DEVICE);
    fixture.append(FOREIGN);
    let rows = fixture.local.preamble(&page).ok().unwrap();
    assert_eq!(
        resources(&rows),
        ["spaces", "pages", "members", "authority", "authority", "authority"]
    );
    assert!(matches!(rows[0], Row::Create(_)));
    assert!(matches!(rows[1], Row::Put(_)));
    assert!(matches!(rows[2], Row::Create(_)));
    // Each original is split and reassembled to the exact stored bytes: one statement, the
    // chain of the one device registered here (never FOREIGN's), and the owner member's wrap.
    let authority: Vec<(originals::Authority, Vec<u8>)> = documents(&rows, "authority")
        .into_iter()
        .map(|document| originals::authority(&[document]).unwrap())
        .collect();
    let kinds: Vec<_> = authority.iter().map(|(kind, _)| *kind).collect();
    assert_eq!(
        kinds,
        [
            originals::Authority::Membership,
            originals::Authority::Chain,
            originals::Authority::Wrap
        ]
    );
    let chain = fixture
        .store
        .owner_read(&fixture.key.space_id, &fixture.key.owner_public(), |tx| {
            tx.author_chain(DEVICE)
        })
        .unwrap();
    assert_eq!(authority[1].1, chain);
    let statement = fixture
        .store
        .owner_read(&fixture.key.space_id, &fixture.key.owner_public(), |tx| {
            let head = tx.head().cloned().unwrap();
            tx.membership_originals(&head)
        })
        .unwrap();
    assert_eq!(vec![authority[0].1.clone()], statement);
}

#[test]
fn a_page_at_another_epoch_has_nothing_to_publish() {
    let mut fixture = Fixture::new();
    assert!(matches!(
        fixture.local.preamble(&fixture.page(2)).err(),
        Some(Fault::Moved)
    ));
}

#[test]
fn updates_are_due_in_order_and_only_a_confirmation_moves_the_position() {
    let mut fixture = Fixture::new();
    let page = fixture.page(1);
    let sent: Vec<_> = (0..3).map(|_| fixture.append(DEVICE)).collect();
    let first = fixture.local.next(&page).ok().unwrap().unwrap();
    // Reading again without a confirmation offers the same original.
    let again = fixture.local.next(&page).ok().unwrap().unwrap();
    assert_eq!(update_seq(&first), 1);
    assert_eq!(update_seq(&again), 1);
    let [document] = &first.documents[..] else {
        panic!("one record per update")
    };
    assert_eq!(
        decode_envelope(document),
        String::from_utf8(sent[0].clone()).unwrap()
    );
    for (index, bytes) in sent.iter().enumerate() {
        let data = fixture.local.next(&page).ok().unwrap().unwrap();
        assert_eq!(update_seq(&data), index as u64 + 1);
        assert_eq!(
            decode_envelope(&data.documents[0]),
            String::from_utf8(bytes.clone()).unwrap()
        );
        fixture.local.confirm(&page, &data.confirm).ok().unwrap();
    }
    assert!(fixture.local.next(&page).ok().unwrap().is_none());
    // A later update continues from the confirmed position.
    fixture.append(DEVICE);
    let data = fixture.local.next(&page).ok().unwrap().unwrap();
    assert_eq!(update_seq(&data), 4);
}
fn decode_envelope(document: &sync::Document) -> String {
    let fields = documents::decode(Resource::Log, document).unwrap();
    match fields.get("envelope") {
        Some(Field::String(text)) => text.clone(),
        _ => panic!("a log record carries its envelope"),
    }
}

#[test]
fn a_confirmation_for_other_bytes_is_refused_and_the_position_stays() {
    let mut fixture = Fixture::new();
    let page = fixture.page(1);
    fixture.append(DEVICE);
    let data = fixture.local.next(&page).ok().unwrap().unwrap();
    let Confirm::Update { stream, seq, .. } = data.confirm else {
        panic!("an update")
    };
    let wrong = Confirm::Update {
        stream,
        seq,
        hash: [9; 32],
    };
    assert!(stuck(fixture.local.confirm(&page, &wrong)));
    let still = fixture.local.next(&page).ok().unwrap().unwrap();
    assert_eq!(update_seq(&still), 1);
}

#[test]
fn a_compacted_stream_starts_from_its_pair_then_the_retained_tail() {
    let mut fixture = Fixture::new();
    let page = fixture.page(1);
    for _ in 0..2 {
        fixture.append(DEVICE);
    }
    let pair = fixture.pair(DEVICE, 2);
    fixture.append(DEVICE);
    let data = fixture.local.next(&page).ok().unwrap().unwrap();
    let Confirm::Checkpoint {
        seq, content, own, ..
    } = &data.confirm
    else {
        panic!("the pair is due before the tail")
    };
    assert_eq!(*seq, 2);
    assert_eq!(*content, crypto::digest(&pair[0]));
    assert_eq!(*own, crypto::digest(&pair[1]));
    // Both namespaces' originals travel, byte for byte.
    let rebuilt = originals::checkpoint(&data.documents[..data.documents.len() / 2]).unwrap();
    assert_eq!(rebuilt.bytes, pair[0]);
    let rebuilt = originals::checkpoint(&data.documents[data.documents.len() / 2..]).unwrap();
    assert_eq!(rebuilt.bytes, pair[1]);
    fixture.local.confirm(&page, &data.confirm).ok().unwrap();
    let tail = fixture.local.next(&page).ok().unwrap().unwrap();
    assert_eq!(update_seq(&tail), 3);
}

#[test]
fn a_stream_this_machine_did_not_register_is_never_published() {
    let mut fixture = Fixture::new();
    let page = fixture.page(1);
    fixture.append(FOREIGN);
    assert!(fixture.local.next(&page).ok().unwrap().is_none());
    fixture.append(DEVICE);
    let data = fixture.local.next(&page).ok().unwrap().unwrap();
    assert!(matches!(&data.confirm, Confirm::Update { stream, .. } if stream == DEVICE));
}
