//! Backend additions and explicit sending consent share the original journal's
//! real Store transaction; they cannot replace the persisted policy in a receipt.
use super::durable_fixture::{DurableFixture, admitted};
use super::*;
use crate::invitations::store::ConfirmationExtras;

#[test]
fn explicit_sending_and_backend_additions_commit_with_the_original_receipt() {
    for enabled in [false, true] {
        let f = DurableFixture::new();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &stopped,
        };
        let backend = Mailbox::new(f.context.binding.clone());
        let (_, delivery, pending) = f.pending(backend, f.sink.clone(), &io);
        let (verified, snapshot, grant, key) = admitted(&delivery, &pending.candidate);
        let actor = OwnerActor::local_interactive();
        let input = || Confirmation {
            verified: &verified,
            snapshot: &snapshot,
            actor: &actor,
            grant: &grant,
            receipt_key: &key,
            sending: enabled,
            io: &io,
        };
        let original = f
            .sink
            .confirm_with(input(), |tx, _, _| {
                tx.execute(
                    "INSERT INTO cloud_owner_projects VALUES('demo-invitation-transaction','{}')",
                    [],
                )
                .map_err(crate::store::database)?;
                Ok(ConfirmationExtras {
                    receipt_fields: serde_json::from_value(
                        serde_json::json!({"backendProof":"captured-capability"}),
                    )
                    .unwrap(),
                    routes: Some(b"captured route publication".to_vec()),
                })
            })
            .unwrap();
        let persisted = f
            .store
            .lock()
            .unwrap()
            .grant(&grant.client_id)
            .unwrap()
            .unwrap();
        assert_eq!(persisted.permits_scope("talk"), enabled);
        assert_eq!(original.grant_revision, persisted.revision);
        let receipt: serde_json::Value = serde_json::from_slice(&original.receipt).unwrap();
        let expected: serde_json::Value = serde_json::from_slice(
            &crate::pairing::receipt(
                &persisted,
                &snapshot.context.owner,
                &snapshot.context.owner_key,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(receipt["grant"], expected["grant"]);
        assert_eq!(receipt["backendProof"], "captured-capability");
        assert_eq!(original.proof, key.prove(&original.receipt).unwrap());
        assert_eq!(
            original.routes.as_deref(),
            Some(b"captured route publication".as_slice())
        );
        f.reopen();
        assert!(f.sink.original(&snapshot).unwrap().as_ref() == Some(&original));
        f.store
            .lock()
            .unwrap()
            .talk(&grant.client_id, !enabled)
            .unwrap();
        let replay = f
            .sink
            .confirm_with(input(), |_, _, _| {
                panic!("original replay repeated backend effects")
            })
            .unwrap();
        assert!(replay == original);
        let current = f
            .store
            .lock()
            .unwrap()
            .grant(&grant.client_id)
            .unwrap()
            .unwrap();
        assert_eq!(current.permits_scope("talk"), !enabled);
        assert_eq!(f.counts(), (1, 1, 1, 1));
    }
}

#[test]
fn failed_backend_extension_or_reserved_receipt_field_rolls_back_every_effect() {
    for replace_policy in [false, true] {
        let f = DurableFixture::new();
        let stopped = AtomicBool::new(false);
        let io = IoBudget {
            deadline: Instant::now() + Duration::from_secs(30),
            cancelled: &stopped,
        };
        let (_, delivery, pending) =
            f.pending(Mailbox::new(f.context.binding.clone()), f.sink.clone(), &io);
        let (verified, snapshot, grant, key) = admitted(&delivery, &pending.candidate);
        let error = f.sink.confirm_with(
            Confirmation {
                verified: &verified,
                snapshot: &snapshot,
                actor: &OwnerActor::local_interactive(),
                grant: &grant,
                receipt_key: &key,
                sending: true,
                io: &io,
            },
            |tx, _, _| {
                tx.execute(
                    "INSERT INTO cloud_owner_projects VALUES('demo-invitation-transaction','{}')",
                    [],
                )
                .map_err(crate::store::database)?;
                if !replace_policy {
                    return Err(invalid());
                }
                Ok(ConfirmationExtras {
                    receipt_fields: serde_json::from_value(
                        serde_json::json!({"grant":{"scopes":["talk"]}}),
                    )
                    .unwrap(),
                    routes: None,
                })
            },
        );
        assert!(matches!(error, Err(ConfirmationFailure::Refused(_))));
        f.reopen();
        assert_eq!(f.counts(), (0, 0, 0, 0));
        let projects = f
            .store
            .lock()
            .unwrap()
            .connection
            .query_row("SELECT COUNT(*) FROM cloud_owner_projects", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(projects, 0);
        assert!(f.sink.original(&snapshot).unwrap().is_none());
    }
}

#[test]
fn coordinator_carries_only_separate_explicit_owner_sending_consent() {
    let f = DurableFixture::new();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (coordinator, _, snapshot) =
        f.pending(Mailbox::new(f.context.binding.clone()), f.sink.clone(), &io);
    coordinator
        .confirm_owner_decision(
            &OwnerActor::local_interactive(),
            &snapshot,
            OwnerDecision {
                matched: true,
                enable_sending: true,
            },
            &PairingPolicy::default(),
            crate::pairing::now_ms().unwrap(),
            &io,
        )
        .unwrap();
    let receipt = f.sink.original(&snapshot).unwrap().unwrap();
    let grant = f
        .store
        .lock()
        .unwrap()
        .grant(&snapshot.candidate.machine)
        .unwrap()
        .unwrap();
    assert!(grant.permits_scope("talk"));
    assert_eq!(
        receipt.receipt,
        crate::pairing::receipt(&grant, &snapshot.context.owner, &snapshot.context.owner_key)
            .unwrap()
    );
}
