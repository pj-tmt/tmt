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
            home.key.local_writer().unwrap().0,
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
fn the_root_local_writer_is_this_machines_stream_though_it_has_no_registration() {
    let mut fixture = Fixture::new();
    let page = fixture.page(1);
    let writer = fixture.key.local_writer().unwrap().0;
    // What the CLI and every browser save write: the keyring's writer, a chain, no registration.
    fixture.append_as_local_writer();
    fixture.append(FOREIGN);
    let rows = fixture.local.preamble(&page).ok().unwrap();
    let chains: Vec<_> = documents(&rows, "authority")
        .into_iter()
        .filter_map(|document| originals::authority(&[document]).ok())
        .filter(|(kind, _)| *kind == originals::Authority::Chain)
        .map(|(_, bytes)| bytes)
        .collect();
    assert!(
        chains.contains(&fixture.chain(&writer)),
        "the writer's chain is published"
    );
    assert!(
        !chains.contains(&fixture.chain(FOREIGN)),
        "a retained foreign chain is not"
    );
    let data = fixture.local.next(&page).ok().unwrap().unwrap();
    assert!(matches!(&data.confirm, Confirm::Update { stream, .. } if *stream == writer));
    // Once it is confirmed nothing else is due: the foreign stream still is not published.
    let Confirm::Update { stream, seq, hash } = data.confirm else {
        unreachable!()
    };
    fixture
        .local
        .confirm(&page, &Confirm::Update { stream, seq, hash })
        .ok()
        .unwrap();
    assert!(fixture.local.next(&page).ok().unwrap().is_none());
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

#[test]
fn the_page_rows_expiry_is_recorded_only_once_the_cloud_confirms_it() {
    let mut fixture = Fixture::new();
    let page = fixture.page(1);
    fixture.append(DEVICE);
    let recorded = |fixture: &Fixture| -> Vec<(String, String, u64)> {
        let db = rusqlite::Connection::open(fixture.layout.directory.join("space.db")).unwrap();
        let mut query = db
            .prepare("SELECT page,epoch,expires_at_ms FROM cloud_page_rows")
            .unwrap();
        query
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? as u64))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let expires = fixture
        .local
        .preamble(&page)
        .ok()
        .unwrap()
        .into_iter()
        .find_map(|row| match row {
            Row::Put { expires_ms, .. } => Some(expires_ms),
            Row::Create(_) => None,
        })
        .unwrap();
    assert_eq!(expires, NOW + 30 * DAY_MS);
    // Reading the rows to write records nothing: the cloud has not confirmed them.
    assert!(recorded(&fixture).is_empty());
    fixture.local.page_row_confirmed(&page, expires).unwrap();
    assert_eq!(
        recorded(&fixture),
        [(page.page.clone(), "1".to_owned(), expires)]
    );
    // A later confirmation replaces it, and one for another epoch than the page's is ignored.
    fixture
        .local
        .page_row_confirmed(&page, expires + DAY_MS)
        .unwrap();
    let other_epoch = PageRef {
        epoch: 2,
        ..page.clone()
    };
    fixture
        .local
        .page_row_confirmed(&other_epoch, expires + 9 * DAY_MS)
        .unwrap();
    assert_eq!(
        recorded(&fixture),
        [(page.page.clone(), "1".to_owned(), expires + DAY_MS)]
    );
}

const LINK: &str = "50000000-0000-4000-8000-000000000001";
const LINK_SEED: [u8; 32] = [31; 32];

