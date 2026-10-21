//! An inbound pass between two owner homes that share one owner key, with an in-memory cloud
//! between them: real stores, real signed statements, chains and sealed updates, and the
//! serve's own append path doing the applying.
use super::*;
use crate::{
    object_channel::{
        cloud_sync::{
            fixture::{
                BINDING, Firestore, SPACE, ScriptedRemote,
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
    store::{Namespace, NamespaceCursor, Store, StreamScope},
    sync::{Access, Admission, CatchupContext, Code, Server, SyncScope},
};
use ed25519_dalek::SigningKey;
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};
use tmt_colab_model::object;
use tmt_extension_objects::sync;

const A: &str = "30000000-0000-4000-8000-00000000000a";
const B: &str = "30000000-0000-4000-8000-00000000000b";
const C: &str = "30000000-0000-4000-8000-00000000000c";

/// An admission that never admits: ingest brings its own admitted key. It has no native writer, and
/// counts the times the serve looks for one, which is how a pass shows it went to settle the
/// page's edit requests.
struct Never(Arc<std::sync::atomic::AtomicUsize>);
impl Admission for Never {
    fn save_source(&self) -> Option<crate::page::save::SourceOpener> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        None
    }
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
    /// How many times a pass went to settle edit requests.
    settles: Arc<std::sync::atomic::AtomicUsize>,
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
    let settles = Arc::<std::sync::atomic::AtomicUsize>::default();
    let server = Server::new(
        Store::open(&reader.layout).unwrap(),
        Never(Arc::clone(&settles)),
    );
    let inbound = StoreInbound::new(&reader.layout, server, reader.key.space_id.clone()).unwrap();
    Rig {
        puller: Puller::new(Reader::new(client, BINDING.into())),
        writer,
        reader,
        cloud,
        inbound,
        reports: Arc::default(),
        settles,
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
    /// Put checkpoint (or fold) originals in the cloud whole, as the publisher's chunks.
    fn publish_checkpoints(&self, objects: &[&[u8]]) {
        let space = self.writer.key.space_id.clone();
        let routing = Routing {
            space: &space,
            page: PAGE,
            epoch: 1,
            uid: "owner",
            expires_at: Field::timestamp(1_790_000_000_000).unwrap(),
        };
        for bytes in objects {
            let fields = originals::checkpoint_base(&routing, bytes).unwrap();
            for document in originals::checkpoint_documents(fields, bytes).unwrap() {
                self.cloud.put(&document);
            }
        }
    }
    /// The latest pair of a stream as its home holds it, content then own.
    fn pair_of(&self, home: &Home, stream: &str) -> Vec<Vec<u8>> {
        [Namespace::Content, Namespace::Own]
            .into_iter()
            .map(|namespace| {
                let object = home
                    .store
                    .namespace_next(
                        StreamScope {
                            page: PAGE,
                            epoch: 1,
                            stream,
                        },
                        namespace,
                        NamespaceCursor::default(),
                    )
                    .unwrap()
                    .unwrap();
                assert!(object.checkpoint);
                object.bytes
            })
            .collect()
    }
    /// How many times the cloud was asked to list the page's checkpoints.
    fn checkpoint_discoveries(&self) -> usize {
        self._remote
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| {
                matches!(
                    call,
                    sync::Call::Query {
                        query: sync::Query::Discovery { resource, .. },
                        ..
                    } if resource == "checkpoints"
                )
            })
            .count()
    }
    /// Whether the reader holds any checkpoint of `stream`.
    fn holds_checkpoints(&self, stream: &str) -> bool {
        [Namespace::Content, Namespace::Own]
            .into_iter()
            .any(|namespace| {
                self.reader
                    .store
                    .namespace_next(
                        StreamScope {
                            page: PAGE,
                            epoch: 1,
                            stream,
                        },
                        namespace,
                        NamespaceCursor::default(),
                    )
                    .unwrap()
                    .is_some_and(|object| object.checkpoint)
            })
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
fn a_page_new_to_the_process_is_settled_once_and_a_members_originals_do_not_ask_again() {
    use std::sync::atomic::Ordering;
    let mut rig = rig();
    let first: Vec<_> = (0..2).map(|_| rig.writer.append(A)).collect();
    rig.publish_authority(&[A]);
    first.iter().for_each(|bytes| rig.publish(bytes));
    // Requests left while this process was not running are looked for when the page is first
    // read here, once...
    assert_eq!(rig.pass(), Ok(true));
    assert_eq!(rig.settles.load(Ordering::SeqCst), 1);
    // ...and a member's originals, which carry no link's request, do not ask for it again.
    let more = rig.writer.append(A);
    rig.publish(&more);
    assert_eq!(rig.pass(), Ok(true));
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.settles.load(Ordering::SeqCst), 1);
}

#[test]
fn only_a_chain_a_link_issued_is_a_link_device() {
    let rig = rig();
    let link = tmt_colab_model::link::device_chain(
        &[3; 32],
        &rig.writer.key.space_id,
        "20000000-0000-4000-8000-000000000001",
        "2",
        &[9; 32],
        tmt_colab_model::link::Transport::Door,
    )
    .unwrap();
    assert!(super::store::link_issued(&link));
    assert!(!super::store::link_issued(&rig.writer.chain(A)));
    assert!(!super::store::link_issued(b"not a chain"));
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
        rig.reader.key.local_writer().unwrap().0,
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
    let known = |rig: &Rig| {
        let writer = rig.reader.key.local_writer().unwrap().0;
        rig.reader
            .store
            .owner_read(
                &rig.reader.key.space_id,
                &rig.reader.key.owner_public(),
                |tx| Ok((tx.device(A)?.is_some(), tx.is_local_device(&writer, A)?)),
            )
            .unwrap()
    };
    assert_eq!(known(&rig), (false, false));
    assert_eq!(rig.inbound.admit(&page, &good), Ok(Applied::New));
    // The writer's chain is kept so a browser opening the page can verify its updates, and
    // the writer is still not one of this machine's devices.
    assert_eq!(known(&rig), (true, false));
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
            "Cloud sync stopped following one writer of page {PAGE}: this machine cannot open this page's cloud copy yet. Keep editing it on the machine that created it."
        )
    );
}

