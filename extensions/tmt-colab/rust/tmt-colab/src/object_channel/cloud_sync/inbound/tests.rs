//! An inbound pass between two owner homes that share one owner key, with an in-memory cloud
//! between them: real stores, real signed statements, chains and sealed updates, and the
//! serve's own append path doing the applying.
use super::*;
use crate::{
    object_channel::{
        cloud_sync::{
            fixture::{
                BINDING, Firestore, ScriptedRemote,
                home::{DeviceSpec, Home, PAGE},
                peer,
            },
            local::{Bound, StoreLocal},
            publisher::Local,
        },
        documents::{
            Field,
            originals::{self, Authority, Routing},
        },
    },
    store::{Store, StreamScope},
    sync::{Access, Admission, CatchupContext, Code, Server, SyncScope},
};
use std::sync::{Arc, Mutex};

const A: &str = "30000000-0000-4000-8000-00000000000a";
const B: &str = "30000000-0000-4000-8000-00000000000b";
const C: &str = "30000000-0000-4000-8000-00000000000c";

/// An admission that never admits: ingest brings its own admitted key.
struct Never;
impl Admission for Never {
    fn catchup_context(&self, _: &str, _: &SyncScope, _: &Store) -> Result<CatchupContext, Code> {
        Err(Code::Denied)
    }
    fn authorize(&self, _: &str, _: &SyncScope, _: Access<'_>) -> Result<[u8; 32], Code> {
        Err(Code::Denied)
    }
}
fn spec(id: &'static str, signing: u8) -> DeviceSpec {
    DeviceSpec {
        id,
        signing,
        registered: true,
    }
}
struct Rig {
    /// The writer's home, whose authority and updates are in the cloud.
    writer: Home,
    /// The reader's home, which shares the owner key and registered only its own device.
    reader: Home,
    cloud: Firestore,
    inbound: StoreInbound<Never>,
    puller: Puller,
    reports: Arc<Mutex<Vec<String>>>,
    _remote: ScriptedRemote,
    _peer: crate::object_channel::cloud_sync::fixture::Peer,
}
fn rig() -> Rig {
    let writer = Home::new(&[spec(A, 9), spec(C, 29)], None);
    let reader = Home::new(&[spec(B, 19)], Some(&writer));
    assert_eq!(writer.key.space_id, reader.key.space_id);
    let mut peer = peer();
    let cloud = Firestore::default();
    let answer = cloud.clone();
    let remote = ScriptedRemote::serve(peer.remote.take().unwrap(), move |call, id| {
        answer.answer(call, id)
    });
    let client = peer.channels.client.lock().unwrap().clone().unwrap();
    let server = Server::new(Store::open(&reader.layout).unwrap(), Never);
    let inbound = StoreInbound::new(&reader.layout, server, reader.key.space_id.clone()).unwrap();
    Rig {
        puller: Puller::new(Reader::new(client, BINDING.into())),
        writer,
        reader,
        cloud,
        inbound,
        reports: Arc::default(),
        _remote: remote,
        _peer: peer,
    }
}
impl Rig {
    /// Put the writer home's signed authority in the cloud, as its publisher does.
    fn publish_authority(&self, devices: &[&str]) {
        let space = self.writer.key.space_id.clone();
        let routing = Routing {
            space: &space,
            page: PAGE,
            epoch: 1,
            uid: "owner",
            expires_at: Field::timestamp(1_790_000_000_000).unwrap(),
        };
        for statement in self.writer.statements() {
            for document in
                originals::authority_documents(&routing, Authority::Membership, "", &statement)
                    .unwrap()
            {
                self.cloud.put(&document);
            }
        }
        for device in devices {
            let chain = self.writer.chain(device);
            for document in
                originals::authority_documents(&routing, Authority::Chain, "", &chain).unwrap()
            {
                self.cloud.put(&document);
            }
        }
    }
    fn publish(&self, original: &[u8]) {
        let space = self.writer.key.space_id.clone();
        let routing = Routing {
            space: &space,
            page: PAGE,
            epoch: 1,
            uid: "owner",
            expires_at: Field::timestamp(1_790_000_000_000).unwrap(),
        };
        self.cloud
            .put(&originals::log_document(&routing, original).unwrap());
    }
    fn stored(&self, home: &Home, stream: &str, seq: u64) -> Vec<u8> {
        home.store
            .payload(
                StreamScope {
                    page: PAGE,
                    epoch: 1,
                    stream,
                },
                seq,
            )
            .unwrap()
            .unwrap()
    }
    fn pass(&mut self) -> Result<bool, Fault> {
        let sink = Arc::clone(&self.reports);
        let page = self.reader.page(1);
        self.puller.pass(&mut self.inbound, &page, &move |line| {
            sink.lock().unwrap().push(line.to_owned())
        })
    }
    fn head(&self, stream: &str) -> Option<(u64, [u8; 32])> {
        self.reader
            .store
            .stream_head(StreamScope {
                page: PAGE,
                epoch: 1,
                stream,
            })
            .unwrap()
    }
}

