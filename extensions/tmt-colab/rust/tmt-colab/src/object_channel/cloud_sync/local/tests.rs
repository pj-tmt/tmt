//! `StoreLocal` over a real owner Store: real signed statements, chains, wraps and sealed
//! updates, so what it hands the publisher is what a reader would verify.
use super::*;
use crate::object_channel::cloud_sync::fixture::home::{DeviceSpec, Home};
use std::ops::{Deref, DerefMut};
use tmt_extension_objects::sync;

const DEVICE: &str = "30000000-0000-4000-8000-000000000001";
const FOREIGN: &str = "30000000-0000-4000-8000-000000000002";
const UID: &str = "owner-uid";
const NOW: u64 = 1_790_000_000_000;

struct Fixture {
    home: Home,
    local: StoreLocal,
}
impl Fixture {
    fn new() -> Self {
        // Only this machine registered DEVICE; FOREIGN's records arrived from elsewhere.
        let home = Home::new(
            &[
                DeviceSpec {
                    id: DEVICE,
                    signing: 9,
                    registered: true,
                },
                DeviceSpec {
                    id: FOREIGN,
                    signing: 19,
                    registered: false,
                },
            ],
            None,
        );
        let local = StoreLocal::new(
            Store::open(&home.layout).unwrap(),
            home.key.owner_public(),
            home.key.space_id.clone(),
            Bound {
                project: "fixture".into(),
                declaration_digest: "b".repeat(64),
                uid: UID.into(),
            },
            Arc::new(|| NOW),
        );
        Self { home, local }
    }
}
impl Deref for Fixture {
    type Target = Home;
    fn deref(&self) -> &Home {
        &self.home
    }
}
impl DerefMut for Fixture {
    fn deref_mut(&mut self) -> &mut Home {
        &mut self.home
    }
}

fn resources(rows: &[Row]) -> Vec<&str> {
    rows.iter()
        .map(|row| match row {
            Row::Create(document) | Row::Put { document, .. } => document.target.resource.as_str(),
        })
        .collect()
}
fn documents(rows: &[Row], resource: &str) -> Vec<sync::Document> {
    rows.iter()
        .map(|row| match row {
            Row::Create(document) | Row::Put { document, .. } => document,
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
        [
            "spaces",
            "pages",
            "members",
            "authority",
            "authority",
            "authority"
        ]
    );
    assert!(matches!(rows[0], Row::Create(_)));
    assert!(matches!(rows[1], Row::Put { .. }));
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

#[test]
fn the_cloud_lifetime_follows_the_retention_within_the_declared_maximum_and_forever_takes_it() {
    for (retention, days) in [
        (Some(0), 1),
        (Some(1), 1),
        (Some(30), 30),
        (Some(364), 364),
        (Some(365), 365),
        (Some(366), 365),
        (Some(100_000), 365),
        (None, 365),
    ] {
        assert_eq!(cloud_days(retention), days, "{retention:?}");
    }
    // The row is written again on activity after at most a day, and after half of a short life.
    let day = std::time::Duration::from_secs(24 * 3600);
    assert_eq!(row_refresh(365), day);
    assert_eq!(row_refresh(30), day);
    assert_eq!(row_refresh(2), day);
    assert_eq!(row_refresh(1), day / 2);
}

#[test]
fn a_retention_override_moves_the_page_rows_expiry_and_it_is_the_rows_own_refresh() {
    let mut fixture = Fixture::new();
    let page = fixture.page(1);
    fixture.append(DEVICE);
    let page_row = |rows: Vec<Row>| {
        rows.into_iter()
            .find_map(|row| match row {
                Row::Put {
                    document, refresh, ..
                } => Some((document, refresh)),
                Row::Create(_) => None,
            })
            .unwrap()
    };
    let expiry =
        |days: u64| serde_json::to_value(Field::timestamp(NOW + days * DAY_MS).unwrap()).unwrap();
    // The policy's own default is 30 days.
    let (document, refresh) = page_row(fixture.local.preamble(&page).ok().unwrap());
    assert_eq!(document.fields["expiresAt"], expiry(30));
    assert_eq!(refresh, row_refresh(30));
    // An owner override changes both the expiry and, with the policy, what is rewritten.
    fixture.retention(90, "40000000-0000-4000-8000-000000000002");
    let (document, _) = page_row(fixture.local.preamble(&page).ok().unwrap());
    assert_eq!(document.fields["expiresAt"], expiry(90));
    fixture.retention(1000, "40000000-0000-4000-8000-000000000003");
    let (document, _) = page_row(fixture.local.preamble(&page).ok().unwrap());
    assert_eq!(document.fields["expiresAt"], expiry(365));
}