/// A Store side that follows one writer and refuses its originals on one page only.
struct RefusesOnePage {
    refuse: &'static str,
    admitted: Vec<String>,
}
impl Inbound for RefusesOnePage {
    fn membership(&mut self, _: &PageRef, _: &[Vec<u8>]) -> Result<(), Fault> {
        Ok(())
    }
    fn begin(&mut self, _: &PageRef, _: &[Vec<u8>]) -> Result<Vec<String>, Fault> {
        Ok(vec![A.to_owned()])
    }
    fn head(&mut self, _: &PageRef, _: &str) -> Result<u64, Fault> {
        Ok(0)
    }
    fn admit(&mut self, page: &PageRef, _: &[u8]) -> Result<Applied, Reject> {
        if page.page == self.refuse {
            return Err(Reject::Refused);
        }
        self.admitted.push(page.page.clone());
        Ok(Applied::New)
    }
}

#[test]
fn a_writer_stopped_on_one_page_is_still_followed_on_another_and_stays_stopped_on_the_first() {
    const OTHER: &str = "10000000-0000-4000-8000-000000000002";
    let mut peer = peer();
    let cloud = Firestore::default();
    let answer = cloud.clone();
    let _remote = ScriptedRemote::serve(peer.remote.take().unwrap(), move |call, id| {
        answer.answer(call, id)
    });
    let client = peer.channels.client.lock().unwrap().clone().unwrap();
    let mut puller = Puller::new(Reader::new(client, BINDING.into()));
    // The same writer has a first original on each of two pages.
    let signer = SigningKey::from_bytes(&[5; 32]);
    for page in [PAGE, OTHER] {
        let context = object::Context {
            space: SPACE.into(),
            page: page.into(),
            epoch: "1".into(),
            kind: "update".into(),
            namespace: "content".into(),
            author_device: A.into(),
            membership_revision: "1".into(),
            stream_seq: "1".into(),
            prev_hash: [0; 32],
        };
        let original = object::seal(&context, &[9; 32], &signer, b"update")
            .unwrap()
            .to_json()
            .unwrap();
        let routing = Routing {
            space: SPACE,
            page,
            epoch: 1,
            uid: "owner",
            expires_at: Field::timestamp(1_790_000_000_000).unwrap(),
        };
        cloud.put(&originals::log_document(&routing, &original).unwrap());
    }
    let on = |page: &str| PageRef {
        space: SPACE.into(),
        page: page.into(),
        epoch: 1,
    };
    let reports = Mutex::new(Vec::new());
    let report = |line: &str| reports.lock().unwrap().push(line.to_owned());
    let mut inbound = RefusesOnePage {
        refuse: PAGE,
        admitted: Vec::new(),
    };
    // Refused on the first page: one line, and the writer is not tried there again.
    puller.pass(&mut inbound, &on(PAGE), &report).unwrap();
    puller.pass(&mut inbound, &on(PAGE), &report).unwrap();
    assert_eq!(reports.lock().unwrap().len(), 1);
    assert!(inbound.admitted.is_empty());
    // The other page is another scope: the same writer is applied there, with no line.
    puller.refresh();
    puller.pass(&mut inbound, &on(OTHER), &report).unwrap();
    assert_eq!(inbound.admitted, [OTHER]);
    assert_eq!(reports.lock().unwrap().len(), 1);
    // Back on the first page it is still stopped, and says nothing more.
    puller.refresh();
    puller.pass(&mut inbound, &on(PAGE), &report).unwrap();
    assert_eq!(inbound.admitted, [OTHER]);
    assert_eq!(reports.lock().unwrap().len(), 1);
}

