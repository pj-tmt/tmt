//! The publisher against an in-memory Firestore on the real bus: ordering, idempotent
//! preamble, uncertain effects, conflicts and refusals.
use super::*;
use crate::cloud_sync::fixture::{BINDING, Firestore, ScriptedRemote, document, peer};
use crate::object_channel::documents;
use std::{collections::VecDeque, time::Duration};

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
    /// A stream that appears after this many reads: its chain row and its first original.
    arrival: Option<(usize, Row, Data)>,
    reads: usize,
    /// Links that ended and whose cloud rows are not yet removed.
    ended: Vec<Retirement>,
    retired: Vec<String>,
    /// The expiries the publisher reported as confirmed page rows, in order.
    page_rows: Vec<u64>,
}
impl Fake {
    /// Every read of local state may be the moment a concurrent append lands.
    fn read(&mut self) {
        self.reads += 1;
        if self
            .arrival
            .as_ref()
            .is_some_and(|(at, ..)| *at == self.reads)
        {
            let (_, chain, first) = self.arrival.take().unwrap();
            self.rows.push(chain);
            self.queue.push_back(first);
        }
    }
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
        let rows = self
            .rows
            .iter()
            .map(|r| match r {
                Row::Create(d) => Row::Create(d.clone()),
                Row::Put {
                    document,
                    days,
                    refresh,
                    expires_ms,
                } => Row::Put {
                    document: document.clone(),
                    days: *days,
                    refresh: *refresh,
                    expires_ms: *expires_ms,
                },
            })
            .collect();
        self.read();
        Ok(rows)
    }
    fn next(&mut self, _: &PageRef) -> Result<Option<Data>, Fault> {
        let next = self.queue.front().map(|d| Data {
            documents: d.documents.clone(),
            confirm: match &d.confirm {
                Confirm::Update { stream, seq, hash } => Confirm::Update {
                    stream: stream.clone(),
                    seq: *seq,
                    hash: *hash,
                },
                Confirm::Checkpoint { .. } => unreachable!(),
            },
        });
        self.read();
        Ok(next)
    }
    fn confirm(&mut self, _: &PageRef, confirm: &Confirm) -> Result<(), Fault> {
        let Confirm::Update { seq, .. } = confirm else {
            unreachable!()
        };
        self.confirmed.push(format!("update-{seq}"));
        self.queue.pop_front();
        Ok(())
    }
    fn page_row_confirmed(&mut self, _: &PageRef, expires_ms: u64) -> Result<(), Fault> {
        self.page_rows.push(expires_ms);
        Ok(())
    }
    fn retirements(&mut self, _: &PageRef) -> Result<Vec<Retirement>, Fault> {
        Ok(self.ended.clone())
    }
    fn retired(&mut self, _: &PageRef, link: &str) -> Result<(), Fault> {
        self.ended.retain(|ended| ended.link != link);
        self.retired.push(link.into());
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
fn a_streams_chain_exists_before_its_first_original_even_when_that_arrives_mid_pass() {
    let mut rig = rig();
    let mut local = Fake {
        rows: vec![row("spaces", "s")],
        // Lands right after the pass's first read of local state, wherever in the pass it is.
        arrival: Some((1, row("authority", "chain-w"), data(1))),
        ..Fake::default()
    };
    while rig.publisher.pass(&mut local, &page()) == Ok(true) || local.confirmed.is_empty() {}
    assert_eq!(
        kinds(&rig),
        [
            "create spaces/s",
            "create authority/chain-w",
            "create log/log-1"
        ]
    );
    assert_eq!(local.confirmed, ["update-1"]);
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

fn page_row(fields: &[(&str, &str)], refresh: Duration) -> Row {
    lifetime_row(fields, 30, refresh)
}
fn lifetime_row(fields: &[(&str, &str)], days: u64, refresh: Duration) -> Row {
    Row::Put {
        document: document("pages", "p", fields),
        days,
        refresh,
        expires_ms: days * 86_400_000,
    }
}

#[test]
fn the_page_row_is_put_once_and_not_rewritten_until_it_is_old() {
    let mut rig = rig();
    let mut local = Fake {
        rows: vec![page_row(&[("epoch", "1")], Duration::from_secs(3600))],
        ..Fake::default()
    };
    rig.publisher.pass(&mut local, &page()).unwrap();
    rig.publisher.pass(&mut local, &page()).unwrap();
    assert_eq!(kinds(&rig), ["put pages/p"]);
    // Only the write is reported as confirmed, with the expiry that write carried.
    assert_eq!(local.page_rows, [30 * 86_400_000]);
}

#[test]
fn the_page_row_is_written_again_when_its_content_changes_or_its_refresh_interval_has_passed() {
    let mut rig = rig();
    let hour = Duration::from_secs(3600);
    let mut local = Fake {
        rows: vec![page_row(&[("epoch", "1"), ("state", "open")], hour)],
        ..Fake::default()
    };
    rig.publisher.pass(&mut local, &page()).unwrap();
    // The epoch advanced: the row the log creates are checked against must follow at once.
    local.rows = vec![page_row(&[("epoch", "2"), ("state", "open")], hour)];
    rig.publisher.pass(&mut local, &page()).unwrap();
    assert_eq!(kinds(&rig), ["put pages/p", "put pages/p"]);
    // The page was archived.
    local.rows = vec![page_row(&[("epoch", "2"), ("state", "archived")], hour)];
    rig.publisher.pass(&mut local, &page()).unwrap();
    assert_eq!(kinds(&rig).len(), 3);
    // Nothing changed and the interval has not passed: not written.
    rig.publisher.pass(&mut local, &page()).unwrap();
    assert_eq!(kinds(&rig).len(), 3);
    // An owner override changed the lifetime and nothing else: the row follows at once.
    local.rows = vec![lifetime_row(
        &[("epoch", "2"), ("state", "archived")],
        90,
        hour,
    )];
    rig.publisher.pass(&mut local, &page()).unwrap();
    assert_eq!(kinds(&rig).len(), 4);
    // Nothing changed but the interval is shorter than the row's age: an activity refresh.
    local.rows = vec![lifetime_row(
        &[("epoch", "2"), ("state", "archived")],
        90,
        Duration::ZERO,
    )];
    rig.publisher.pass(&mut local, &page()).unwrap();
    assert_eq!(kinds(&rig).len(), 5);
    // Each of the five writes was confirmed once, with its own expiry; the skipped pass none.
    assert_eq!(local.page_rows.len(), 5);
    assert_eq!(local.page_rows[3..], [90 * 86_400_000; 2]);
}

#[test]
fn a_page_row_the_cloud_did_not_confirm_is_not_reported() {
    let mut rig = rig();
    rig.firestore.faults.lock().unwrap().refuse_next = Some(sync::Error {
        code: sync::Code::Denied,
        state: None,
        reset_at_ms: None,
        retry_after_ms: None,
    });
    let mut local = Fake {
        rows: vec![page_row(&[("epoch", "1")], Duration::from_secs(3600))],
        ..Fake::default()
    };
    assert!(rig.publisher.pass(&mut local, &page()).is_err());
    assert!(local.page_rows.is_empty());
}

const LINK: &str = "30000000-0000-4000-8000-000000000001";
const OTHER_LINK: &str = "30000000-0000-4000-8000-000000000002";
/// A member row enrolled by `link`, as the provider holds it.
fn holder(uid: &str, link: &str) -> sync::Document {
    let page = page();
    document(
        "members",
        &format!("{}_{}_{uid}", page.space, page.page),
        &[("space", &page.space), ("page", &page.page), ("link", link)],
    )
}
fn ended(link: &str) -> Retirement {
    let page = page();
    Retirement {
        link: link.into(),
        target: documents::link_target(&page.space, &page.page, link).unwrap(),
        scope: documents::holders_scope(&page.space, &page.page, link).unwrap(),
    }
}

#[test]
fn an_ended_links_row_goes_first_then_its_holders_and_nothing_else() {
    let mut rig = rig();
    let page = page();
    let link_row = document(
        "links",
        &format!("{}_{}_{LINK}", page.space, page.page),
        &[("link", LINK)],
    );
    for doc in [
        &link_row,
        &holder("one", LINK),
        &holder("two", LINK),
        &holder("kept", OTHER_LINK),
    ] {
        rig.firestore.put(doc);
    }
    let mut local = Fake {
        rows: vec![row("spaces", "s")],
        queue: (1..=1).map(data).collect(),
        ended: vec![ended(LINK)],
        ..Fake::default()
    };
    assert_eq!(rig.publisher.pass(&mut local, &page), Ok(false));
    let prefix = format!("{}_{}", page.space, page.page);
    assert_eq!(
        kinds(&rig),
        [
            "create spaces/s".to_owned(),
            format!("delete links/{prefix}_{LINK}"),
            "discover members/".to_owned(),
            format!("delete members/{prefix}_one"),
            format!("delete members/{prefix}_two"),
            "create log/log-1".to_owned(),
        ],
        "authority first, then the link's removal, then data"
    );
    assert_eq!(local.retired, [LINK]);
    let docs = rig.firestore.docs.lock().unwrap();
    assert!(docs.contains_key(&("members".into(), format!("{prefix}_kept"))));
    assert!(!docs.keys().any(|(resource, _)| resource == "links"));
}

#[test]
fn an_ended_link_with_nothing_left_in_the_cloud_is_retired_without_error() {
    let mut rig = rig();
    let mut local = Fake {
        ended: vec![ended(LINK)],
        ..Fake::default()
    };
    assert_eq!(rig.publisher.pass(&mut local, &page()), Ok(false));
    assert_eq!(
        local.retired,
        [LINK],
        "a refused delete of an absent row reads as gone"
    );
}

#[test]
fn a_retirement_larger_than_a_pass_continues_where_the_discovery_left_off() {
    let mut rig = rig();
    rig.firestore
        .page
        .store(10, std::sync::atomic::Ordering::SeqCst);
    for n in 0..70 {
        rig.firestore.put(&holder(&format!("u{n:02}"), LINK));
    }
    let mut local = Fake {
        ended: vec![ended(LINK)],
        ..Fake::default()
    };
    assert_eq!(rig.publisher.pass(&mut local, &page()), Ok(true));
    assert!(local.retired.is_empty(), "not retired while holders remain");
    assert_eq!(
        rig.firestore.docs.lock().unwrap().len(),
        70 - PASS_DOCUMENTS
    );
    assert_eq!(rig.publisher.pass(&mut local, &page()), Ok(false));
    assert_eq!(local.retired, [LINK]);
    assert!(rig.firestore.docs.lock().unwrap().is_empty());
}

#[test]
fn a_refused_retirement_other_than_absence_stops_nothing_it_did_not_finish() {
    let mut rig = rig();
    rig.firestore.faults.lock().unwrap().refuse_next = Some(sync::Error {
        code: sync::Code::Budget,
        state: None,
        reset_at_ms: Some(1),
        retry_after_ms: Some(7_000),
    });
    let mut local = Fake {
        ended: vec![ended(LINK)],
        ..Fake::default()
    };
    assert_eq!(
        rig.publisher.pass(&mut local, &page()),
        Err(Fault::Budget(7_000))
    );
    assert!(local.retired.is_empty());
    assert_eq!(
        rig.publisher.pass(&mut local, &page()),
        Ok(false),
        "and it is retried"
    );
    assert_eq!(local.retired, [LINK]);
}
