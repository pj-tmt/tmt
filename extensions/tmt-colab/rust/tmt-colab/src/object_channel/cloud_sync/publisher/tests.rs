//! The publisher against an in-memory Firestore on the real bus: ordering, idempotent
//! preamble, uncertain effects, conflicts and refusals.
use super::*;
use crate::cloud_sync::fixture::{BINDING, Firestore, ScriptedRemote, document, peer};
use std::collections::VecDeque;

const PAGE: &str = "10000000-0000-4000-8000-000000000001";
fn page() -> PageRef {
    PageRef {
        space: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        page: PAGE.into(),
        epoch: 1,
    }
}
#[derive(Default)]
struct Fake {
    rows: Vec<Row>,
    queue: VecDeque<Data>,
    confirmed: Vec<String>,
    preambles: usize,
}
fn data(seq: u64) -> Data {
    Data {
        documents: vec![document(
            "log",
            &format!("log-{seq}"),
            &[("seq", &seq.to_string())],
        )],
        confirm: Confirm::Update {
            stream: "w".into(),
            seq,
            hash: [seq as u8; 32],
        },
    }
}
fn row(resource: &str, id: &str) -> Row {
    Row::Create(document(resource, id, &[("k", id)]))
}
impl Local for Fake {
    fn preamble(&mut self, _: &PageRef) -> Result<Vec<Row>, Fault> {
        self.preambles += 1;
        Ok(self
            .rows
            .iter()
            .map(|r| match r {
                Row::Create(d) => Row::Create(d.clone()),
                Row::Put(d) => Row::Put(d.clone()),
            })
            .collect())
    }
    fn next(&mut self, _: &PageRef) -> Result<Option<Data>, Fault> {
        Ok(self.queue.front().map(|d| Data {
            documents: d.documents.clone(),
            confirm: match &d.confirm {
                Confirm::Update { stream, seq, hash } => Confirm::Update {
                    stream: stream.clone(),
                    seq: *seq,
                    hash: *hash,
                },
                Confirm::Checkpoint { .. } => unreachable!(),
            },
        }))
    }
    fn confirm(&mut self, _: &PageRef, confirm: &Confirm) -> Result<(), Fault> {
        let Confirm::Update { seq, .. } = confirm else {
            unreachable!()
        };
        self.confirmed.push(format!("update-{seq}"));
        self.queue.pop_front();
        Ok(())
    }
}
struct Rig {
    publisher: Publisher,
    firestore: Firestore,
    _remote: ScriptedRemote,
    _peer: crate::cloud_sync::fixture::Peer,
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
        publisher: Publisher::new(client, BINDING.into()),
        firestore,
        _remote: remote,
        _peer: peer,
    }
}
fn kinds(rig: &Rig) -> Vec<String> {
    rig.firestore
        .log
        .lock()
        .unwrap()
        .iter()
        .map(|(kind, resource, id)| format!("{kind} {resource}/{id}"))
        .collect()
}

#[test]
fn rows_and_authority_come_first_then_each_original_in_order_and_is_confirmed() {
    let mut rig = rig();
    let mut local = Fake {
        rows: vec![row("spaces", "s"), row("authority", "a1")],
        queue: (1..=3).map(data).collect(),
        ..Fake::default()
    };
    assert_eq!(rig.publisher.pass(&mut local, &page()), Ok(false));
    assert_eq!(
        kinds(&rig),
        [
            "create spaces/s",
            "create authority/a1",
            "create log/log-1",
            "create log/log-2",
            "create log/log-3"
        ]
    );
    assert_eq!(local.confirmed, ["update-1", "update-2", "update-3"]);
}

#[test]
fn rows_already_present_are_not_read_or_written_again_in_this_process() {
    let mut rig = rig();
    let mut local = Fake {
        rows: vec![row("spaces", "s")],
        ..Fake::default()
    };
    rig.publisher.pass(&mut local, &page()).unwrap();
    let before = kinds(&rig).len();
    local.queue = (1..=1).map(data).collect();
    rig.publisher.pass(&mut local, &page()).unwrap();
    assert_eq!(
        kinds(&rig).len(),
        before + 1,
        "only the new original is sent"
    );
}

#[test]
fn a_lost_answer_for_a_stored_original_is_read_back_and_confirmed_without_a_second_create() {
    let mut rig = rig();
    rig.firestore.faults.lock().unwrap().unknown_after_store = true;
    let mut local = Fake {
        queue: (1..=1).map(data).collect(),
        ..Fake::default()
    };
    assert_eq!(rig.publisher.pass(&mut local, &page()), Ok(false));
    assert_eq!(kinds(&rig), ["create log/log-1", "read log/log-1"]);
    assert_eq!(local.confirmed, ["update-1"]);
}