/// How many times the cloud was asked for the membership records and for the device chains.
fn authority_reads(rig: &Rig) -> (usize, usize) {
    let (mut membership, mut chains) = (0, 0);
    for call in rig._remote.calls.lock().unwrap().iter() {
        if let sync::Call::Query {
            query: sync::Query::Discovery { scope, .. },
            ..
        } = call
        {
            match scope
                .get("kind")
                .and_then(|kind| kind["stringValue"].as_str())
            {
                Some("membership") => membership += 1,
                Some("chain") => chains += 1,
                _ => {}
            }
        }
    }
    (membership, chains)
}

#[test]
fn a_records_change_reads_the_chains_again_only_once_their_spacing_has_passed() {
    let mut rig = rig();
    rig.publish_authority(&[A]);
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(authority_reads(&rig), (1, 1));
    assert_eq!(rig.puller.recheck_at(), None);
    // The log or the checkpoints changed: the chains wait for their spacing, the membership
    // records are not read at all.
    rig.puller.records_changed();
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(authority_reads(&rig), (1, 1));
    assert!(rig.puller.recheck_at().is_some());
    // Once the spacing has passed they are read, and the wait is over.
    rig.puller.chains.as_mut().unwrap().1 = Instant::now() - CHAIN_RECHECK;
    assert_eq!(
        rig.puller.recheck_at().map(|at| at <= Instant::now()),
        Some(true)
    );
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(authority_reads(&rig), (1, 2));
    assert_eq!(rig.puller.recheck_at(), None);
    // A change of the authority reads both at once.
    rig.puller.refresh();
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(authority_reads(&rig), (2, 3));
}

/// What the readers of a page (`page read`, export, the shell decoder, a fold of other devices)
/// are handed for `stream`: its cuts and, for each, the stored objects behind it.
type Cuts = Vec<(
    crate::store::owner::epoch::Cut,
    Vec<(u64, [u8; 32], [u8; 32], bool, Vec<u8>)>,
)>;
fn reads(home: &Home, stream: &str) -> Cuts {
    home.store
        .owner_read(&home.key.space_id, &home.key.owner_public(), |tx| {
            let mut out = Vec::new();
            for cut in tx
                .read_cuts(PAGE, 1)?
                .into_iter()
                .filter(|cut| cut.stream == stream)
            {
                let objects = tx
                    .cut_objects(&cut)?
                    .into_iter()
                    .map(|o| (o.seq, o.hash, o.previous, o.checkpoint, o.bytes))
                    .collect();
                out.push((cut, objects));
            }
            Ok(out)
        })
        .unwrap()
}

#[test]
fn the_readers_of_a_page_see_a_bootstrapped_stream_as_its_writer_does() {
    let mut rig = rig();
    for _ in 0..4 {
        rig.writer.append(A);
    }
    let pair = rig.writer.pair(A, 4);
    rig.publish_authority(&[A]);
    rig.publish_checkpoints(&[&pair[0], &pair[1]]);
    // The pair, and then a tail that names the head the pair bound.
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(reads(&rig.reader, A), reads(&rig.writer, A));
    let bound = rig.writer.head(A).1;
    rig.writer.append(A);
    let fifth = rig.stored(&rig.writer, A, 5);
    rig.publish(&fifth);
    rig.puller.records_changed();
    assert_eq!(rig.pass(), Ok(true));
    let read = reads(&rig.reader, A);
    assert_eq!(read, reads(&rig.writer, A));
    let tail: Vec<_> = read
        .iter()
        .flat_map(|(_, objects)| objects.iter().filter(|object| !object.3))
        .collect();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].2, bound);
}

