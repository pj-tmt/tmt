//! The real transaction refuses stale or changed admitted inputs before effects.
use super::durable_fixture::{DurableFixture, admitted};
use super::*;

fn refusals<B: ScenarioBackend>(make: impl Fn(BackendBinding) -> B) {
    let f = DurableFixture::new();
    let stopped = AtomicBool::new(false);
    let io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: &stopped,
    };
    let (_, mut delivery, snapshot) =
        f.pending(make(f.context.binding.clone()), f.sink.clone(), &io);
    let (verified, snapshot, grant, key) = admitted(&delivery, &snapshot.candidate);
    let owner = OwnerActor::local_interactive();
    let mut changed = snapshot.clone();
    changed.digest[0] ^= 1;
    assert!(
        f.sink
            .confirm(Confirmation {
                verified: &verified,
                snapshot: &changed,
                actor: &owner,
                grant: &grant,
                receipt_key: &key,
                sending: false,
                io: &io
            })
            .is_err(),
        "writer accepted a changed frozen snapshot"
    );
    assert_eq!(f.counts(), (0, 0, 0, 0));
    assert!(
        f.sink
            .confirm(Confirmation {
                verified: &verified,
                snapshot: &snapshot,
                actor: &OwnerActor::automated(),
                grant: &grant,
                receipt_key: &key,
                sending: false,
                io: &io
            })
            .is_err(),
        "agent manufactured consent"
    );
    let mut requested = grant.clone();
    requested.scopes.push("talk".into());
    assert!(
        f.sink
            .confirm(Confirmation {
                verified: &verified,
                snapshot: &snapshot,
                actor: &owner,
                grant: &requested,
                receipt_key: &key,
                sending: false,
                io: &io
            })
            .is_err()
    );
    assert_eq!(f.counts(), (0, 0, 0, 0));
    stopped.store(true, Ordering::Release);
    assert!(
        f.sink
            .confirm(Confirmation {
                verified: &verified,
                snapshot: &snapshot,
                actor: &owner,
                grant: &grant,
                receipt_key: &key,
                sending: false,
                io: &io
            })
            .is_err()
    );
    stopped.store(false, Ordering::Release);
    // Valid native proof checked against an old caller clock must still fail
    // the real writer's clock. No sleeps or provider effects are needed.
    delivery.context.issued_at_ms = 1;
    delivery.context.expires_at_ms = 600_001;
    let (expired, snapshot, grant, key) = admitted(&delivery, &snapshot.candidate);
    assert!(
        f.sink
            .confirm(Confirmation {
                verified: &expired,
                snapshot: &snapshot,
                actor: &owner,
                grant: &grant,
                receipt_key: &key,
                sending: false,
                io: &io
            })
            .is_err(),
        "writer accepted an expired proof from a stale caller clock"
    );
    f.reopen();
    assert_eq!(f.counts(), (0, 0, 0, 0));
}

#[test]
fn mailbox_real_writer_refuses_changed_candidate_expiry_and_automation() {
    refusals(Mailbox::new);
}
#[test]
fn append_log_real_writer_refuses_changed_candidate_expiry_and_automation() {
    refusals(AppendLog::new);
}