#[test]
fn a_lost_answer_for_an_unstored_original_advances_nothing_and_retries_the_same_id() {
    let mut rig = rig();
    rig.firestore.faults.lock().unwrap().unknown_without_store = true;
    let mut local = Fake {
        queue: (1..=1).map(data).collect(),
        ..Fake::default()
    };
    assert_eq!(rig.publisher.pass(&mut local, &page()), Err(Fault::Retry));
    assert!(
        local.confirmed.is_empty(),
        "no progress without a confirmed original"
    );
    assert_eq!(rig.publisher.pass(&mut local, &page()), Ok(false));
    assert_eq!(local.confirmed, ["update-1"]);
    let creates = kinds(&rig)
        .iter()
        .filter(|k| k.starts_with("create log/log-1"))
        .count();
    assert_eq!(creates, 2);
}

#[test]
fn an_expiry_only_difference_is_the_same_original_and_other_bytes_are_a_conflict() {
    let mut rig = rig();
    let mut local = Fake::default();
    rig.firestore.docs.lock().unwrap().insert(
        ("log".into(), "log-1".into()),
        document("log", "log-1", &[("seq", "1"), ("expiresAt", "old")]).fields,
    );
    let mut same = data(1);
    same.documents = vec![document(
        "log",
        "log-1",
        &[("seq", "1"), ("expiresAt", "new")],
    )];
    local.queue = VecDeque::from([same]);
    assert_eq!(rig.publisher.pass(&mut local, &page()), Ok(false));
    assert_eq!(local.confirmed, ["update-1"]);

    let mut local = Fake::default();
    rig.firestore.docs.lock().unwrap().insert(
        ("log".into(), "log-2".into()),
        document("log", "log-2", &[("seq", "OTHER")]).fields,
    );
    local.queue = VecDeque::from([data(2)]);
    assert!(matches!(
        rig.publisher.pass(&mut local, &page()),
        Err(Fault::Stuck(_))
    ));
    assert!(local.confirmed.is_empty());
}

#[test]
fn a_budget_refusal_is_reported_with_its_wait_and_advances_nothing() {
    let mut rig = rig();
    rig.firestore.faults.lock().unwrap().refuse_next = Some(sync::Error {
        code: sync::Code::Budget,
        state: None,
        reset_at_ms: Some(1),
        retry_after_ms: Some(5_000),
    });
    let mut local = Fake {
        queue: (1..=1).map(data).collect(),
        ..Fake::default()
    };
    assert_eq!(
        rig.publisher.pass(&mut local, &page()),
        Err(Fault::Budget(5_000))
    );
    assert!(local.confirmed.is_empty());
}

#[test]
fn an_admission_refusal_stops_the_page() {
    let mut rig = rig();
    rig.firestore.faults.lock().unwrap().refuse_next = Some(sync::Error {
        code: sync::Code::Denied,
        state: None,
        reset_at_ms: None,
        retry_after_ms: None,
    });
    let mut local = Fake {
        queue: (1..=1).map(data).collect(),
        ..Fake::default()
    };
    assert!(matches!(
        rig.publisher.pass(&mut local, &page()),
        Err(Fault::Stuck(_))
    ));
}

#[test]
fn a_pass_yields_after_a_bounded_number_of_documents_and_the_next_continues() {
    let mut rig = rig();
    let mut local = Fake {
        queue: (1..=70).map(data).collect(),
        ..Fake::default()
    };
    assert_eq!(rig.publisher.pass(&mut local, &page()), Ok(true));
    assert_eq!(local.confirmed.len(), PASS_DOCUMENTS);
    assert_eq!(rig.publisher.pass(&mut local, &page()), Ok(false));
    assert_eq!(local.confirmed.len(), 70);
}

#[test]
fn the_page_row_is_put_once_and_not_rewritten_until_it_is_old() {
    let mut rig = rig();
    let mut local = Fake {
        rows: vec![Row::Put(document("pages", "p", &[("epoch", "1")]))],
        ..Fake::default()
    };
    rig.publisher.pass(&mut local, &page()).unwrap();
    rig.publisher.pass(&mut local, &page()).unwrap();
    assert_eq!(kinds(&rig), ["put pages/p"]);
}