#[test]
fn a_pair_with_no_log_after_it_opens_on_a_second_machine_without_waiting_for_the_next_edit() {
    let mut rig = rig();
    for _ in 0..4 {
        rig.writer.append(A);
    }
    let pair = rig.writer.pair(A, 4);
    rig.publish_authority(&[A]);
    rig.publish_checkpoints(&[&pair[0], &pair[1]]);
    // Nothing of the log is in the cloud at all: the tail reads as empty, not as a missing prefix.
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.head(A), Some(rig.writer.head(A)));
    assert_eq!(rig.pair_of(&rig.reader, A), pair);
    assert!(rig.reports.lock().unwrap().is_empty());
    // Then the writer's next update follows.
    rig.writer.append(A);
    let fifth = rig.stored(&rig.writer, A, 5);
    rig.publish(&fifth);
    assert_eq!(rig.pass(), Ok(true));
    assert_eq!(rig.head(A), Some(rig.writer.head(A)));
}

#[test]
fn a_writer_with_nothing_in_the_cloud_is_looked_for_once_until_its_records_change() {
    let mut rig = rig();
    rig.writer.append(A);
    rig.publish_authority(&[A]);
    assert_eq!(rig.pass(), Ok(false));
    assert!(rig.reports.lock().unwrap().is_empty());
    assert_eq!(rig.checkpoint_discoveries(), 1);
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.checkpoint_discoveries(), 1);
    rig.puller.records_changed();
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.checkpoint_discoveries(), 2);
}

const CANNOT_OPEN: &str = "this machine cannot open this page's cloud copy yet. Keep editing it on the machine that created it.";
fn line(text: &str) -> String {
    format!("Cloud sync stopped following one writer of page {PAGE}: {text}")
}

#[test]
fn a_checkpoint_pair_published_before_any_log_opens_on_a_second_machine_and_follows_its_tail() {
    let mut rig = rig();
    for _ in 0..4 {
        rig.writer.append(A);
    }
    let pair = rig.writer.pair(A, 4);
    rig.writer.append(A);
    rig.writer.append(A);
    rig.publish_authority(&[A]);
    rig.publish_checkpoints(&[&pair[0], &pair[1]]);
    let (fifth, sixth) = (rig.stored(&rig.writer, A, 5), rig.stored(&rig.writer, A, 6));
    rig.publish(&fifth);
    rig.publish(&sixth);
    assert_eq!(rig.pass(), Ok(true));
    assert_eq!(rig.head(A), Some(rig.writer.head(A)));
    assert!(rig.reports.lock().unwrap().is_empty());
    // The reader holds the pair and the tail, never the pruned prefix.
    assert_eq!(rig.pair_of(&rig.reader, A), pair);
    assert_eq!(rig.stored(&rig.reader, A, 5), fifth);
    assert_eq!(rig.stored(&rig.reader, A, 6), sixth);
    assert_eq!(
        rig.reader
            .store
            .payload(
                StreamScope {
                    page: PAGE,
                    epoch: 1,
                    stream: A
                },
                1
            )
            .unwrap(),
        None
    );
    // Caught up: nothing more is read, and the checkpoints were listed once.
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.checkpoint_discoveries(), 1);
}

