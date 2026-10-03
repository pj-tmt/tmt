use super::test_support::*;
use super::*;
use crate::{
    registration::{OwnerAdmission, Registration},
    sync::{Access, Admission},
    transitions::{OwnerAction, ShareMode},
};
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
    let mut restarted = Registration::new(
        Store::open(&layout).unwrap(),
        Keyring::read(&layout).unwrap(),
        std::env::current_exe().unwrap(),
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
