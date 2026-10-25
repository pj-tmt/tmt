//! Owner metadata frames reuse the real duplex serve, publisher and outcome store.
use super::*;

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