/// The owner adds a viewer link for the page, as `link.add` leaves it in the Store: the signed
/// statement, the recipient, the wrap of the page secret, and (when `token`) the admission token.
fn add_link(fixture: &mut Fixture, token: bool) -> String {
    let keys =
        tmt_colab_model::link::Keys::derive(&LINK_SEED, &fixture.key.space_id, LINK).unwrap();
    let recipient = crate::store::owner::Recipient {
        kind: "link".into(),
        id: LINK.into(),
        role: Some("viewer".into()),
        signing_key: keys.signing_public(),
        encryption_key: keys.recipient().public_key(),
        pages: vec![crate::object_channel::cloud_sync::fixture::home::PAGE.into()],
        revoked: false,
    };
    let payload = serde_json::to_vec(&serde_json::json!({
        "linkId": LINK, "role": "viewer",
        "linkSignKey": tmt_colab_model::values::encode_binary(&recipient.signing_key),
        "linkEncKey": tmt_colab_model::values::encode_binary(&recipient.encryption_key),
        "pages": recipient.pages,
    }))
    .unwrap();
    let home = &mut fixture.home;
    let head = home
        .store
        .owner_head(&home.key.space_id, &home.key.owner_public())
        .unwrap()
        .unwrap();
    let key = &home.key;
    home.store
        .owner_transaction(
            &key.space_id,
            &key.owner_public(),
            crate::store::owner::Mutation {
                operation_id: "40000000-0000-4000-8000-0000000000a1",
                digest: [8; 32],
                expected_revision: head.revision,
            },
            |tx| {
                tx.append_statement(&key.sign_statement(tx.head(), "link.add", &payload)?)?;
                let revision = tx.head().unwrap().revision;
                tx.put_recipient(&recipient)?;
                if token {
                    tx.put_link_admission(LINK, &keys.rules_token())?;
                }
                tx.put_wrap(&key.seal_wrap(
                    &tmt_colab_model::wrap::Header {
                        space: key.space_id.clone(),
                        page: crate::object_channel::cloud_sync::fixture::home::PAGE.into(),
                        epoch: "1".into(),
                        recipient_kind: "link".into(),
                        recipient_id: LINK.into(),
                        recipient_key: recipient.encryption_key,
                        signer_key: key.owner_public(),
                        membership_revision: revision.to_string(),
                    },
                    &crate::object_channel::cloud_sync::fixture::home::SECRET,
                )?)?;
                Ok(Vec::new())
            },
        )
        .unwrap();
    keys.rules_token()
}
/// The owner keeps the reader devices the link's seed derives, one per transport, as `link.add`
/// does with the seed borrowed: device rows naming the `link.add` statement, never registrations.
fn add_link_devices(fixture: &mut Fixture) -> Vec<Vec<u8>> {
    let home = &mut fixture.home;
    let key = &home.key;
    let head = home
        .store
        .owner_head(&key.space_id, &key.owner_public())
        .unwrap()
        .unwrap();
    let chains: Vec<Vec<u8>> = tmt_colab_model::link::Transport::ALL
        .into_iter()
        .map(|transport| {
            tmt_colab_model::link::device_chain(
                &LINK_SEED,
                &key.space_id,
                LINK,
                &head.revision.to_string(),
                &head.hash,
                transport,
            )
            .unwrap()
        })
        .collect();
    home.store
        .device_transaction(&key.space_id, &key.owner_public(), |tx| {
            for chain in &chains {
                tx.put_device(&crate::store::owner::Device {
                    chain: chain.clone(),
                    revoked: false,
                })?;
            }
            Ok(Vec::new())
        })
        .unwrap();
    chains
}
/// The owner removes the link: its recipient is revoked and its admission token is dropped.
fn end_link(fixture: &mut Fixture) {
    let home = &mut fixture.home;
    let key = &home.key;
    let head = home
        .store
        .owner_head(&key.space_id, &key.owner_public())
        .unwrap()
        .unwrap();
    home.store
        .owner_transaction(
            &key.space_id,
            &key.owner_public(),
            crate::store::owner::Mutation {
                operation_id: "40000000-0000-4000-8000-0000000000a2",
                digest: [9; 32],
                expected_revision: head.revision,
            },
            |tx| {
                let mut link = tx.recipient("link", LINK)?.unwrap();
                link.revoked = true;
                tx.put_recipient(&link)?;
                tx.drop_link_admission(LINK)?;
                tx.append_statement(&key.sign_statement(
                    tx.head(),
                    "link.remove",
                    &serde_json::to_vec(&serde_json::json!({"linkId": LINK, "cuts": []}))?,
                )?)?;
                Ok(Vec::new())
            },
        )
        .unwrap();
}

