//! The cloud reads against an in-memory Firestore on the real bus: paging, exact bytes,
//! catch-up and refusals.
use super::*;
use crate::object_channel::{
    cloud_sync::fixture::{BINDING, Firestore, SPACE, ScriptedRemote, peer, sealed_update},
    documents::{
        Field,
        originals::{self, Routing},
    },
};
use std::sync::atomic::Ordering;

const PAGE: &str = "10000000-0000-4000-8000-000000000001";
const STREAM: &str = "20000000-0000-4000-8000-000000000001";
fn page() -> PageRef {
    PageRef {
        space: SPACE.into(),
        page: PAGE.into(),
        epoch: 1,
    }
}
fn routing() -> Routing<'static> {
    Routing {
        space: SPACE,
        page: PAGE,
        epoch: 1,
        uid: "owner",
        expires_at: Field::timestamp(1_790_000_000_000).unwrap(),
    }
}
struct Rig {
    reader: Reader,
    firestore: Firestore,
    _remote: ScriptedRemote,
    _peer: crate::object_channel::cloud_sync::fixture::Peer,
}
fn rig() -> Rig {
    let mut peer = peer();
    let firestore = Firestore::default();
    let answer = firestore.clone();
    let remote = ScriptedRemote::serve(peer.remote.take().unwrap(), move |call, id| {
        answer.answer(call, id)
    });
    let client = peer.channels.client.lock().unwrap().clone().unwrap();
    Rig {
        reader: Reader::new(client, BINDING.into()),
        firestore,
        _remote: remote,
        _peer: peer,
    }
}
/// Publish updates `from..=to` of the stream as log records, returning their exact bytes.
fn publish(rig: &Rig, from: u64, to: u64) -> Vec<Vec<u8>> {
    let mut previous = [0; 32];
    let mut out = Vec::new();
    for seq in 1..=to {
        let (bytes, hash) = sealed_update(SPACE, PAGE, STREAM, seq, previous);
        previous = hash;
        if seq >= from {
            rig.firestore
                .put(&originals::log_document(&routing(), &bytes).unwrap());
            out.push(bytes);
        }
    }
    out
}

#[test]
fn a_tail_is_the_contiguous_originals_after_the_position_one_provider_page_at_a_time() {
    let rig = rig();
    rig.firestore.page.store(2, Ordering::SeqCst);
    let sent = publish(&rig, 1, 5);
    let Tail::Originals(first) = rig.reader.tail(&page(), STREAM, 0).unwrap() else {
        panic!("originals expected")
    };
    assert_eq!(first, vec![(1, sent[0].clone()), (2, sent[1].clone())]);
    let Tail::Originals(next) = rig.reader.tail(&page(), STREAM, 2).unwrap() else {
        panic!("originals expected")
    };
    assert_eq!(next, vec![(3, sent[2].clone()), (4, sent[3].clone())]);
    assert_eq!(
        rig.reader.tail(&page(), STREAM, 5).unwrap(),
        Tail::Originals(Vec::new())
    );
}

#[test]
fn a_sequence_the_provider_no_longer_holds_needs_a_bootstrap() {
    let rig = rig();
    publish(&rig, 4, 6);
    assert_eq!(
        rig.reader.tail(&page(), STREAM, 0).unwrap(),
        Tail::CatchUpRequired
    );
    assert!(matches!(
        rig.reader.tail(&page(), STREAM, 3).unwrap(),
        Tail::Originals(found) if found.len() == 3
    ));
}

#[test]
fn a_record_that_disagrees_with_its_signed_bytes_is_malformed_and_a_refusal_is_typed() {
    let rig = rig();
    publish(&rig, 1, 2);
    let key = (
        "log".to_owned(),
        originals::log_document(
            &routing(),
            &sealed_update(SPACE, PAGE, STREAM, 1, [0; 32]).0,
        )
        .unwrap()
        .target
        .id,
    );
    // The routing carries another hash than the signed bytes have.
    rig.firestore
        .docs
        .lock()
        .unwrap()
        .get_mut(&key)
        .unwrap()
        .insert(
            "hash".into(),
            serde_json::json!({"stringValue": tmt_colab_model::values::encode_binary(&[9; 32])}),
        );
    assert!(matches!(
        rig.reader.tail(&page(), STREAM, 0),
        Err(Fault::Stuck(_))
    ));
    rig.firestore.faults.lock().unwrap().refuse_next = Some(sync::Error {
        code: sync::Code::Budget,
        state: None,
        reset_at_ms: Some(1),
        retry_after_ms: Some(1_500),
    });
    assert_eq!(
        rig.reader.tail(&page(), STREAM, 0).err(),
        Some(Fault::Budget(1_500))
    );
}

#[test]
fn authority_is_discovered_across_pages_and_reassembled_to_the_exact_bytes() {
    let rig = rig();
    rig.firestore.page.store(2, Ordering::SeqCst);
    // A large statement spans pieces; two chains are single records.
    let statement = vec![b's'; 70_000];
    let chains = [vec![b'a'; 300], vec![b'b'; 400]];
    for document in
        originals::authority_documents(&routing(), originals::Authority::Membership, "", &statement)
            .unwrap()
    {
        rig.firestore.put(&document);
    }
    for chain in &chains {
        for document in
            originals::authority_documents(&routing(), originals::Authority::Chain, "", chain)
                .unwrap()
        {
            rig.firestore.put(&document);
        }
    }
    assert_eq!(
        rig.reader.authority(&page(), "membership").unwrap(),
        vec![statement]
    );
    let mut found = rig.reader.authority(&page(), "chain").unwrap();
    found.sort();
    assert_eq!(found, chains);
    // A partial set of pieces is refused rather than reassembled short.
    rig.firestore
        .docs
        .lock()
        .unwrap()
        .retain(|(resource, _), fields| {
            !(resource == "authority"
                && fields["kind"] == serde_json::json!({"stringValue": "membership"})
                && fields["seq"] == serde_json::json!({"integerValue": "2"}))
        });
    assert!(matches!(
        rig.reader.authority(&page(), "membership"),
        Err(Fault::Stuck(_))
    ));
}