#[test]
fn a_writers_stream_is_pulled_admitted_and_applied_in_order_then_nothing_more() {
    let mut rig = rig();
    let sent: Vec<_> = (0..3).map(|_| rig.writer.append(A)).collect();
    rig.publish_authority(&[A]);
    sent.iter().for_each(|bytes| rig.publish(bytes));
    assert_eq!(rig.pass(), Ok(true));
    assert_eq!(rig.head(A).unwrap().0, 3);
    for (index, bytes) in sent.iter().enumerate() {
        assert_eq!(&rig.stored(&rig.reader, A, index as u64 + 1), bytes);
    }
    // Nothing is left, and a repeat changes nothing.
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.head(A).unwrap().0, 3);
    assert!(rig.reports.lock().unwrap().is_empty());
}

#[test]
fn what_arrives_from_the_cloud_is_never_queued_for_publication() {
    let mut rig = rig();
    let sent: Vec<_> = (0..2).map(|_| rig.writer.append(A)).collect();
    rig.publish_authority(&[A]);
    sent.iter().for_each(|bytes| rig.publish(bytes));
    assert_eq!(rig.pass(), Ok(true));
    assert_eq!(rig.head(A).unwrap().0, 2);
    let mut local = StoreLocal::new(
        Store::open(&rig.reader.layout).unwrap(),
        rig.reader.key.owner_public(),
        rig.reader.key.space_id.clone(),
        Bound {
            project: "fixture".into(),
            declaration_digest: "b".repeat(64),
            uid: "owner-uid".into(),
        },
        Arc::new(|| 1_790_000_000_000),
    );
    let page = rig.reader.page(1);
    assert!(local.next(&page).ok().unwrap().is_none());
}

#[test]
fn a_cloud_log_ahead_of_this_machine_or_forked_from_it_stops_the_page_with_its_own_line() {
    let mut rig = rig();
    rig.writer.append(A);
    rig.writer
        .retention(30, "40000000-0000-4000-8000-000000000002");
    rig.publish_authority(&[A]);
    let Err(Fault::Stuck(ahead)) = rig.pass() else {
        panic!("ahead is a refusal")
    };
    assert!(ahead.contains("newer sharing changes"));
    assert_eq!(rig.head(A), None);
    // This machine has its own second revision: the logs differ at the same revision.
    rig.reader
        .retention(31, "40000000-0000-4000-8000-000000000003");
    rig.puller.refresh();
    let Err(Fault::Stuck(fork)) = rig.pass() else {
        panic!("a fork is a refusal")
    };
    assert!(fork.contains("disagree"));
    assert_ne!(ahead, fork);
    assert_eq!(rig.head(A), None);
}

