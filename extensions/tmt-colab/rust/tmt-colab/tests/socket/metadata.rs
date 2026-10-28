//! Owner metadata frames reuse the real duplex serve, publisher and outcome store.
use super::*;

fn register_question(layout: &Layout, key: &Keyring, question: &str) {
    let before = read(layout, key);
    let store = Store::read(layout).unwrap();
    let mut decoder = tmt_colab::decoder::Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    let prepared = tmt_colab::page::prepare_structured_publication(
        &store,
        key,
        PAGE,
        tmt_colab::page::StructuredAction {
            kind: tmt_colab::decoder::StructuredKind::Questions,
            action: "question.add",
            body: &json!({"id":question,"title":"Question without source markup"}),
            expected_revision: &before.revision,
            display_agent: None,
            operation_id: None,
        },
        &mut decoder,
        &|| Ok(now()),
    )
    .unwrap();
    store.close().unwrap();
    let tmt_colab::page::PublicationPreparation::Write(frozen) = prepared else {
        panic!("new question")
    };
    tmt_colab::page::ipc::publish(layout, key, &frozen)
        .unwrap()
        .unwrap();
}

fn request_result(server: &Running, operation: &str) -> Value {
    server.decision(operation);
    let text: String = server
        .oracle()
        .query_row(
            "SELECT result FROM edit_decisions WHERE operation=?",
            [operation],
            |row| row.get(0),
        )
        .unwrap();
    let result: Value = serde_json::from_str(&text).unwrap();
    let layout = Layout::existing(&server.root).unwrap().unwrap();
    let key = Keyring::read(&layout).unwrap();
    let probe = prepare_write(&layout, &key, "unpublished identity probe");
    let writer = probe.job().manifest.stream_id.clone();
    let author = author_key(&probe);
    let expected = tmt_colab::edit_request::metadata::outcome_key(REQUEST_DEVICE, operation);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut previous = 0;
    loop {
        let db = server.oracle();
        let mut query=db.prepare("SELECT payload FROM receipts WHERE page=? AND stream=? AND namespace='own' ORDER BY seq").unwrap();
        let packets: Vec<Vec<u8>> = query
            .query_map(rusqlite::params![PAGE, writer], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        if packets.len() != previous {
            previous = packets.len();
            let updates: Vec<Vec<u8>> = packets
                .iter()
                .map(|bytes| {
                    let envelope = object::Envelope::from_json(bytes).unwrap();
                    let header = object::Header::decode(envelope.header()).unwrap();
                    assert_eq!(header.context.author_device, writer);
                    assert_eq!(header.context.page, PAGE);
                    assert_eq!(header.context.epoch, "1");
                    object::open(&envelope, &header.context, &[8; 32], &author).unwrap()
                })
                .collect();
            let refs: Vec<&[u8]> = updates.iter().map(Vec::as_slice).collect();
            let mut decoder = tmt_colab::decoder::Decoder::with_config(support::decoder_config(
                env!("CARGO_BIN_EXE_tmt-colab").into(),
            ))
            .unwrap();
            let decoded = decoder
                .decode(
                    tmt_colab::decoder::UpdateBatch {
                        namespace: tmt_colab::decoder::Namespace::Own,
                        baseline: &[],
                        updates: &refs,
                    },
                    tmt_colab::decoder::Role::Commenter,
                    None,
                )
                .unwrap();
            if let Some(receipt) = decoded.projection["messages"].get(&expected) {
                assert_eq!(
                    receipt, &result,
                    "signed owner receipt differs from durable outcome"
                );
                return result;
            }
        }
        assert!(
            Instant::now() < deadline,
            "no signed owner receipt for {operation}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn editor_metadata_changes_automatically_and_a_stale_target_never_reopens_it() {
    let server = Running::start(Tunnels::PRODUCT);
    let session = server.link_session("editor");
    let layout = Layout::existing(&server.root).unwrap().unwrap();
    let key = Keyring::read(&layout).unwrap();
    let question = request_op(90);
    register_question(&layout, &key, &question);
    let before = read(&layout, &key);
    let original = before.questions.as_ref().unwrap()[0].clone();
    let mut peer = server.reader_peer(&session);
    let mut stream = RequestStream::new();
    let ask = |stream: &mut RequestStream, peer: &mut _, n: u64, action: &str, target: Value| {
        let operation = request_op(n);
        let record = json!({"version":2,"kind":"edit-request","operationId":operation,"action":action,"body":{"id":question},"target":target});
        send(
            peer,
            stream.record(&server, &session, "intents", &operation, record),
        );
        receive_until(peer, "receipt");
        request_result(&server, &operation)
    };
    let handled = ask(
        &mut stream,
        &mut peer,
        1,
        "question.handle",
        original.clone(),
    );
    assert_eq!(handled["result"]["state"], "committed", "{handled}");
    assert!(
        handled["result"]["revision"]
            .as_str()
            .unwrap()
            .starts_with("v1:")
    );
    let current = read(&layout, &key);
    assert_eq!(current.questions.as_ref().unwrap()[0]["state"], "handled");
    assert_eq!(current.source, before.source);
    assert_eq!(current.creator, before.creator);
    let stale = ask(&mut stream, &mut peer, 2, "question.reopen", original);
    assert_eq!(stale["result"]["code"], "COLAB_STALE_BASE");
    assert_eq!(read(&layout, &key).questions, current.questions);
    let reopened = ask(
        &mut stream,
        &mut peer,
        3,
        "question.reopen",
        current.questions.as_ref().unwrap()[0].clone(),
    );
    assert_eq!(reopened["result"]["state"], "committed", "{reopened}");
    assert_eq!(
        read(&layout, &key).questions.as_ref().unwrap()[0]["state"],
        "open"
    );
    // Later actions do not replace the original operation's terminal result.
    assert_eq!(request_result(&server, &request_op(1)), handled);
}

#[test]
fn commenter_metadata_requests_are_final_refusals_without_content_changes() {
    let server = Running::start(Tunnels::PRODUCT);
    let session = server.link_session("commenter");
    let layout = Layout::existing(&server.root).unwrap().unwrap();
    let key = Keyring::read(&layout).unwrap();
    let question = request_op(90);
    register_question(&layout, &key, &question);
    let before = read(&layout, &key);
    let operation = request_op(1);
    let mut peer = server.reader_peer(&session);
    let mut stream = RequestStream::new();
    let record = json!({"version":2,"kind":"edit-request","operationId":operation,"action":"question.handle","body":{"id":question},"target":before.questions.as_ref().unwrap()[0]});
    send(
        &mut peer,
        stream.record(&server, &session, "intents", &operation, record),
    );
    receive_until(&mut peer, "receipt");
    let refused = request_result(&server, &operation);
    assert_eq!(refused["result"]["code"], "COLAB_DENIED");
    assert_eq!(read(&layout, &key).questions, before.questions);
}

#[test]
fn a_committed_metadata_operation_is_recovered_before_its_old_target_is_checked() {
    let server = Running::start(Tunnels::PRODUCT);
    let session = server.link_session("editor");
    let layout = Layout::existing(&server.root).unwrap().unwrap();
    let key = Keyring::read(&layout).unwrap();
    let question = request_op(90);
    register_question(&layout, &key, &question);
    let before = read(&layout, &key);
    let external = request_op(1);
    let operation =
        tmt_colab::edit_request::metadata::operation(PAGE, "1", REQUEST_DEVICE, &external);
    let record = json!({"version":2,"kind":"edit-request","operationId":external,"action":"question.handle",
        "body":{"id":question},"target":before.questions.as_ref().unwrap()[0]});
    let (entered, parked) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    *server.save_gate.lock().unwrap() = Some((entered, wait));
    let mut peer = server.reader_peer(&session);
    let mut stream = RequestStream::new();
    send(
        &mut peer,
        stream.record(&server, &session, "intents", &external, record.clone()),
    );
    parked.recv_timeout(Duration::from_secs(30)).unwrap();
    // Recreate the durable crash boundary: publication exists, but no request decision or
    // result exists. The consumer has no in-memory knowledge of this separate publication.
    let revision = read(&layout, &key).revision;
    let root = server.root.clone();
    let open: tmt_colab::page::save::SourceOpener = Arc::new(move || {
        tmt_colab::page::save::open_source(
            &root,
            support::decoder_config(env!("CARGO_BIN_EXE_tmt-colab").into()),
        )
    });
    let prepared = tmt_colab::page::metadata::prepare(
        &open,
        &tmt_colab::page::metadata::MetadataEdit {
            page: PAGE.into(),
            operation_id: operation.clone(),
            expected_revision: revision,
            action: "question.handle".into(),
            body: json!({"id":question}),
        },
        &|| Ok(now()),
    )
    .unwrap();
    let tmt_colab::page::save::Prepared::Write(frozen) = prepared else {
        panic!("new handling")
    };
    let published = tmt_colab::page::ipc::publish(&layout, &key, &frozen)
        .unwrap()
        .unwrap();
    let content_count = || {
        server
            .oracle()
            .query_row(
                "SELECT count(*) FROM receipts WHERE page=? AND namespace='content'",
                [PAGE],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    };
    let after_commit = content_count();
    release.send(()).unwrap();
    receive_until(&mut peer, "receipt");
    let result = request_result(&server, &external);
    assert_eq!(result["result"]["state"], "committed", "{result}");
    assert_eq!(result["result"]["revision"], published.revision.unwrap());
    assert_eq!(
        result["requestSha256"],
        values::encode_binary(&tmt_colab_model::crypto::digest(
            &tmt_colab::closed::canonical(&record).unwrap()
        ))
    );
    assert_eq!(
        content_count(),
        after_commit,
        "recovery published a second metadata action"
    );
    assert_eq!(
        read(&layout, &key).questions.as_ref().unwrap()[0]["state"],
        "handled"
    );
    let store = Store::read(&layout).unwrap();
    let status = tmt_colab::page::publication_status_by_operation(&store, &key, PAGE, &operation)
        .unwrap()
        .unwrap();
    assert_eq!(status.bytes, published.record.bytes);
    store.close().unwrap();
}

#[test]
fn a_link_authored_result_cannot_suppress_the_owners_metadata_publication() {
    let server = Running::start(Tunnels::PRODUCT);
    let session = server.link_session("editor");
    let layout = Layout::existing(&server.root).unwrap().unwrap();
    let key = Keyring::read(&layout).unwrap();
    let question = request_op(90);
    register_question(&layout, &key, &question);
    let before = read(&layout, &key);
    let operation = request_op(1);
    let mut peer = server.reader_peer(&session);
    let mut stream = RequestStream::new();
    let request = json!({"version":2,"kind":"edit-request","operationId":operation,"action":"question.handle",
        "body":{"id":question},"target":before.questions.as_ref().unwrap()[0]});
    let fake = json!({"version":1,"kind":"edit-result","writer":REQUEST_DEVICE,"operationId":operation,
        "requestSha256":values::encode_binary(&tmt_colab_model::crypto::digest(&tmt_colab::closed::canonical(&request).unwrap())),
        "result":{"operationId":operation,"state":"rejected","code":"COLAB_DENIED","message":"A link says no."}});
    let outcome_key = tmt_colab::edit_request::metadata::outcome_key(REQUEST_DEVICE, &operation);
    send(
        &mut peer,
        stream.record(&server, &session, "messages", &outcome_key, fake),
    );
    receive_until(&mut peer, "receipt");
    send(
        &mut peer,
        stream.record(&server, &session, "intents", &operation, request),
    );
    receive_until(&mut peer, "receipt");
    let genuine = request_result(&server, &operation);
    assert_eq!(genuine["result"]["state"], "committed", "{genuine}");
    assert_eq!(
        read(&layout, &key).questions.as_ref().unwrap()[0]["state"],
        "handled"
    );
}

#[test]
fn a_lost_metadata_receipt_republishes_the_first_refusal_without_reevaluation() {
    let server = Running::start(Tunnels::PRODUCT);
    let session = server.link_session("editor");
    let layout = Layout::existing(&server.root).unwrap().unwrap();
    let key = Keyring::read(&layout).unwrap();
    let question = request_op(90);
    register_question(&layout, &key, &question);
    let before = read(&layout, &key);
    let operation = request_op(1);
    let mut peer = server.reader_peer(&session);
    let mut stream = RequestStream::new();
    let request = json!({"version":2,"kind":"edit-request","operationId":operation,"action":"question.handle",
        "body":{"id":question},"target":before.questions.as_ref().unwrap()[0]});
    let (entered, parked) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    *server.save_gate.lock().unwrap() = Some((entered, wait));
    send(
        &mut peer,
        stream.record(&server, &session, "intents", &operation, request.clone()),
    );
    parked.recv_timeout(Duration::from_secs(30)).unwrap();
    // The immutable request is stored. Recreate the durable state after a refusal decision but
    // before its receipt; reevaluating now would incorrectly handle the still-open question.
    let terminal = json!({"version":1,"kind":"edit-result","writer":REQUEST_DEVICE,"operationId":operation,
        "requestSha256":values::encode_binary(&tmt_colab_model::crypto::digest(&tmt_colab::closed::canonical(&request).unwrap())),
        "result":{"operationId":operation,"state":"rejected","code":"COLAB_DENIED","message":"The first terminal refusal."}});
    let mut store = Store::open(&layout).unwrap();
    store
        .decide_metadata(
            PAGE,
            1,
            REQUEST_DEVICE,
            &operation,
            tmt_colab::store::EditState::Refused,
            &terminal,
        )
        .unwrap();
    store.close().unwrap();
    release.send(()).unwrap();
    receive_until(&mut peer, "receipt");
    assert_eq!(request_result(&server, &operation), terminal);
    assert_eq!(read(&layout, &key).questions, before.questions);
    assert_eq!(read(&layout, &key).source, before.source);
}

#[test]
fn an_old_metadata_receipt_is_not_republished_into_a_rotated_page_epoch() {
    use tmt_colab::transitions::{Engine, LinkAction, OwnerAction, OwnerRequest};
    let mut server = Running::start(Tunnels::PRODUCT);
    let session = server.link_session("editor");
    let layout = Layout::existing(&server.root).unwrap().unwrap();
    let key = Keyring::read(&layout).unwrap();
    let question = request_op(90);
    register_question(&layout, &key, &question);
    let before = read(&layout, &key);
    let operation = request_op(1);
    let mut peer = server.reader_peer(&session);
    let mut stream = RequestStream::new();
    let request = json!({"version":2,"kind":"edit-request","operationId":operation,"action":"question.handle",
        "body":{"id":question},"target":before.questions.as_ref().unwrap()[0]});
    let (entered, parked) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    // Pending read, metadata target read and metadata preparation finish; receipt open parks.
    server.save_gate_skip.store(3, Ordering::SeqCst);
    *server.save_gate.lock().unwrap() = Some((entered, wait));
    send(
        &mut peer,
        stream.record(&server, &session, "intents", &operation, request),
    );
    parked.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(server.decision(&operation), "applied");
    let mut store = Store::open(&layout).unwrap();
    let expected_revision = store
        .owner_head(&server.space, &key.owner_public())
        .unwrap()
        .unwrap()
        .revision;
    Engine::with_decoder_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap()
    .apply(
        &mut store,
        &key,
        OwnerRequest {
            operation_id: &request_op(77),
            expected_revision,
            action: OwnerAction::Link(LinkAction::Remove {
                link_id: REQUEST_LINK,
            }),
            transport_digest: None,
            scope: None,
        },
        now(),
    )
    .unwrap();
    store.close().unwrap();
    let epoch: String = server
        .oracle()
        .query_row("SELECT epoch FROM pages WHERE page=?", [PAGE], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(epoch.parse::<u64>().unwrap() > 1);
    let database = server.oracle();
    let own_count = || {
        database
            .query_row(
                "SELECT count(*) FROM receipts WHERE page=? AND epoch=? AND namespace='own'",
                rusqlite::params![PAGE, epoch],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    };
    let count = own_count();
    release.send(()).unwrap();
    // Join the in-flight writer before inspecting durable state; dropping the private socket
    // harness joins handlers and does not substitute a timer for completion.
    server.stop.store(true, Ordering::SeqCst);
    server.worker.take().unwrap().join().unwrap();
    assert_eq!(own_count(), count, "old result crossed the epoch boundary");
}

#[test]
fn concurrent_consumers_leave_one_metadata_operation_with_its_current_owner() {
    let server = Running::start(Tunnels::PRODUCT);
    let session = server.link_session("editor");
    let layout = Layout::existing(&server.root).unwrap().unwrap();
    let key = Keyring::read(&layout).unwrap();
    let question = request_op(90);
    register_question(&layout, &key, &question);
    let before = read(&layout, &key);
    let operation = request_op(1);
    let mut first = server.reader_peer(&session);
    let mut stream = RequestStream::new();
    let (second_session, second_id) = server.derived_link_session(&session, LinkTransport::Hosted);
    let mut second = server.reader_peer(&second_session);
    second
        .get_ref()
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let (entered, parked) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel();
    // Hold the first consumer at target/identity preparation, after its pending read.
    server.save_gate_skip.store(1, Ordering::SeqCst);
    *server.save_gate.lock().unwrap() = Some((entered, wait));
    let request = json!({"version":2,"kind":"edit-request","operationId":operation,"action":"question.handle",
        "body":{"id":question},"target":before.questions.as_ref().unwrap()[0]});
    send(
        &mut first,
        stream.record(&server, &session, "intents", &operation, request),
    );
    parked.recv_timeout(Duration::from_secs(30)).unwrap();
    let signer = tmt_colab_model::link::device_signer(
        &LINK_SEED_BYTES,
        &server.space,
        LINK_ID,
        LinkTransport::Hosted,
    )
    .unwrap();
    let mut holder = RequestStream::of(second_id, signer);
    send(
        &mut second,
        holder.record(
            &server,
            &second_session,
            "messages",
            "wake",
            json!({"note":"another consumer turn"}),
        ),
    );
    second.send(Message::Ping(vec![91].into())).unwrap();
    // Pong is a fence after this connection's append/settlement turn, not a wall-clock wait.
    loop {
        if matches!(second.read().unwrap(), Message::Pong(_)) {
            break;
        }
    }
    let decided: i64 = server
        .oracle()
        .query_row(
            "SELECT count(*) FROM edit_decisions WHERE operation=?",
            [&operation],
            |row| row.get(0),
        )
        .unwrap();
    let during = read(&layout, &key).questions;
    // Always release the parked writer before any assertion can unwind its channel.
    release.send(()).unwrap();
    receive_until(&mut first, "receipt");
    let result = request_result(&server, &operation);
    assert_eq!(decided, 0, "another consumer took the in-flight operation");
    assert_eq!(during, before.questions);
    assert_eq!(result["result"]["state"], "committed");
    assert_eq!(
        read(&layout, &key).questions.as_ref().unwrap()[0]["state"],
        "handled"
    );
}

fn read(layout: &Layout, key: &Keyring) -> tmt_colab::page::Page {
    let store = Store::read(layout).unwrap();
    let mut decoder = tmt_colab::decoder::Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    let value = tmt_colab::page::read(&store, key, PAGE, &mut decoder).unwrap();
    store.close().unwrap();
    value
}

#[test]
fn owner_metadata_fences_revision_preserves_source_and_recovers_through_save_status() {
    let (server, layout, key, mut peer) = publish_fixture();
    peer.get_ref()
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    for frame in server.save_frames(SAVE_ONE, "", "<p>Keep this source</p>") {
        send(&mut peer, frame);
    }
    let saved = receive_acked(&server, &mut peer, "saveresult");
    let before = read(&layout, &key);
    let body =
        json!({"agent":{"id":"10000000-0000-4000-8000-000000000099","name":"A new creator"}});
    let change = |peer: &mut _, operation: &str, revision: &Value, action: &str, body: Value| {
        send(peer, server.frame("metadata", json!({"operationId":operation,"expectedRevision":revision,"action":action,"body":body})));
        receive_acked(&server, peer, "metadataresult")
    };
    let first = change(
        &mut peer,
        WORK_ONE,
        &saved["revision"],
        "creator.set",
        body.clone(),
    );
    assert_eq!(first["state"], "committed", "{first}");
    let current = read(&layout, &key);
    assert_eq!(current.creator.as_ref().unwrap()["agent"], body["agent"]);
    assert_eq!(current.source, before.source);
    assert_eq!(current.original_author, before.original_author);
    send(
        &mut peer,
        server.frame("savestatus", json!({"operationId":WORK_ONE})),
    );
    let recovered = receive_acked(&server, &mut peer, "saveresult");
    assert_eq!(recovered["state"], "committed");
    assert_eq!(recovered["revision"], first["revision"]);
    let stale = change(
        &mut peer,
        WORK_TWO,
        &saved["revision"],
        "creator.set",
        json!({"agent":{"id":"10000000-0000-4000-8000-000000000098","name":"Stale creator"}}),
    );
    assert_eq!(stale["code"], "COLAB_STALE_BASE", "{stale}");
    assert_eq!(read(&layout, &key).revision, current.revision);
    let unchanged = change(
        &mut peer,
        WORK_THREE,
        &first["revision"],
        "creator.set",
        body.clone(),
    );
    // Attribution has its own time, so a repeated selection may commit a fresh attributed choice.
    assert!(
        matches!(unchanged["state"].as_str(), Some("unchanged" | "committed")),
        "{unchanged}"
    );
    assert_eq!(read(&layout, &key).source, before.source);
    let before_invalid = read(&layout, &key).revision;
    send(&mut peer, server.frame("metadata", json!({"operationId":WORK_FOUR,"expectedRevision":first["revision"],"action":"question.add","body":{"id":PAGE}})));
    assert_eq!(receive_acked(&server, &mut peer, "error")["type"], "error");
    assert_eq!(read(&layout, &key).revision, before_invalid);
}

#[test]
fn link_roles_cannot_send_metadata_even_with_well_formed_identity_and_revision() {
    for role in ["viewer", "commenter", "editor"] {
        let server = Running::start(Tunnels::PRODUCT);
        let session = server.link_session(role);
        let layout = Layout::existing(&server.root).unwrap().unwrap();
        let key = Keyring::read(&layout).unwrap();
        let before = read(&layout, &key);
        let mut peer = server.reader_peer(&session);
        send(&mut peer, server.reader_frame(&session, "metadata", json!({
            "operationId":WORK_ONE,"expectedRevision":before.revision,"action":"creator.set",
            "body":{"agent":{"id":"10000000-0000-4000-8000-000000000099","name":"No authority"}}
        })));
        let (error, frames) = receive_until(&mut peer, "error");
        assert_eq!(error["type"], "error", "{role}");
        assert!(frames.iter().all(|frame| frame["type"] != "metadataresult"));
        assert_eq!(read(&layout, &key).revision, before.revision);
    }
}

#[test]
fn owner_metadata_handles_and_reopens_a_registered_question_removed_from_source() {
    let (server, layout, key, mut peer) = publish_fixture();
    peer.get_ref()
        .set_read_timeout(Some(Duration::from_secs(60)))
        .unwrap();
    for frame in server.save_frames(SAVE_ONE, "", "<p>No question markup remains</p>") {
        send(&mut peer, frame);
    }
    receive_acked(&server, &mut peer, "saveresult");
    let before = read(&layout, &key);
    let store = Store::read(&layout).unwrap();
    let mut decoder = tmt_colab::decoder::Decoder::with_config(support::decoder_config(
        env!("CARGO_BIN_EXE_tmt-colab").into(),
    ))
    .unwrap();
    let question = "10000000-0000-4000-8000-000000000055";
    let prepared = tmt_colab::page::prepare_structured_publication(
        &store,
        &key,
        PAGE,
        tmt_colab::page::StructuredAction {
            kind: tmt_colab::decoder::StructuredKind::Questions,
            action: "question.add",
            body: &json!({"id":question,"title":"Removed question"}),
            expected_revision: &before.revision,
            display_agent: None,
            operation_id: None,
        },
        &mut decoder,
        &|| Ok(now()),
    )
    .unwrap();
    store.close().unwrap();
    let tmt_colab::page::PublicationPreparation::Write(frozen) = prepared else {
        panic!("new registration");
    };
    tmt_colab::page::ipc::publish(&layout, &key, &frozen)
        .unwrap()
        .unwrap();
    for (operation, action, expected) in [
        (WORK_ONE, "question.handle", "handled"),
        (WORK_TWO, "question.reopen", "open"),
    ] {
        let revision = read(&layout, &key).revision;
        send(&mut peer, server.frame("metadata", json!({"operationId":operation,"expectedRevision":revision,"action":action,"body":{"id":question}})));
        let result = receive_acked(&server, &mut peer, "metadataresult");
        assert_eq!(result["state"], "committed", "{result}");
        let current = read(&layout, &key);
        assert_eq!(current.questions.as_ref().unwrap()[0]["state"], expected);
        assert_eq!(current.source, before.source);
    }
}

#[test]
fn manual_page_state_uses_owner_metadata_admission_and_denies_share_writers() {
    let (server, layout, key, mut peer) = publish_fixture();
    let before = read(&layout, &key);
    assert_eq!(before.state.as_deref(), Some("todo"));
    send(
        &mut peer,
        server.frame(
            "metadata",
            json!({
                "operationId": WORK_ONE, "expectedRevision": before.revision,
                "action": "page.state", "body": {"state":"done"}
            }),
        ),
    );
    let result = receive_acked(&server, &mut peer, "metadataresult");
    assert_eq!(result["state"], "committed", "{result}");
    let after = read(&layout, &key);
    assert_eq!(after.state.as_deref(), Some("done"));
    assert_eq!(after.source, before.source);
    drop(peer);
    drop(server);
    for role in ["viewer", "commenter", "editor"] {
        let server = Running::start(Tunnels::PRODUCT);
        let layout = Layout::existing(&server.root).unwrap().unwrap();
        let key = Keyring::read(&layout).unwrap();
        let session = server.link_session(role);
        let current = read(&layout, &key);
        let mut reader = server.reader_peer(&session);
        send(
            &mut reader,
            server.reader_frame(
                &session,
                "metadata",
                json!({
                    "operationId": WORK_TWO, "expectedRevision": current.revision,
                    "action": "page.state", "body": {"state":"todo"}
                }),
            ),
        );
        let (error, frames) = receive_until(&mut reader, "error");
        assert_eq!(error["code"], "DENIED", "{error}");
        assert!(frames.iter().all(|frame| frame["type"] != "metadataresult"));
        assert_eq!(read(&layout, &key).state, current.state);
    }
}
