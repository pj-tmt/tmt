use super::test_support::*;
use super::*;
use crate::{
    registration::{OwnerAdmission, Registration},
    sync::{Access, Admission},
    transitions::{OwnerAction, ShareMode},
};
use ed25519_dalek::SigningKey;
use std::{os::unix::net::UnixStream, sync::atomic::Ordering};
use tungstenite::{Message, WebSocket, protocol::Role};
#[test]
fn possession_scope_replay_expiry_and_binding_fail_closed() {
    let mut f = Fixture::new();
    assert_eq!(
        f.request(
            CHALLENGE_PATH,
            json!({"kind":"public","space":f.key.space_id,"page":PAGE})
        ),
        Err(Code::Denied)
    );
    f.share(ShareMode::Link);
    f.add_link();
    let mut bad: serde_json::Value = serde_json::from_slice(&f.chain).unwrap();
    bad["issuerSignature"] = json!(values::encode_binary(&[0; 64]));
    assert_eq!(f.request(CHALLENGE_PATH,json!({"kind":"link","space":f.key.space_id,"page":PAGE,"chain":values::encode_binary(&serde_json::to_vec(&bad).unwrap())})),Err(Code::Denied));
    let c = f.challenge(true);
    let mut proof = f.exchange_body(&c, true);
    proof["signature"] = json!(values::encode_binary(&[0; 64]));
    assert_eq!(f.request(SESSION_PATH, proof), Err(Code::Denied));
    assert_eq!(
        f.request(SESSION_PATH, f.exchange_body(&c, true)),
        Err(Code::Denied)
    );
    let c = f.challenge(true);
    f.clock.store(61_000, Ordering::SeqCst);
    assert_eq!(
        f.request(SESSION_PATH, f.exchange_body(&c, true)),
        Err(Code::Expired)
    );
    f.clock.store(1000, Ordering::SeqCst);
    let c = f.challenge(true);
    let proof = f.exchange_body(&c, true);
    let s = f.request(SESSION_PATH, proof.clone()).unwrap();
    assert_eq!(f.request(SESSION_PATH, proof), Err(Code::Denied));
    let id = f.upgrade(&s).unwrap();
    assert_eq!(f.upgrade(&s), Err(Code::Denied));
    // A valid new certificate cannot replace a device's admitted key binding.
    let original = f.chain.clone();
    let old = certificate::Chain::from_json(&original).unwrap();
    let cert = old.certificate().unwrap();
    let different = tmt_colab_model::wrap::RecipientKey::from_seed(&[21; 32])
        .unwrap()
        .public_key();
    let input = certificate::input(&certificate::Certificate {
        encryption_key: &different,
        ..cert
    })
    .unwrap();
    let keys = tmt_colab_model::link::Keys::derive(&SEED, &f.key.space_id, LINK).unwrap();
    let mut replacement: serde_json::Value = serde_json::from_slice(&original).unwrap();
    replacement["deviceCertificate"] = json!(values::encode_binary(&input));
    replacement["issuerSignature"] = json!(values::encode_binary(
        &keys.certify(&certificate::decode(&input).unwrap()).unwrap()
    ));
    f.chain = serde_json::to_vec(&replacement).unwrap();
    let c = f.challenge(true);
    assert_eq!(
        f.request(SESSION_PATH, f.exchange_body(&c, true)),
        Err(Code::Denied)
    );
    let stored = f
        .store
        .owner_read(&f.key.space_id, &f.key.owner_public(), |tx| {
            Ok(tx.device(DEVICE)?.unwrap().chain)
        })
        .unwrap();
    assert_eq!(stored, original);
    f.chain = original;
    let scope = f.scope();
    assert_eq!(
        OwnerAdmission(f.service.clone()).authorize(&id, &scope, Access::Publish),
        Err(crate::sync::Code::Denied)
    );
    for namespace in ["content", "own"] {
        let c = tmt_colab_model::object::Context {
            space: scope.space.clone(),
            page: scope.page.clone(),
            epoch: scope.epoch.clone(),
            kind: "update".into(),
            namespace: namespace.into(),
            author_device: DEVICE.into(),
            membership_revision: "2".into(),
            stream_seq: "1".into(),
            prev_hash: [0; 32],
        };
        assert_eq!(
            OwnerAdmission(f.service.clone()).authorize(&id, &scope, Access::Append(&c)),
            Err(crate::sync::Code::Denied)
        );
    }

    assert!(f.read(&id, &scope).is_ok());
    for wrong in [
        SyncScope {
            page: OTHER.into(),
            ..scope.clone()
        },
        SyncScope {
            epoch: "99".into(),
            ..scope.clone()
        },
    ] {
        assert_eq!(f.read(&id, &wrong), Err(crate::sync::Code::Denied));
    }
    f.clock.store(601_000, Ordering::SeqCst);
    assert_eq!(f.read(&id, &scope), Err(crate::sync::Code::Expired));
    assert_eq!(
        OwnerAdmission(f.service.clone()).alive(&id),
        Err(crate::sync::Code::Expired)
    );
}
#[test]
fn reader_wrap_selection_preserves_owner_bytes_and_never_delivers_owner_keys() {
    let mut f = Fixture::new();
    f.share(ShareMode::Link);
    f.add_link();
    f.apply(OwnerAction::EpochAdvance { page: PAGE });
    let s = f.session(true);
    let id = f.upgrade(&s).unwrap();
    let scope = f.scope();
    let owner = f.key.management_member().unwrap();
    let db = rusqlite::Connection::open(f.root.join("colab/space.db")).unwrap();
    let head = f
        .store
        .owner_head(&f.key.space_id, &f.key.owner_public())
        .unwrap()
        .unwrap();
    let mut query=db.prepare("SELECT envelope FROM wraps WHERE page=?1 AND epoch<=?2 AND revision<=?3 AND (kind='device' AND recipient=?4 OR kind='member' AND recipient=?5) ORDER BY epoch,kind,recipient,revision").unwrap();
    let expected = query
        .query_map(
            rusqlite::params![
                PAGE,
                format!("{:020}", values::decimal(&scope.epoch, false).unwrap()),
                format!("{:020}", head.revision),
                DEVICE,
                owner.id
            ],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .unwrap()
        .map(|r| values::encode_binary(&r.unwrap()))
        .collect::<Vec<_>>();
    let head = f
        .store
        .owner_head(&f.key.space_id, &f.key.owner_public())
        .unwrap()
        .unwrap();
    let (actual, more) = f
        .store
        .owner_read(&f.key.space_id, &f.key.owner_public(), |tx| {
            tx.wrap_page(
                PAGE,
                values::decimal(&scope.epoch, false)?,
                DEVICE,
                &head,
                0,
                &WrapRecipients::Owner,
            )
        })
        .unwrap();
    assert!(!more);
    assert!(!actual.is_empty());
    assert_eq!(actual, expected);
    let recipients = OwnerAdmission(f.service.clone())
        .catchup_context(&id, &scope, &f.store)
        .unwrap()
        .recipients;
    let (links, _) = f
        .store
        .owner_read(&f.key.space_id, &f.key.owner_public(), |tx| {
            tx.wrap_page(
                PAGE,
                values::decimal(&scope.epoch, false)?,
                &id,
                &head,
                0,
                &recipients,
            )
        })
        .unwrap();
    assert!(!links.is_empty());
    for bytes in links {
        let e = tmt_colab_model::wrap::Envelope::from_json(&values::binary(&bytes, 2048).unwrap())
            .unwrap();
        let h = e.header().unwrap();
        assert_eq!(h.recipient_kind, "link");
        assert_eq!(h.recipient_id, LINK);
    }
    f.share(ShareMode::Public);
    let s = f.session(false);
    let id = f.upgrade(&s).unwrap();
    assert!(matches!(
        OwnerAdmission(f.service.clone())
            .catchup_context(&id, &f.scope(), &f.store)
            .unwrap()
            .recipients,
        WrapRecipients::None
    ));
}
#[test]
fn bounded_sessions_and_restart_drop_capabilities_without_eviction() {
    let mut f = Fixture::new();
    f.share(ShareMode::Public);
    let s = f.session(false);
    let id = f.upgrade(&s).unwrap();
    let scope = f.scope();
    for _ in 1..CAP {
        f.challenge(false);
    }
    assert_eq!(
        f.request(
            CHALLENGE_PATH,
            json!({"kind":"public","space":f.key.space_id,"page":PAGE})
        ),
        Err(Code::Capacity)
    );
    assert!(f.read(&id, &scope).is_ok());
    f.clock.store(61_000, Ordering::SeqCst);
    f.challenge(false);
    assert!(f.read(&id, &scope).is_ok());
    f.service.lock().unwrap().release_reader(&id);
    assert_eq!(f.read(&id, &scope), Err(crate::sync::Code::Denied));
    let layout = crate::keyring::Layout::open(&f.root).unwrap();
    let mut restarted = Registration::with_decoder_config(
        Store::open(&layout).unwrap(),
        Keyring::read(&layout).unwrap(),
        decoder_config(std::env::current_exe().unwrap()),
    )
    .unwrap();
    let token = values::binary(s["token"].as_str().unwrap(), 32)
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(restarted.reader_upgrade(&token), Err(Code::Denied));
    assert_eq!(
        upgrade_token(&["colab-reader-v1.bad".into()]),
        Err(Code::Invalid)
    );
    assert_eq!(
        upgrade_token(&[
            format!("colab-reader-v1.{}", s["token"].as_str().unwrap()),
            format!("colab-reader-v1.{}", s["token"].as_str().unwrap())
        ]),
        Err(Code::Invalid)
    );
}
#[test]
fn prehello_tunnel_is_closed_when_policy_changes() {
    let mut f = Fixture::new();
    f.share(ShareMode::Public);
    let s = f.session(false);
    let id = f.upgrade(&s).unwrap();
    let (server, client) = UnixStream::pair().unwrap();
    server.set_nonblocking(true).unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    let mut connection = f.server.connect(server, id).unwrap();
    let mut peer = WebSocket::from_raw_socket(client, Role::Client, None);
    f.share(ShareMode::Private);
    assert_eq!(connection.poll(), crate::sync::Progress::Closed);
    assert!(matches!(
        peer.read(),
        Ok(Message::Close(_))
            | Err(tungstenite::Error::ConnectionClosed)
            | Err(tungstenite::Error::Protocol(
                tungstenite::error::ProtocolError::ResetWithoutClosingHandshake
            ))
    ));
}

#[test]
fn attachment_context_requires_an_active_actual_reader_and_its_exact_epoch_entitlement() {
    let mut f = Fixture::new();
    f.share(ShareMode::Link);
    f.add_link();
    let ticket = f.session(true);
    let principal = ticket["principal"].as_str().unwrap();
    let admission = OwnerAdmission(f.service.clone());
    assert!(
        admission
            .attachment_read_owner(principal, &f.scope())
            .is_err()
    );
    let id = f.upgrade(&ticket).unwrap();
    let scope = f.scope();
    let epoch = values::decimal(&scope.epoch, false).unwrap();
    let context = admission.attachment_context(&id, &scope, epoch).unwrap();
    assert_eq!(
        admission.attachment_context(&id, &scope, epoch).unwrap(),
        context
    );
    assert!(
        admission
            .attachment_context(&id, &scope, epoch + 1)
            .is_err()
    );
    assert!(
        admission
            .attachment_context(
                &id,
                &SyncScope {
                    page: OTHER.into(),
                    ..scope.clone()
                },
                epoch
            )
            .is_err()
    );
    f.clock.store(601_000, Ordering::SeqCst);
    assert!(admission.attachment_context(&id, &scope, epoch).is_err());
    f.service.lock().unwrap().release_reader(&id);
    assert!(admission.attachment_context(&id, &scope, epoch).is_err());
}

#[test]
fn attachment_context_uses_shared_history_wraps_and_owner_signed_public_keys() {
    let mut f = Fixture::new();
    f.share(ShareMode::Link);
    f.add_link();
    f.apply(OwnerAction::EpochAdvance { page: PAGE });
    let ticket = f.session(true);
    let id = f.upgrade(&ticket).unwrap();
    let admission = OwnerAdmission(f.service.clone());
    let scope = f.scope();
    assert!(admission.attachment_context(&id, &scope, 1).is_ok());
    assert!(admission.attachment_read_owner(&id, &scope).is_ok());
    f.share(ShareMode::Public);
    let ticket = f.session(false);
    let public = f.upgrade(&ticket).unwrap();
    assert!(admission.attachment_context(&public, &f.scope(), 1).is_ok());
    f.share(ShareMode::Private);
    assert!(
        admission
            .attachment_context(&public, &f.scope(), 1)
            .is_err()
    );
    assert!(admission.attachment_context(&id, &f.scope(), 1).is_err());
}

#[test]
fn attachment_history_refuses_ambient_owner_keys_for_a_current_only_reader() {
    let mut f = Fixture::new();
    f.share(ShareMode::Link);
    f.apply(OwnerAction::History {
        page: PAGE,
        mode: crate::transitions::HistoryMode::Current,
    });
    f.apply(OwnerAction::EpochAdvance { page: PAGE });
    f.add_link();
    let ticket = f.session(true);
    let id = f.upgrade(&ticket).unwrap();
    let admission = OwnerAdmission(f.service.clone());
    let scope = f.scope();
    assert!(
        admission
            .attachment_context(&id, &scope, values::decimal(&scope.epoch, false).unwrap())
            .is_ok()
    );
    assert!(admission.attachment_context(&id, &scope, 1).is_err());
    // The local owner still holds epoch 1; that is not this reader's entitlement.
    assert!(
        f.store
            .owner_read(&f.key.space_id, &f.key.owner_public(), |tx| tx
                .epoch_secret(PAGE, 1))
            .unwrap()
            .is_some()
    );
}

fn context(
    f: &Fixture,
    namespace: &str,
    device: &str,
    revision: u64,
) -> tmt_colab_model::object::Context {
    let scope = f.scope();
    tmt_colab_model::object::Context {
        space: scope.space,
        page: scope.page,
        epoch: scope.epoch,
        kind: "update".into(),
        namespace: namespace.into(),
        author_device: device.into(),
        membership_revision: revision.to_string(),
        stream_seq: "1".into(),
        prev_hash: [0; 32],
    }
}

#[test]
fn a_link_session_stores_what_its_role_may_and_only_as_its_own_device() {
    let key = SigningKey::from_bytes(&[19; 32]).verifying_key().to_bytes();
    for (role, content, own) in [
        ("viewer", false, false),
        ("commenter", false, true),
        ("editor", true, true),
    ] {
        let mut f = Fixture::new();
        f.share(ShareMode::Link);
        f.add_link_as(role);
        let id = f.upgrade(&f.session(true)).unwrap();
        let (admission, scope, head) = (OwnerAdmission(f.service.clone()), f.scope(), f.revision());
        assert_eq!(
            admission.device(&id),
            DEVICE,
            "{role}: the stream is the link device's"
        );
        // Awareness, saves, usage, compaction and object requests belong to no link.
        assert_eq!(
            admission.authorize(&id, &scope, Access::Publish),
            Err(crate::sync::Code::Denied)
        );
        assert_eq!(
            admission.authorize(&id, &scope, Access::Write),
            if role == "viewer" {
                Err(crate::sync::Code::Denied)
            } else {
                Ok(key)
            },
            "{role}: may open an upload"
        );
        for (namespace, allowed) in [("content", content), ("own", own)] {
            let append = |device: &str, revision: u64| {
                admission.authorize(
                    &id,
                    &scope,
                    Access::Append(&context(&f, namespace, device, revision)),
                )
            };
            assert_eq!(
                append(DEVICE, head),
                if allowed {
                    Ok(key)
                } else {
                    Err(crate::sync::Code::Denied)
                },
                "{role}: {namespace}"
            );
            // Another device's stream, or a stale membership revision, is never admitted.
            assert_eq!(append(OTHER, head), Err(crate::sync::Code::Denied));
            assert_eq!(append(DEVICE, head - 1), Err(crate::sync::Code::Denied));
        }
    }
}

#[test]
fn a_public_session_and_a_removed_link_store_nothing() {
    let mut f = Fixture::new();
    f.share(ShareMode::Public);
    let public = f.upgrade(&f.session(false)).unwrap();
    let admission = OwnerAdmission(f.service.clone());
    let scope = f.scope();
    assert_eq!(admission.device(&public), public, "no chain, no device");
    assert_eq!(
        admission.authorize(&public, &scope, Access::Write),
        Err(crate::sync::Code::Denied)
    );
    let head = f.revision();
    assert_eq!(
        admission.authorize(
            &public,
            &scope,
            Access::Append(&context(&f, "own", DEVICE, head))
        ),
        Err(crate::sync::Code::Denied)
    );
    f.share(ShareMode::Link);
    f.add_link();
    let id = f.upgrade(&f.session(true)).unwrap();
    let scope = f.scope();
    assert!(admission.authorize(&id, &scope, Access::Write).is_ok());
    f.apply(OwnerAction::Link(crate::transitions::LinkAction::Remove {
        link_id: LINK,
    }));
    assert_eq!(
        admission.authorize(&id, &f.scope(), Access::Write),
        Err(crate::sync::Code::Denied)
    );
    let head = f.revision();
    assert_eq!(
        admission.authorize(
            &id,
            &f.scope(),
            Access::Append(&context(&f, "own", DEVICE, head))
        ),
        Err(crate::sync::Code::Denied)
    );
}

/// One frame in, whatever the server answers out; a close or silence reads as no receipt.
fn answer(
    connection: &mut crate::sync::Connection<UnixStream, OwnerAdmission>,
    peer: &mut WebSocket<UnixStream>,
    frame: serde_json::Value,
) -> serde_json::Value {
    if peer.send(Message::Text(frame.to_string().into())).is_err() {
        return json!({"type": "closed"});
    }
    for _ in 0..8 {
        let _ = connection.poll();
    }
    match peer.read() {
        Ok(Message::Text(text)) => serde_json::from_str(&text).unwrap(),
        _ => json!({"type": "closed"}),
    }
}
fn update(
    f: &Fixture,
    namespace: &str,
    seq: u64,
    previous: [u8; 32],
) -> (serde_json::Value, [u8; 32]) {
    let scope = f.scope();
    let mut c = context(f, namespace, DEVICE, f.revision());
    (c.stream_seq, c.prev_hash) = (seq.to_string(), previous);
    let envelope = tmt_colab_model::object::seal(
        &c,
        &[11; 32],
        &SigningKey::from_bytes(&[19; 32]),
        namespace.as_bytes(),
    )
    .unwrap();
    let hash = envelope.hash().unwrap();
    (
        json!({"type":"append","version":1,"space":scope.space,"page":scope.page,"epoch":scope.epoch,
            "streamId":DEVICE,"seq":seq.to_string(),"envelopeHash":values::encode_binary(&hash),
            "envelope":values::encode_binary(&envelope.to_json().unwrap())}),
        hash,
    )
}

#[test]
fn an_append_frame_from_a_link_session_is_stored_as_the_link_devices_stream_by_role() {
    // (role, namespace, accepted)
    for (role, accepted) in [
        ("viewer", [false, false]),
        ("commenter", [false, true]),
        ("editor", [true, true]),
    ] {
        let mut f = Fixture::new();
        f.share(ShareMode::Link);
        f.add_link_as(role);
        let id = f.upgrade(&f.session(true)).unwrap();
        let (server, client) = UnixStream::pair().unwrap();
        server.set_nonblocking(true).unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        let mut connection = f.server.connect(server, id).unwrap();
        let mut peer = WebSocket::from_raw_socket(client, Role::Client, None);
        let (mut seq, mut previous) = (0, [0; 32]);
        for (namespace, accepted) in ["content", "own"].into_iter().zip(accepted) {
            let (frame, hash) = update(&f, namespace, seq + 1, previous);
            let reply = answer(&mut connection, &mut peer, frame);
            assert_eq!(
                reply["type"] == "receipt",
                accepted,
                "{role} {namespace}: {reply}"
            );
            if accepted {
                (seq, previous) = (seq + 1, hash);
                assert_eq!(reply["streamId"], DEVICE);
            }
            let stored = f
                .store
                .stream_head(crate::store::StreamScope {
                    page: PAGE,
                    epoch: f.scope().epoch.parse().unwrap(),
                    stream: DEVICE,
                })
                .unwrap();
            assert_eq!(stored.map_or(0, |(seq, _)| seq), seq, "{role} {namespace}");
        }
    }
}

#[test]
fn no_link_has_the_serve_save_page_text_or_the_rest_of_a_writer() {
    for role in ["viewer", "commenter", "editor"] {
        let mut f = Fixture::new();
        f.share(ShareMode::Link);
        f.add_link_as(role);
        let id = f.upgrade(&f.session(true)).unwrap();
        let (admission, scope) = (OwnerAdmission(f.service.clone()), f.scope());
        // An editor link asks for a text change with an edit request; it does not save one.
        for access in [Access::Save, Access::Publish] {
            assert_eq!(
                admission.authorize(&id, &scope, access),
                Err(crate::sync::Code::Denied),
                "{role}"
            );
        }
    }
}