#[test]
fn a_forged_original_stops_only_its_own_stream_and_is_reported_once() {
    let mut rig = rig();
    let good = rig.writer.append(C);
    rig.publish_authority(&[A, C]);
    rig.publish(&good);
    // A's first original is signed by a key that is not the one A's certificate names.
    let forged = rig
        .writer
        .seal_as(A, "update", "content", 1, [0; 32], 99, "1");
    rig.publish(&forged.to_json().unwrap());
    assert!(rig.pass().is_ok());
    assert_eq!(rig.head(A), None);
    assert_eq!(rig.head(C).unwrap().0, 1);
    assert_eq!(rig.reports.lock().unwrap().len(), 1);
    assert_eq!(
        rig.reports.lock().unwrap()[0],
        format!(
            "Cloud sync stopped following one writer of page {PAGE}: its page original is not valid."
        )
    );
    // The refused stream is not read again.
    assert!(rig.pass().is_ok());
    assert_eq!(rig.reports.lock().unwrap().len(), 1);
}

#[test]
fn a_malformed_record_of_one_writer_stops_that_writer_and_no_other() {
    let mut rig = rig();
    let good = rig.writer.append(C);
    let bad = rig.writer.append(A);
    rig.publish_authority(&[A, C]);
    rig.publish(&good);
    rig.publish(&bad);
    // The routing of A's record names another hash than its signed bytes carry.
    let id = originals::log_document(
        &Routing {
            space: &rig.writer.key.space_id,
            page: PAGE,
            epoch: 1,
            uid: "owner",
            expires_at: Field::timestamp(1_790_000_000_000).unwrap(),
        },
        &bad,
    )
    .unwrap()
    .target
    .id;
    rig.cloud
        .docs
        .lock()
        .unwrap()
        .get_mut(&("log".to_owned(), id))
        .unwrap()
        .insert(
            "hash".into(),
            serde_json::json!({"stringValue": tmt_colab_model::values::encode_binary(&[9; 32])}),
        );
    assert!(rig.pass().is_ok());
    assert_eq!(rig.head(A), None);
    assert_eq!(rig.head(C).unwrap().0, 1);
    assert_eq!(rig.reports.lock().unwrap().len(), 1);
    assert_eq!(
        rig.reports.lock().unwrap()[0],
        format!(
            "Cloud sync stopped following one writer of page {PAGE}: one of its cloud records is malformed."
        )
    );
    // It is not read again.
    assert!(rig.pass().is_ok());
    assert_eq!(rig.reports.lock().unwrap().len(), 1);
}

#[test]
fn admission_refuses_another_epoch_or_key_and_a_good_original_applies_once() {
    let mut rig = rig();
    let page = rig.reader.page(1);
    let chains = vec![rig.writer.chain(A)];
    assert_eq!(rig.inbound.begin(&page, &chains), Ok(vec![A.to_owned()]));
    let seal = |signing, epoch| {
        rig.writer
            .seal_as(A, "update", "content", 1, [0; 32], signing, epoch)
            .to_json()
            .unwrap()
    };
    assert_eq!(
        rig.inbound.admit(&page, &seal(9, "2")),
        Err(Reject::Refused)
    );
    assert_eq!(
        rig.inbound.admit(&page, &seal(99, "1")),
        Err(Reject::Refused)
    );
    assert_eq!(rig.head(A), None);
    let good = seal(9, "1");
    assert_eq!(rig.inbound.admit(&page, &good), Ok(Applied::New));
    assert_eq!(rig.inbound.admit(&page, &good), Ok(Applied::Replay));
    assert_eq!(rig.head(A).unwrap().0, 1);
    // A device the cloud sent no chain for cannot be admitted.
    let unknown = rig
        .writer
        .seal(C, "update", "content", 1, [0; 32])
        .to_json()
        .unwrap();
    assert_eq!(rig.inbound.admit(&page, &unknown), Err(Reject::Refused));
}

#[test]
fn a_sequence_the_cloud_no_longer_holds_stops_that_writer_with_a_line() {
    let mut rig = rig();
    for _ in 0..3 {
        rig.writer.append(A);
    }
    rig.publish_authority(&[A]);
    // Only the third update is left in the cloud.
    let third = rig.stored(&rig.writer, A, 3);
    rig.publish(&third);
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.head(A), None);
    assert_eq!(rig.reports.lock().unwrap().len(), 1);
    assert_eq!(
        rig.reports.lock().unwrap()[0],
        format!(
            "Cloud sync stopped following one writer of page {PAGE}: it is too far behind; open a fresh copy of the page to catch up."
        )
    );
}
