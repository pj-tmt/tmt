use super::*;
use crate::members::terminal::{
    MemberTerminalMutation as Mutation, MemberTerminalTarget as Target,
};
fn target(f: &Fixture, client: &str) -> Target {
    let m = f.store.member_for_client(client).unwrap().unwrap();
    Target {
        remote_id: m.remote_id,
        member_id: m.member_id,
        member_revision: m.revision,
    }
}
#[test]
fn terminal_member_effects_fence_revisions_and_keep_no_journal() {
    let mut f = Fixture::new();
    let grant = f.browser("Person");
    let original = target(&f, &grant.client_id);
    let (renamed, _) = f
        .store
        .member_terminal_effect(&Mutation::Rename {
            target: original.clone(),
            name: "New person".into(),
        })
        .unwrap();
    assert_eq!(renamed["changed"], true);
    assert_eq!(renamed["member"]["displayName"], "New person");
    assert_eq!(
        f.store.grant(&grant.client_id).unwrap().unwrap().name,
        "Person"
    );
    let current = target(&f, &grant.client_id);
    let same = Mutation::Rename {
        target: current.clone(),
        name: "New person".into(),
    };
    let (same, clients) = f.store.member_terminal_effect(&same).unwrap();
    assert_eq!(same["changed"], false);
    assert!(clients.is_empty());
    assert_eq!(
        target(&f, &grant.client_id).member_revision,
        current.member_revision
    );
    assert_eq!(
        f.store
            .member_terminal_effect(&Mutation::Sending {
                target: original,
                enabled: true
            })
            .unwrap_err()
            .code,
        "REMOTE_MEMBER_CONFLICT"
    );
    let (sending, clients) = f
        .store
        .member_terminal_effect(&Mutation::Sending {
            target: current,
            enabled: true,
        })
        .unwrap();
    assert_eq!(sending["changed"], true);
    assert_eq!(clients, vec![grant.client_id.clone()]);
    let after = f.store.grant(&grant.client_id).unwrap().unwrap();
    assert!(after.permits_scope("talk"));
    assert_eq!(after.mode, grant.mode);
    assert_eq!(after.agents, grant.agents);
    let current = target(&f, &grant.client_id);
    let (revoked, _) = f
        .store
        .member_terminal_effect(&Mutation::Revoke {
            target: current.clone(),
        })
        .unwrap();
    assert_eq!(revoked["changed"], true);
    assert!(
        f.store
            .member_terminal_effect(&Mutation::Revoke { target: current })
            .is_err()
    );
    let current = target(&f, &grant.client_id);
    let (again, clients) = f
        .store
        .member_terminal_effect(&Mutation::Revoke {
            target: current.clone(),
        })
        .unwrap();
    assert_eq!(again["changed"], false);
    assert_eq!(again["alreadyRevoked"], true);
    assert!(clients.is_empty());
    assert_eq!(
        target(&f, &grant.client_id).member_revision,
        current.member_revision
    );
    assert_eq!(
        f.store
            .connection
            .query_row("SELECT COUNT(*) FROM management_receipts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn terminal_same_sending_preserves_a_narrower_machine_scope() {
    let mut f = Fixture::new();
    let g = f.browser("Person");
    let m = f.store.member_for_client(&g.client_id).unwrap().unwrap();
    f.store
        .connection
        .execute(
            "UPDATE members SET sending_enabled=1 WHERE member_id=?1",
            [&m.member_id],
        )
        .unwrap();
    let before = f.store.grant(&g.client_id).unwrap().unwrap();
    let (same, clients) = f
        .store
        .member_terminal_effect(&Mutation::Sending {
            target: target(&f, &g.client_id),
            enabled: true,
        })
        .unwrap();
    assert_eq!(same["changed"], false);
    assert!(clients.is_empty());
    assert_eq!(f.store.grant(&g.client_id).unwrap().unwrap(), before);
    assert!(!before.permits_scope("talk"));
}
#[test]
fn terminal_late_writer_failure_rolls_back_every_machine_and_member() {
    let mut f = Fixture::new();
    let first = f.browser("Person");
    let second = f.browser("Laptop");
    let association = f.request(&first, &second);
    f.store
        .associate_member(Owner::LocalOwner, &association, 10)
        .unwrap();
    let one = f.store.grant(&first.client_id).unwrap().unwrap();
    let two = f.store.grant(&second.client_id).unwrap().unwrap();
    let before = f
        .store
        .member_for_client(&first.client_id)
        .unwrap()
        .unwrap();
    let clients = f.store.member_clients(&before.member_id).unwrap();
    let last = clients.last().unwrap();
    f.store.connection.execute_batch(&format!("CREATE TRIGGER terminal_writer_failure BEFORE UPDATE ON grants WHEN OLD.client_id='{last}' BEGIN SELECT RAISE(ABORT,'injected terminal writer failure'); END;")).unwrap();
    let t = target(&f, &first.client_id);
    assert!(
        f.store
            .member_terminal_effect(&Mutation::Sending {
                target: t,
                enabled: true
            })
            .is_err()
    );
    assert_eq!(f.store.grant(&first.client_id).unwrap().unwrap(), one);
    assert_eq!(f.store.grant(&second.client_id).unwrap().unwrap(), two);
    assert_eq!(
        f.store
            .member_for_client(&first.client_id)
            .unwrap()
            .unwrap(),
        before
    );
    assert_eq!(f.store.connection.query_row("SELECT COUNT(*) FROM management_receipts WHERE operation!='remote.members.associate'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn terminal_writer_admission_expires_without_a_late_store_acquisition() {
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};
    let root = std::env::temp_dir().join(format!("tmt-member-writer-{}", uuid_v4().unwrap()));
    let serving = Layout::open(&root).unwrap().serve_lock().unwrap();
    let store = Arc::new(Mutex::new(Store::open(&serving).unwrap()));
    let devices = Arc::new(crate::devices::Devices::new(Arc::clone(&store), None));
    let held = store.lock().unwrap();
    let worker_devices = Arc::clone(&devices);
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let answer = match worker_devices.terminal_store(Instant::now() + Duration::from_millis(40))
        {
            Ok(_) => "acquired".to_owned(),
            Err(error) => error.code,
        };
        sender.send(answer).unwrap();
    });
    let observed = receiver.recv_timeout(Duration::from_secs(1));
    drop(held);
    worker.join().unwrap();
    let positive = devices
        .terminal_store(Instant::now() + Duration::from_secs(1))
        .is_ok();
    drop(devices);
    drop(store);
    drop(serving);
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(observed.unwrap(), "REMOTE_MEMBER_BUSY");
    assert!(positive);
}