#[test]
fn an_owner_fold_published_with_its_anchor_opens_on_a_second_machine_and_follows_its_tail() {
    let mut rig = rig();
    rig.writer.append(A);
    rig.writer.append(A);
    rig.writer.fold_pair(A, 2);
    rig.writer.append(A);
    rig.publish_authority(&[A, C]);
    let scope = StreamScope {
        page: PAGE,
        epoch: 1,
        stream: A,
    };
    let anchor = rig.writer.store.anchor(scope, 2).unwrap().unwrap();
    rig.publish(&anchor.bytes);
    let folds = rig.pair_of(&rig.writer, A);
    rig.publish_checkpoints(&[&folds[0], &folds[1]]);
    let third = rig.stored(&rig.writer, A, 3);
    rig.publish(&third);
    assert_eq!(rig.pass(), Ok(true));
    assert!(rig.reports.lock().unwrap().is_empty());
    assert_eq!(rig.head(A), Some(rig.writer.head(A)));
    assert_eq!(rig.pair_of(&rig.reader, A), folds);
    // The anchor is kept as the receipt, so the fold stays readable and re-foldable here.
    let kept = rig.reader.store.anchor(scope, 2).unwrap().unwrap();
    assert_eq!(kept.bytes, anchor.bytes);
    assert_eq!(rig.stored(&rig.reader, A, 3), third);
}

#[test]
fn a_fold_without_its_anchor_or_with_the_wrong_one_opens_nothing() {
    let mut rig = rig();
    rig.writer.append(A);
    rig.writer.append(A);
    rig.writer.fold_pair(A, 2);
    rig.writer.append(A);
    rig.publish_authority(&[A, C]);
    let folds = rig.pair_of(&rig.writer, A);
    rig.publish_checkpoints(&[&folds[0], &folds[1]]);
    let third = rig.stored(&rig.writer, A, 3);
    rig.publish(&third);
    // The folds are in the cloud and the update they bind is not.
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.head(A), None);
    assert!(!rig.holds_checkpoints(A));
    assert_eq!(rig.reports.lock().unwrap().clone(), [line(CANNOT_OPEN)]);
}

#[test]
fn half_a_pair_waits_without_stopping_or_looping_and_opens_when_the_rest_arrives() {
    let mut rig = rig();
    for stream in [A, C] {
        for _ in 0..4 {
            rig.writer.append(stream);
        }
    }
    let (pair_a, pair_c) = (rig.writer.pair(A, 4), rig.writer.pair(C, 4));
    rig.writer.append(A);
    rig.writer.append(C);
    rig.publish_authority(&[A, C]);
    for stream in [A, C] {
        let fifth = rig.stored(&rig.writer, stream, 5);
        rig.publish(&fifth);
    }
    // Only the content half of each pair has been written so far.
    rig.publish_checkpoints(&[&pair_a[0], &pair_c[0]]);
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!((rig.head(A), rig.head(C)), (None, None));
    assert!(!rig.holds_checkpoints(A) && !rig.holds_checkpoints(C));
    assert!(rig.reports.lock().unwrap().is_empty());
    // Two waiting streams share one listing, and they are not listed again until the records change.
    assert_eq!(rig.checkpoint_discoveries(), 1);
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.checkpoint_discoveries(), 1);
    rig.publish_checkpoints(&[&pair_a[1], &pair_c[1]]);
    rig.puller.records_changed();
    assert_eq!(rig.pass(), Ok(true));
    assert_eq!(rig.head(A), Some(rig.writer.head(A)));
    assert_eq!(rig.head(C), Some(rig.writer.head(C)));
    assert_eq!(rig.checkpoint_discoveries(), 2);
    assert!(rig.reports.lock().unwrap().is_empty());
}

#[test]
fn a_forged_pair_changes_nothing_and_is_said_once() {
    let mut rig = rig();
    for _ in 0..4 {
        rig.writer.append(A);
    }
    rig.writer.pair(A, 4);
    rig.writer.append(A);
    rig.publish_authority(&[A]);
    let fifth = rig.stored(&rig.writer, A, 5);
    rig.publish(&fifth);
    // Signed by a key that is not the writer's.
    let forged: Vec<Vec<u8>> = ["content", "own"]
        .into_iter()
        .map(|namespace| {
            rig.writer
                .seal_as(A, "checkpoint", namespace, 4, [4; 32], 77, "1")
                .to_json()
                .unwrap()
        })
        .collect();
    rig.publish_checkpoints(&[&forged[0], &forged[1]]);
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.head(A), None);
    assert!(!rig.holds_checkpoints(A));
    assert_eq!(
        rig.reports.lock().unwrap().clone(),
        [line("its page original is not valid.")]
    );
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.reports.lock().unwrap().len(), 1);
}