#[test]
fn a_live_link_is_a_row_and_wraps_for_the_link_and_is_tracked_for_its_removal() {
    let mut fixture = Fixture::new();
    let page = fixture.page(1);
    let token = add_link(&mut fixture, true);
    let rows = fixture.local.preamble(&page).ok().unwrap();
    assert_eq!(
        resources(&rows),
        [
            "spaces",
            "pages",
            "members",
            "links",
            "authority",
            "authority",
            "authority",
            "authority"
        ],
        "the link row follows the owner's rows; then two statements, the owner's wrap and the link's"
    );
    let [link] = &documents(&rows, "links")[..] else {
        panic!("one link row")
    };
    let fields = documents::decode(Resource::Links, link).unwrap();
    for (key, value) in [
        ("link", LINK),
        ("role", "viewer"),
        ("token", token.as_str()),
    ] {
        assert_eq!(fields.get(key), Some(&Field::String(value.into())), "{key}");
    }
    let wraps: Vec<_> = documents(&rows, "authority")
        .into_iter()
        .filter(|document| {
            documents::decode(Resource::Authority, document)
                .unwrap()
                .get("recipient")
                == Some(&Field::String(format!("link:{LINK}")))
        })
        .collect();
    let [wrap] = &wraps[..] else {
        panic!("one wrap for the link")
    };
    let (kind, bytes) = originals::authority(std::slice::from_ref(wrap)).unwrap();
    assert_eq!(kind, originals::Authority::Wrap);
    let header = tmt_colab_model::wrap::Envelope::from_json(&bytes)
        .unwrap()
        .header()
        .unwrap();
    assert_eq!(
        (header.recipient_kind.as_str(), header.recipient_id.as_str()),
        ("link", LINK)
    );
    assert_eq!(
        fixture.local.store.link_publications(&page.page).unwrap(),
        [LINK],
        "recorded before the row is written"
    );
    assert!(fixture.local.retirements(&page).ok().unwrap().is_empty());
    // The owner's own member row says what it is: a constant seq beside the null link.
    let [member] = &documents(&rows, "members")[..] else {
        panic!("the owner's member row")
    };
    assert_eq!(
        documents::decode(Resource::Members, member)
            .unwrap()
            .get("seq"),
        Some(&Field::number(1).unwrap())
    );
}

#[test]
fn a_live_links_device_chain_is_published_after_the_writers_and_an_ended_links_is_not() {
    let mut fixture = Fixture::new();
    let page = fixture.page(1);
    fixture.append(DEVICE);
    add_link(&mut fixture, true);
    let mut link_chains = add_link_devices(&mut fixture);
    link_chains.sort();
    let chains = |rows: &[Row]| -> Vec<Vec<u8>> {
        documents(rows, "authority")
            .into_iter()
            .filter_map(|document| originals::authority(&[document]).ok())
            .filter(|(kind, _)| *kind == originals::Authority::Chain)
            .map(|(_, bytes)| bytes)
            .collect()
    };
    let writer = fixture
        .store
        .owner_read(&fixture.key.space_id, &fixture.key.owner_public(), |tx| {
            tx.author_chain(DEVICE)
        })
        .unwrap();
    // No stream of the link's exists here, yet the chains of its devices, one per transport, are
    // in the cloud for whoever follows them.
    let rows = fixture.local.preamble(&page).ok().unwrap();
    let published = chains(&rows);
    assert_eq!(published[0], writer);
    let mut link_published = published[1..].to_vec();
    link_published.sort();
    assert_eq!(link_published, link_chains);
    assert_eq!(link_published.len(), 2);
    // The link ends: it is no longer live, so its chains are not published again.
    end_link(&mut fixture);
    let rows = fixture.local.preamble(&page).ok().unwrap();
    assert_eq!(chains(&rows), [writer]);
}

#[test]
fn a_link_without_a_token_is_not_published() {
    let mut fixture = Fixture::new();
    let page = fixture.page(1);
    add_link(&mut fixture, false);
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
        ],
        "two statements and the owner's wrap, nothing for the link"
    );
    assert!(
        fixture
            .local
            .store
            .link_publications(&page.page)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_link_that_ends_is_retired_once_and_forgotten() {
    let mut fixture = Fixture::new();
    let page = fixture.page(1);
    add_link(&mut fixture, true);
    fixture.local.preamble(&page).ok().unwrap();
    end_link(&mut fixture);
    // The ended link is no longer published, and its cloud rows are due for removal.
    let rows = fixture.local.preamble(&page).ok().unwrap();
    assert!(!resources(&rows).contains(&"links"));
    let retirements = fixture.local.retirements(&page).ok().unwrap();
    let [retirement] = &retirements[..] else {
        panic!("one retirement")
    };
    assert_eq!(retirement.link, LINK);
    assert_eq!(retirement.target.resource, "links");
    assert_eq!(
        retirement.target.id,
        format!("{}_{}_{LINK}", page.space, page.page)
    );
    assert_eq!(
        retirement.scope["link"],
        serde_json::json!({"stringValue": LINK})
    );
    // It stays due across calls, until it is recorded as done.
    assert_eq!(fixture.local.retirements(&page).ok().unwrap().len(), 1);
    fixture.local.retired(&page, LINK).ok().unwrap();
    assert!(fixture.local.retirements(&page).ok().unwrap().is_empty());
    assert!(
        fixture
            .local
            .store
            .link_publications(&page.page)
            .unwrap()
            .is_empty()
    );
}
