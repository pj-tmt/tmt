//! Real private Store and independently frozen predecessor schema; no fake SQL owner.
use super::*;
use crate::{
    state::{Layout, Serving},
    store::{DEFAULT_SCOPES, Grant, Store, uuid_v4},
};
use rusqlite::{Connection, params};
use std::{fs, path::PathBuf};

struct Fixture {
    store: Store,
    serving: Serving,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("tmt-members-{}", uuid_v4().unwrap()));
        let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
        let store = Store::open(&serving).unwrap();
        Self {
            store,
            serving,
            root,
        }
    }
    fn reopen(&mut self) {
        self.store.connection = Connection::open_in_memory().unwrap();
        self.store = Store::open(&self.serving).unwrap();
    }
    fn grant(&mut self, name: &str, talk: bool) -> Grant {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).unwrap();
        let mut g = Grant {
            client_id: uuid_v4().unwrap(),
            public_key: ed25519_dalek::SigningKey::from_bytes(&seed)
                .verifying_key()
                .to_bytes(),
            kind: "cli".into(),
            origin: "cli".into(),
            name: name.into(),
            agents: "all".into(),
            scopes: DEFAULT_SCOPES.map(str::to_owned).into(),
            mode: "hold".into(),
            issued_at_ms: 1,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        };
        if talk {
            g.scopes.push("talk".into());
        }
        self.store.insert_grant(&g).unwrap();
        g
    }
    fn browser(&mut self, name: &str) -> Grant {
        let mut grant = self.grant(name, false);
        grant.kind = "browser".into();
        grant.origin = "http://127.0.0.1:1234".into();
        self.store
            .connection
            .execute(
                "UPDATE grants SET kind=?2,origin=?3 WHERE client_id=?1",
                params![grant.client_id, grant.kind, grant.origin],
            )
            .unwrap();
        grant
    }
    fn request(&self, target: &Grant, candidate: &Grant) -> Association {
        let m = self
            .store
            .member_for_client(&target.client_id)
            .unwrap()
            .unwrap();
        Association {
            operation_id: uuid_v4().unwrap(),
            remote_id: m.remote_id,
            member_id: m.member_id,
            member_revision: m.revision,
            client_id: candidate.client_id.clone(),
            grant_revision: self
                .store
                .grant(&candidate.client_id)
                .unwrap()
                .unwrap()
                .revision,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}
#[test]
fn confirmed_member_defaults_off_direct_and_preserves_narrower_authority() {
    let mut f = Fixture::new();
    let old = f.grant("Old", true);
    let mut candidate = old.clone();
    candidate.client_id = uuid_v4().unwrap();
    candidate.public_key = [7; 32];
    candidate.agents = "[]".into();
    let m = f.store.confirm_member_grant(&candidate).unwrap();
    assert_eq!(m.role, Role::User);
    assert!(!m.sending_enabled);
    let g = f.store.grant(&candidate.client_id).unwrap().unwrap();
    assert_eq!(g.mode, "direct");
    assert_eq!(g.agents, "[]");
    assert!(!g.permits_scope("talk"));
    assert_eq!(f.store.grant(&old.client_id).unwrap().unwrap(), old);
    f.reopen();
    assert_eq!(f.store.member_for_client(&g.client_id).unwrap(), Some(m));
}
#[test]
fn explicit_association_is_atomic_idempotent_and_tombstones_the_old_member() {
    let mut f = Fixture::new();
    let a = f.grant("Same", false);
    let b = f.grant("Same", true);
    let before = f.store.member_for_client(&b.client_id).unwrap().unwrap();
    let req = f.request(&a, &b);
    let out = f
        .store
        .associate_member(Owner::LocalOwner, &req, 10)
        .unwrap();
    assert_eq!(out.member.member_id, a.client_id);
    assert_eq!(out.grant_revision, 2);
    let g = f.store.grant(&b.client_id).unwrap().unwrap();
    assert!(!g.permits_scope("talk"));
    assert_eq!(g.mode, "hold");
    assert_eq!(g.agents, b.agents);
    assert_eq!(g.public_key, b.public_key);
    assert!(
        f.store
            .members()
            .unwrap()
            .iter()
            .find(|m| m.member_id == before.member_id)
            .unwrap()
            .revoked_at_ms
            .is_some()
    );
    let bytes = f.store.members().unwrap();
    f.reopen();
    assert_eq!(
        f.store
            .associate_member(Owner::LocalOwner, &req, 11)
            .unwrap(),
        out
    );
    assert_eq!(f.store.members().unwrap(), bytes);
    let mut changed = req;
    changed.member_id = b.client_id;
    assert_eq!(
        f.store
            .associate_member(Owner::LocalOwner, &changed, 12)
            .unwrap_err()
            .code,
        "REMOTE_ID_CONFLICT"
    );
}
#[test]
fn stale_foreign_expired_revoked_and_user_authority_refuse_without_partial_writes() {
    let mut f = Fixture::new();
    let a = f.grant("A", false);
    let b = f.grant("B", false);
    let req = f.request(&a, &b);
    let before = f.store.members().unwrap();
    assert!(
        f.store
            .associate_member(
                Owner::DesignatedBrowser {
                    client_id: &b.client_id,
                    revision: 1
                },
                &req,
                10
            )
            .is_err()
    );
    for variant in 0..5 {
        let mut r = req.clone();
        match variant {
            0 => r.member_revision += 1,
            1 => r.grant_revision += 1,
            2 => r.member_id = uuid_v4().unwrap(),
            3 => r.client_id = uuid_v4().unwrap(),
            _ => r.remote_id = uuid_v4().unwrap(),
        };
        assert!(f.store.associate_member(Owner::LocalOwner, &r, 10).is_err());
        assert_eq!(f.store.members().unwrap(), before);
        assert_eq!(f.store.grant(&b.client_id).unwrap(), Some(b.clone()));
    }
    f.store
        .connection
        .execute(
            "UPDATE grants SET expires_at_ms=9 WHERE client_id=?1",
            [&b.client_id],
        )
        .unwrap();
    assert!(
        f.store
            .associate_member(Owner::LocalOwner, &req, 10)
            .is_err()
    );
    f.store
        .connection
        .execute(
            "UPDATE grants SET expires_at_ms=NULL WHERE client_id=?1",
            [&b.client_id],
        )
        .unwrap();
    f.store.revoke(&a.client_id).unwrap();
    assert!(
        f.store
            .associate_member(Owner::LocalOwner, &req, 10)
            .is_err()
    );
}
#[test]
fn competing_associations_cannot_steal_the_same_machine() {
    let mut f = Fixture::new();
    let a = f.grant("A", false);
    let b = f.grant("B", false);
    let c = f.grant("C", false);
    let first = f.request(&a, &c);
    let second = f.request(&b, &c);
    let store = std::sync::Mutex::new(&mut f.store);
    let barrier = std::sync::Barrier::new(2);
    let outcomes = std::thread::scope(|scope| {
        let handles: Vec<_> = [first, second]
            .into_iter()
            .map(|request| {
                let store = &store;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    store
                        .lock()
                        .unwrap()
                        .associate_member(Owner::LocalOwner, &request, 10)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(outcomes.iter().filter(|r| r.is_err()).count(), 1);
    let winner = outcomes
        .into_iter()
        .find_map(Result::ok)
        .unwrap()
        .member
        .member_id;
    assert_eq!(
        f.store
            .member_for_client(&c.client_id)
            .unwrap()
            .unwrap()
            .member_id,
        winner
    );
}
#[test]
fn member_policy_fans_out_fences_sessions_and_revoke_queues_routes() {
    let mut f = Fixture::new();
    let a = f.grant("A", false);
    let b = f.grant("B", false);
    let req = f.request(&a, &b);
    let out = f
        .store
        .associate_member(Owner::LocalOwner, &req, 10)
        .unwrap();
    let enabled = f
        .store
        .set_member_sending(
            Owner::LocalOwner,
            &a.client_id,
            out.member.revision,
            true,
            20,
        )
        .unwrap();
    assert_eq!(enabled.clients.len(), 2);
    for id in [&a.client_id, &b.client_id] {
        let g = f.store.grant(id).unwrap().unwrap();
        assert!(g.permits_scope("talk"));
        assert_eq!(g.mode, "hold");
        let sid = uuid_v4().unwrap();
        f.store
            .start_session(&g, &sid, &uuid_v4().unwrap(), 21)
            .unwrap();
        f.store
            .connection
            .execute(
                "INSERT INTO cloud_peer_routes VALUES('demo-members',?1,?1,1,0,0)",
                [id],
            )
            .unwrap();
    }
    let disabled = f
        .store
        .set_member_sending(
            Owner::LocalOwner,
            &a.client_id,
            enabled.member.revision,
            false,
            30,
        )
        .unwrap();
    assert_eq!(disabled.clients.len(), 2);
    let live: i64 = f
        .store
        .connection
        .query_row(
            "SELECT count(*) FROM sessions WHERE ended_reason IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(live, 0);
    let revoked = f
        .store
        .revoke_member(
            Owner::LocalOwner,
            &a.client_id,
            disabled.member.revision,
            40,
        )
        .unwrap();
    assert!(revoked.member.revoked_at_ms.is_some());
    let pending: i64 = f
        .store
        .connection
        .query_row(
            "SELECT count(*) FROM cloud_peer_routes WHERE revoked=1 AND pending=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 2);
    for id in [&a.client_id, &b.client_id] {
        assert!(f.store.grant(id).unwrap().unwrap().disabled);
    }
}
#[test]
fn frozen_schema13_preserves_grants_receipts_designation_and_migrates_once() {
    let mut f = Fixture::new();
    f.store.connection = Connection::open_in_memory().unwrap();
    fs::remove_file(f.serving.layout().directory.join("remote.db")).unwrap();
    f.serving.layout().file("remote.db").unwrap();
    let c = Connection::open(f.serving.layout().directory.join("remote.db")).unwrap();
    c.execute_batch(include_str!("../../tests/fixtures/members/schema13.sql"))
        .unwrap();
    let remote = "10000000-0000-4000-8000-000000000001";
    let a = "20000000-0000-4000-8000-000000000002";
    let b = "client";
    c.execute(
        "INSERT INTO machine VALUES(1,?1,'/r/aaaaaaaaaaaaaaaa')",
        [remote],
    )
    .unwrap();
    for (id, key, kind) in [(a, 1u8, "browser"), (b, 2u8, "cli")] {
        c.execute("INSERT INTO grants VALUES(?1,?2,?3,'https://same.example','Same','[]','agents.read talk','hold',1,999,7,0)",params![id,[key;32].as_slice(),kind]).unwrap();
    }
    c.execute(
        "INSERT INTO settings_designation VALUES(1,?1,?2,?3)",
        params![remote, a, [1u8; 32].as_slice()],
    )
    .unwrap();
    c.execute(
        "INSERT INTO cloud_pair_receipts VALUES('old',?1,5000,x'1234','challenge',?1)",
        [[8u8; 32].as_slice()],
    )
    .unwrap();
    c.execute("INSERT INTO sessions VALUES(?1,'40000000-0000-4000-8000-000000000004','50000000-0000-4000-8000-000000000005',7,'9','15',NULL,NULL,NULL)",[a]).unwrap();
    c.execute("INSERT INTO operations(id,client_id,operation,digest,phase,receipt,updated_ms,grant_revision) VALUES(?1,?1,'dispatch.create',zeroblob(32),'uncertain','{}',1,7)", [a]).unwrap();
    for (project, id) in [("demo-a", a), ("demo-b", b)] {
        c.execute(
            "INSERT INTO cloud_peers VALUES(?1,?2,?3,?4,5,NULL)",
            params![project, id, remote, [9u8; 32].as_slice()],
        )
        .unwrap();
        c.execute(
            "INSERT INTO cloud_peer_routes VALUES(?1,'same-provider-uid',?2,5,0,0)",
            params![project, id],
        )
        .unwrap();
    }
    drop(c);
    f.reopen();
    let members = f.store.members().unwrap();
    assert_eq!(members.len(), 2);
    assert!(members.iter().all(|m| m.member_id == a || m.member_id == b));
    assert_eq!(
        f.store.member_for_client(a).unwrap().unwrap().role,
        Role::Owner
    );
    assert_eq!(
        f.store.member_for_client(b).unwrap().unwrap().role,
        Role::User
    );
    let grants = f.store.grants().unwrap();
    let operation_members: Vec<(String, Option<String>, String)> = f
        .store
        .connection
        .prepare("SELECT client_id,member_id,phase FROM operations ORDER BY client_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        operation_members,
        [(a.into(), Some(a.into()), "uncertain".into())]
    );
    assert!(grants.iter().all(|g| g.revision == 7
        && g.mode == "hold"
        && g.agents == "[]"
        && g.expires_at_ms == Some(999)));
    f.reopen();
    assert_eq!(f.store.members().unwrap(), members);
    assert_eq!(f.store.grants().unwrap(), grants);
    let counters: (String, String) = f
        .store
        .connection
        .query_row(
            "SELECT next_client_sequence,next_server_sequence FROM sessions WHERE client_id=?1",
            [a],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(counters, ("9".into(), "15".into()));
    let pins: i64 = f
        .store
        .connection
        .query_row(
            "SELECT count(*) FROM cloud_peers WHERE generation=5 AND owner_key=?1",
            [[9u8; 32].as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pins, 2);
    let receipt: Vec<u8> = f
        .store
        .connection
        .query_row(
            "SELECT receipt FROM cloud_pair_receipts WHERE offer='old'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(receipt, [0x12, 0x34]);
    let legacy = f.store.member_for_client(b).unwrap().unwrap();
    assert_eq!(legacy.member_id, b);
    let off = f
        .store
        .set_member_sending(Owner::LocalOwner, b, legacy.revision, false, 10)
        .unwrap();
    assert!(!off.member.sending_enabled);
    assert_eq!(off.clients, [b]);
    assert!(!f.store.grant(b).unwrap().unwrap().permits_scope("talk"));
    let revoked = f
        .store
        .revoke_member(Owner::LocalOwner, b, off.member.revision, 11)
        .unwrap();
    assert_eq!(revoked.member.revoked_at_ms, Some(11));
    assert_eq!(revoked.clients, [b]);
    assert!(f.store.grant(b).unwrap().unwrap().disabled);
    let route: (bool, bool) = f
        .store
        .connection
        .query_row(
            "SELECT revoked,pending FROM cloud_peer_routes WHERE client_id=?1",
            [b],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(route, (true, true));
    f.reopen();
    assert_eq!(f.store.member_for_client(b).unwrap(), Some(revoked.member));
    assert!(f.store.grant(b).unwrap().unwrap().disabled);
}

#[test]
fn operation_member_migration_backfills_only_existing_associations() {
    let mut f = Fixture::new();
    f.store.connection = Connection::open_in_memory().unwrap();
    fs::remove_file(f.serving.layout().directory.join("remote.db")).unwrap();
    f.serving.layout().file("remote.db").unwrap();
    let c = Connection::open(f.serving.layout().directory.join("remote.db")).unwrap();
    c.execute_batch(include_str!("../../tests/fixtures/members/schema13.sql"))
        .unwrap();
    c.execute_batch(include_str!(
        "../../tests/fixtures/members/schema14-members.sql"
    ))
    .unwrap();
    let remote = "10000000-0000-4000-8000-000000000001";
    let mapped = "20000000-0000-4000-8000-000000000002";
    let unmapped = "30000000-0000-4000-8000-000000000003";
    c.execute(
        "INSERT INTO machine VALUES(1,?1,'/r/aaaaaaaaaaaaaaaa')",
        [remote],
    )
    .unwrap();
    for (client, key) in [(mapped, 1_u8), (unmapped, 2_u8)] {
        c.execute(
            "INSERT INTO grants VALUES(?1,?2,'cli','cli','Machine','all',?3,'direct',1,NULL,7,0)",
            params![client, [key; 32].as_slice(), DEFAULT_SCOPES.join(" ")],
        )
        .unwrap();
        c.execute("INSERT INTO operations(id,client_id,operation,digest,frozen,phase,receipt,updated_ms,grant_revision) VALUES(?1,?1,'dispatch.create',zeroblob(32),x'1234','uncertain','{}',1,7)", [client]).unwrap();
    }
    c.execute(
        "INSERT INTO members VALUES(?1,?2,'user','Machine',0,1,1,NULL)",
        params![remote, mapped],
    )
    .unwrap();
    c.execute(
        "INSERT INTO member_machines VALUES(?1,?2,?2,1)",
        params![remote, mapped],
    )
    .unwrap();
    drop(c);
    f.reopen();
    let captured: Vec<(String, Option<String>, String, Vec<u8>)> = f
        .store
        .connection
        .prepare("SELECT client_id,member_id,phase,frozen FROM operations ORDER BY client_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        captured,
        [
            (
                mapped.into(),
                Some(mapped.into()),
                "uncertain".into(),
                vec![0x12, 0x34]
            ),
            (unmapped.into(), None, "uncertain".into(), vec![0x12, 0x34])
        ]
    );
    crate::journal::require_operation_member(&f.store.connection, mapped, mapped).unwrap();
    // A later association cannot retroactively bind the unmapped old operation.
    f.store
        .connection
        .execute(
            "INSERT INTO members VALUES(?1,?2,'user','Later',0,1,2,NULL)",
            params![remote, unmapped],
        )
        .unwrap();
    f.store
        .connection
        .execute(
            "INSERT INTO member_machines VALUES(?1,?2,?2,2)",
            params![remote, unmapped],
        )
        .unwrap();
    f.reopen();
    assert!(
        crate::journal::require_operation_member(&f.store.connection, unmapped, unmapped).is_err()
    );
    assert_eq!(
        f.store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM _migrations WHERE name='operation_members'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn new_grants_and_confirmation_reject_noncanonical_ids_without_writing() {
    let mut f = Fixture::new();
    let mut g = f.grant("Existing", true);
    g.client_id = "client".into();
    g.public_key = [19; 32];
    let grants = f.store.grants().unwrap();
    let members = f.store.members().unwrap();
    assert_eq!(
        f.store.insert_grant(&g).unwrap_err().code,
        "REMOTE_STATE_UNAVAILABLE"
    );
    assert_eq!(
        f.store.confirm_member_grant(&g).unwrap_err().code,
        "REMOTE_STATE_UNAVAILABLE"
    );
    assert_eq!(f.store.grants().unwrap(), grants);
    assert_eq!(f.store.members().unwrap(), members);
    assert!(f.store.member_for_client("client").unwrap().is_none());
}

#[test]
fn receipt_save_failure_rolls_back_association_grant_and_tombstone() {
    let mut f = Fixture::new();
    let a = f.grant("A", false);
    let b = f.grant("B", true);
    let req = f.request(&a, &b);
    let before = f.store.members().unwrap();
    f.store.connection.execute_batch("CREATE TRIGGER fail_member_receipt BEFORE INSERT ON management_receipts BEGIN SELECT RAISE(ABORT,'fixture receipt failure'); END").unwrap();
    assert!(
        f.store
            .associate_member(Owner::LocalOwner, &req, 10)
            .is_err()
    );
    assert_eq!(f.store.members().unwrap(), before);
    assert_eq!(f.store.grant(&b.client_id).unwrap(), Some(b.clone()));
    assert_eq!(
        f.store
            .member_for_client(&b.client_id)
            .unwrap()
            .unwrap()
            .member_id,
        b.client_id
    );
    f.store
        .connection
        .execute_batch("DROP TRIGGER fail_member_receipt")
        .unwrap();
    assert!(
        f.store
            .associate_member(Owner::LocalOwner, &req, 10)
            .is_ok()
    );
}

#[test]
fn owner_role_comes_only_from_exact_designation_and_user_cannot_promote_itself() {
    let mut f = Fixture::new();
    let mut a = f.grant("Owner", false);
    f.store
        .connection
        .execute(
            "UPDATE grants SET kind='browser',origin='https://owner.example' WHERE client_id=?1",
            [&a.client_id],
        )
        .unwrap();
    a.kind = "browser".into();
    a.origin = "https://owner.example".into();
    let b = f.grant("Candidate", false);
    let request = f.request(&a, &b);
    assert_eq!(
        f.store
            .member_for_client(&a.client_id)
            .unwrap()
            .unwrap()
            .role,
        Role::User
    );
    assert!(
        f.store
            .associate_member(
                Owner::DesignatedBrowser {
                    client_id: &a.client_id,
                    revision: a.revision
                },
                &request,
                10
            )
            .is_err()
    );
    f.store.designate(&a.client_id, &a.origin, 10).unwrap();
    let request = f.request(&a, &b);
    assert_eq!(
        f.store
            .member_for_client(&a.client_id)
            .unwrap()
            .unwrap()
            .role,
        Role::Owner
    );
    let original = f
        .store
        .associate_member(
            Owner::DesignatedBrowser {
                client_id: &a.client_id,
                revision: a.revision,
            },
            &request,
            10,
        )
        .unwrap();
    let owner: String = f
        .store
        .connection
        .query_row(
            "SELECT client_id FROM management_receipts WHERE id=?1",
            [&request.operation_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(owner, a.client_id);
    let enabled = f
        .store
        .set_member_sending(
            Owner::LocalOwner,
            &a.client_id,
            original.member.revision,
            true,
            11,
        )
        .unwrap();
    a.revision = f.store.grant(&a.client_id).unwrap().unwrap().revision;
    let replay = f
        .store
        .associate_member(
            Owner::DesignatedBrowser {
                client_id: &a.client_id,
                revision: a.revision,
            },
            &request,
            12,
        )
        .unwrap();
    assert_eq!(replay, original);
    assert_eq!(
        f.store.member_for_client(&a.client_id).unwrap().unwrap(),
        enabled.member
    );
    assert!(
        f.store
            .grant(&b.client_id)
            .unwrap()
            .unwrap()
            .permits_scope("talk"),
        "original outcome observation must not reapply old sending-off association"
    );
    f.store.undesignate().unwrap();
    assert_eq!(
        f.store
            .member_for_client(&a.client_id)
            .unwrap()
            .unwrap()
            .role,
        Role::User
    );
    let c = f.grant("Another", false);
    let request = f.request(&a, &c);
    assert!(
        f.store
            .associate_member(
                Owner::DesignatedBrowser {
                    client_id: &a.client_id,
                    revision: a.revision
                },
                &request,
                10
            )
            .is_err()
    );
}

#[test]
fn transaction_local_confirmation_composes_original_receipt_without_nested_or_partial_commit() {
    let mut f = Fixture::new();
    let mut g = f.grant("Seed", false);
    g.client_id = uuid_v4().unwrap();
    g.public_key = [22u8; 32];
    let id = uuid_v4().unwrap();
    {
        let tx = f
            .store
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        super::confirm_member_grant_in(&tx, &g).unwrap();
        // A failed receipt save leaves neither member nor grant after rollback.
        assert!(tx.execute("INSERT INTO management_receipts VALUES(?1,?2,'remote.invite.confirm',?3,1,1,100,'original')",params![id,g.client_id,[7u8;31].as_slice()]).is_err());
    }
    assert!(f.store.grant(&g.client_id).unwrap().is_none());
    assert!(f.store.member_for_client(&g.client_id).unwrap().is_none());
    {
        let tx = f
            .store
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        super::confirm_member_grant_in(&tx, &g).unwrap();
        tx.execute("INSERT INTO management_receipts VALUES(?1,?2,'remote.invite.confirm',?3,1,1,100,'original')",params![id,g.client_id,[7u8;32].as_slice()]).unwrap();
        tx.commit().unwrap();
    }
    f.reopen();
    assert!(f.store.member_for_client(&g.client_id).unwrap().is_some());
    let outcome: String = f
        .store
        .connection
        .query_row(
            "SELECT outcome FROM management_receipts WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(outcome, "original");
}

#[path = "tests/roster.rs"]
mod roster;

#[path = "tests/policy.rs"]
mod policy;

#[path = "tests/selectors.rs"]
mod selectors;

#[path = "tests/terminal.rs"]
mod terminal;

#[path = "tests/terminal_machines.rs"]
mod terminal_machines;