#[test]
fn two_halves_that_bind_different_heads_are_malformed_and_change_nothing() {
    let mut rig = rig();
    for _ in 0..4 {
        rig.writer.append(A);
    }
    rig.writer.pair(A, 4);
    rig.writer.append(A);
    rig.publish_authority(&[A]);
    let fifth = rig.stored(&rig.writer, A, 5);
    rig.publish(&fifth);
    let halves: Vec<Vec<u8>> = [("content", [4u8; 32]), ("own", [5u8; 32])]
        .into_iter()
        .map(|(namespace, previous)| {
            rig.writer
                .seal(A, "checkpoint", namespace, 4, previous)
                .to_json()
                .unwrap()
        })
        .collect();
    rig.publish_checkpoints(&[&halves[0], &halves[1]]);
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.head(A), None);
    assert!(!rig.holds_checkpoints(A));
    assert_eq!(
        rig.reports.lock().unwrap().clone(),
        [line("one of its cloud records is malformed.")]
    );
}

#[test]
fn a_writer_this_machine_already_holds_a_prefix_of_is_never_joined_to_a_pruned_one() {
    let mut rig = rig();
    for _ in 0..4 {
        rig.writer.append(A);
    }
    rig.publish_authority(&[A]);
    for seq in 1..=2 {
        let original = rig.stored(&rig.writer, A, seq);
        rig.publish(&original);
    }
    assert_eq!(rig.pass(), Ok(true));
    assert_eq!(rig.head(A).map(|head| head.0), Some(2));
    // The writer compacts at 4 and the cloud keeps only the pair and the update after it.
    let pair = rig.writer.pair(A, 4);
    rig.writer.append(A);
    rig.publish_checkpoints(&[&pair[0], &pair[1]]);
    let fifth = rig.stored(&rig.writer, A, 5);
    rig.cloud.docs.lock().unwrap().retain(|(resource, id), _| {
        !(resource == "log" && (id.ends_with("_1") || id.ends_with("_2")))
    });
    rig.publish(&fifth);
    rig.puller.refresh();
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.head(A).map(|head| head.0), Some(2));
    assert!(!rig.holds_checkpoints(A));
    assert_eq!(rig.reports.lock().unwrap().clone(), [line(CANNOT_OPEN)]);
    assert_eq!(rig.checkpoint_discoveries(), 0);
}

#[test]
fn a_bootstrapped_stream_survives_a_restart_and_takes_the_same_pair_only_as_a_replay() {
    let mut rig = rig();
    for _ in 0..4 {
        rig.writer.append(A);
    }
    let pair = rig.writer.pair(A, 4);
    rig.writer.append(A);
    rig.publish_authority(&[A]);
    rig.publish_checkpoints(&[&pair[0], &pair[1]]);
    let fifth = rig.stored(&rig.writer, A, 5);
    rig.publish(&fifth);
    assert_eq!(rig.pass(), Ok(true));
    let head = rig.head(A);
    assert_eq!(head, Some(rig.writer.head(A)));
    // A new process over the same files finds the position where it was.
    let server = Server::new(
        Store::open(&rig.reader.layout).unwrap(),
        Never(Arc::default()),
    );
    rig.inbound =
        StoreInbound::new(&rig.reader.layout, server, rig.reader.key.space_id.clone()).unwrap();
    rig.puller.refresh();
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.head(A), head);
    // The same pair again changes nothing; a later one is not merged in.
    let page = rig.reader.page(1);
    assert_eq!(
        rig.inbound.bootstrap(&page, A, None, &pair),
        Ok(Applied::Replay)
    );
    rig.writer.append(A);
    let later = rig.writer.pair(A, 6);
    assert_eq!(
        rig.inbound.bootstrap(&page, A, None, &later),
        Err(Reject::Refused)
    );
    assert_eq!(rig.head(A), head);
    assert_eq!(rig.pair_of(&rig.reader, A), pair);
}

#[test]
fn a_page_with_no_checkpoint_in_the_cloud_keeps_its_line_and_reads_nothing_twice() {
    let mut rig = rig();
    for _ in 0..3 {
        rig.writer.append(A);
    }
    rig.publish_authority(&[A]);
    let third = rig.stored(&rig.writer, A, 3);
    rig.publish(&third);
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.reports.lock().unwrap().clone(), [line(CANNOT_OPEN)]);
    assert_eq!(rig.checkpoint_discoveries(), 1);
    assert_eq!(rig.pass(), Ok(false));
    assert_eq!(rig.checkpoint_discoveries(), 1);
}
