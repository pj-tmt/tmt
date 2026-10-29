//! Product provider with fake ordinary-user ports; no account, network or Core process.
use super::*;
use crate::{
    firebase_auth::{
        AuthError, Cancellation, Clock, Http, Jitter, Operation, Request, Response, Time,
    },
    firestore_budget::{Decision, Model, NativeUsage, Refusal},
    native_sync::{NativeBudgetPort, NativeRuntime},
    objects::IoBudget,
    sealed_sync::{AuthFailure, AuthIdentity, AuthPort, AuthToken, SyncError},
};
use std::{
    future::Future,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

struct Auth {
    now: u64,
}
impl AuthPort for Auth {
    fn identity(&self) -> AuthIdentity {
        AuthIdentity {
            project: "demo-summary".into(),
            uid: "private-uid".into(),
        }
    }
    fn token<'a>(
        &'a self,
        _: &'a IoBudget<'a>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<AuthToken, AuthFailure>> + Send + 'a>> {
        Box::pin(async {
            Ok(AuthToken {
                identity: self.identity(),
                token: "fake-ordinary-user".into(),
                expires_at_ms: self.now + 3_600_000,
            })
        })
    }
}
struct Ports {
    now: AtomicU64,
    document: Mutex<Option<serde_json::Value>>,
    calls: Mutex<Vec<(Operation, String)>>,
    hide_next_read: AtomicBool,
}
impl Clock for Ports {
    fn now(&self) -> Time {
        let now = self.now.load(Ordering::Acquire);
        Time {
            wall_ms: now,
            monotonic_ms: now,
        }
    }
    fn wait_until(&self, _: u64, _: &dyn Cancellation) -> bool {
        true
    }
}
impl Jitter for Ports {
    fn milliseconds(&self, _: u64) -> u64 {
        0
    }
}
impl Http for Ports {
    fn send(&self, request: &Request, _: u64, _: &dyn Cancellation) -> Result<Response, AuthError> {
        self.calls
            .lock()
            .unwrap()
            .push((request.operation(), request.url().into()));
        let (status, body) = match request.operation() {
            Operation::Commit => {
                let request: serde_json::Value = serde_json::from_slice(request.body()).unwrap();
                assert_eq!(request["writes"].as_array().unwrap().len(), 1);
                if let Some(name)=request["writes"][0]["delete"].as_str(){
                    assert!(name.contains("/inbox/"),"summary cleanup must not issue an unused response delete");
                    *self.document.lock().unwrap()=None;
                    return Ok(Response {status:200,body:b"{}".to_vec(),retry_after_ms:None});
                }
                let update = &request["writes"][0]["update"];
                let mut document = update.clone();
                let stamp = crate::cloud_inbox::stamp(self.now.load(Ordering::Acquire));
                document["createTime"] = json!(stamp);
                document["fields"]["createdAt"] = json!({"timestampValue":stamp});
                let text = serde_json::to_string(&document).unwrap();
                assert!(!text.contains("Studio"));
                assert!(!text.contains("unavailable"));
                assert!(
                    update["name"]
                        .as_str()
                        .unwrap()
                        .contains(&format!("/m/{PEER}/inbox/"))
                );
                *self.document.lock().unwrap() = Some(document);
                (200, b"{}".to_vec())
            }
            Operation::Read => {
                if self.hide_next_read.swap(false, Ordering::AcqRel) {
                    (404, b"{}".to_vec())
                } else {
                    let document = self.document.lock().unwrap();
                    match document.as_ref() {
                        Some(document) => {
                            assert!(request.url().ends_with(document["name"].as_str().unwrap()));
                            (200, serde_json::to_vec(document).unwrap())
                        }
                        None => (404, b"{}".to_vec()),
                    }
                }
            }
            Operation::Query => (200,serde_json::to_vec(&json!([{"readTime":crate::cloud_inbox::stamp(self.now.load(Ordering::Acquire))}])).unwrap()),
            other => panic!("unexpected provider operation {other:?}"),
        };
        Ok(Response {
            status,
            body,
            retry_after_ms: None,
        })
    }
}
struct Budget {
    refuse: AtomicBool,
    costs: Mutex<Vec<u64>>,
    now: u64,
}
impl NativeBudgetPort for Budget {
    fn usage(&self, _: &str) -> Result<NativeUsage, SyncError> {
        Ok(NativeUsage::new(self.now))
    }
    fn charge(&self, _: &str, _: &Model, cost: u64, time: Time) -> Result<Decision, SyncError> {
        self.costs.lock().unwrap().push(cost);
        Ok(if self.refuse.load(Ordering::Acquire) {
            Decision::Refuse(Refusal {
                reset_at_ms: time.wall_ms + 1000,
                retry_after_ms: 1000,
            })
        } else {
            Decision::Allow
        })
    }
    fn flush(&self) -> Result<(), SyncError> {
        Ok(())
    }
}
#[test]
fn budget_preserves_unsent_intent_and_lost_ack_restart_only_observes_original() {
    private_child(
        "squads::store::tests::io_tests::budget_preserves_unsent_intent_and_lost_ack_restart_only_observes_original",
        |root| {
            let mut w = World::new(root);
            let remote =
                Layout::open(&w.layout.directory.parent().unwrap().join("recipient")).unwrap();
            use std::io::Write;
            remote
                .file("machine.key")
                .unwrap()
                .write_all(&[8; 32])
                .unwrap();
            let remote_key = crate::state::MachineKey::open(&remote).unwrap();
            let keys = crate::cloud_keys::CloudKeys::new(remote, PEER.into(), remote_key);
            keys.maintain(w.now).unwrap();
            let descriptor = keys.advertised(w.now).unwrap();
            let next = Prepared::for_peer(
                &Summary::unavailable("Studio", w.now).unwrap(),
                &w.grant.authority().unwrap(),
            )
            .unwrap();
            w.store
                .lock()
                .unwrap()
                .summary_observed(&w.grant, &next, w.now)
                .unwrap();
            let pending = w
                .store
                .lock()
                .unwrap()
                .summary_pending(&w.grant, &next, w.now)
                .unwrap();
            let ports = Arc::new(Ports {
                now: AtomicU64::new(w.now),
                document: Mutex::new(None),
                calls: Mutex::new(vec![]),
                hide_next_read: AtomicBool::new(true),
            });
            let budget = Arc::new(Budget {
                refuse: AtomicBool::new(true),
                costs: Mutex::new(vec![]),
                now: w.now,
            });
            let runtime = Arc::new(NativeRuntime::new(
                ports.clone(),
                ports.clone(),
                ports.clone(),
                budget.clone(),
            ));
            let stop = AtomicBool::new(false);
            let io = IoBudget {
                deadline: Instant::now() + Duration::from_secs(5),
                cancelled: &stop,
            };
            let sender = crate::cloud_inbox::Sender::new(
                crate::cloud_inbox::Provider::new(
                    Arc::new(Auth { now: w.now }),
                    runtime.clone(),
                    Model::new(4, 4).unwrap(),
                    PEER,
                )
                .unwrap(),
                w.store.clone(),
                &w.machine,
                crate::state::MachineKey::open(&w.layout).unwrap(),
            )
            .unwrap();
            let call = |sender: &crate::cloud_inbox::Sender| {
                crate::native_sync::complete(
                    sender.push_summary(&pending, &w.grant, &descriptor, w.now, &io),
                    &io,
                )
                .unwrap()
            };
            assert!(call(&sender).is_err());
            assert!(ports.calls.lock().unwrap().is_empty());
            assert!(
                !w.store
                    .lock()
                    .unwrap()
                    .cloud_one_way_outgoing(&w.machine, &pending.id, &pending.payload)
                    .unwrap()
                    .unwrap()
                    .attempted
            );
            budget.refuse.store(false, Ordering::Release);
            ports.now.store(w.now + 1001, Ordering::Release);
            // New guard admission resumes the exact intent. Only this never-attempted
            // carrier may be resealed; its denied preparation performed no HTTP.
            assert!(call(&sender).is_err());
            assert_eq!(
                ports
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(op, _)| *op == Operation::Commit)
                    .count(),
                1
            );
            drop(sender);
            drop(w.store);
            w.store = Arc::new(Mutex::new(Store::open(&w.lease).unwrap()));
            let restored = w
                .store
                .lock()
                .unwrap()
                .summary_pending(&w.grant, &next, w.now + 1001)
                .unwrap();
            assert_eq!(restored.id, pending.id);
            let sender = crate::cloud_inbox::Sender::new(
                crate::cloud_inbox::Provider::new(
                    Arc::new(Auth { now: w.now }),
                    runtime,
                    Model::new(4, 4).unwrap(),
                    PEER,
                )
                .unwrap(),
                w.store.clone(),
                &w.machine,
                crate::state::MachineKey::open(&w.layout).unwrap(),
            )
            .unwrap();
            crate::native_sync::complete(
                sender.push_summary(&restored, &w.grant, &descriptor, w.now + 1001, &io),
                &io,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                ports
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(op, _)| *op == Operation::Commit)
                    .count(),
                1
            );
            assert_eq!(
                ports
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(op, _)| *op == Operation::Read)
                    .count(),
                2
            );
            assert!(budget.costs.lock().unwrap().len() >= 4);
            w.store
                .lock()
                .unwrap()
                .summary_published(&w.grant, &restored.id, &restored.payload, w.now + 1001)
                .unwrap();
            // A later attempted snapshot can be absent even though its original
            // HTTP outcome was unknown. Observe that ID, never recreate it, then
            // retain fresh desired state behind the five-minute metadata hold.
            let later = w.now + crate::limits::SQUAD_SUMMARY_HEARTBEAT_MS + 1001;
            ports.now.store(later, Ordering::Release);
            let heartbeat = Prepared::for_peer(
                &Summary::unavailable("Studio", later).unwrap(),
                &w.grant.authority().unwrap(),
            )
            .unwrap();
            w.store
                .lock()
                .unwrap()
                .summary_observed(&w.grant, &heartbeat, later)
                .unwrap();
            let absent = w
                .store
                .lock()
                .unwrap()
                .summary_pending(&w.grant, &heartbeat, later)
                .unwrap();
            ports.hide_next_read.store(true, Ordering::Release);
            assert!(
                crate::native_sync::complete(
                    sender.push_summary(&absent, &w.grant, &descriptor, later, &io),
                    &io
                )
                .unwrap()
                .is_err()
            );
            *ports.document.lock().unwrap() = None;
            assert_eq!(
                crate::native_sync::complete(
                    sender.push_summary(&absent, &w.grant, &descriptor, later, &io),
                    &io
                )
                .unwrap()
                .unwrap(),
                crate::squads::publisher::Publication::Unobserved
            );
            w.store
                .lock()
                .unwrap()
                .summary_unobserved(&w.grant, &absent.id, later)
                .unwrap();
            let (_, mut newest) = frame(&w.payload(1, "Newest")).unwrap();
            newest.observed_at = later + crate::limits::SQUAD_SUMMARY_HEARTBEAT_MS;
            let newest = Prepared::for_peer(&newest, &w.grant.authority().unwrap()).unwrap();
            w.store
                .lock()
                .unwrap()
                .summary_observed(&w.grant, &newest, later + 1)
                .unwrap();
            let mut schedule = w
                .store
                .lock()
                .unwrap()
                .summary_schedule(PEER, later + 1, 1)
                .unwrap()
                .unwrap();
            schedule.observe(newest.clone(), 1);
            assert!(
                schedule
                    .due(crate::limits::SQUAD_SUMMARY_HEARTBEAT_MS - 1)
                    .is_none()
            );
            assert!(
                schedule
                    .due(crate::limits::SQUAD_SUMMARY_HEARTBEAT_MS)
                    .is_some()
            );
            let fresh = w
                .store
                .lock()
                .unwrap()
                .summary_pending(
                    &w.grant,
                    &newest,
                    later + crate::limits::SQUAD_SUMMARY_HEARTBEAT_MS,
                )
                .unwrap();
            assert_ne!(fresh.id, absent.id);
            assert!(frame(&fresh.payload).unwrap().0 > frame(&absent.payload).unwrap().0);
            assert!(
                w.store
                    .lock()
                    .unwrap()
                    .cloud_one_way_outgoing(&w.machine, &absent.id, &absent.payload)
                    .unwrap()
                    .unwrap()
                    .attempted
            );
            assert_eq!(
                ports
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(op, _)| *op == Operation::Commit)
                    .count(),
                2
            );
            assert_eq!(
                ports
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(op, _)| *op == Operation::Read)
                    .count(),
                4
            );
            let count: i64 = w
                .store
                .lock()
                .unwrap()
                .connection
                .query_row("SELECT COUNT(*) FROM operations", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "metadata never enters the Core dispatch ledger");
        },
    );
}

#[test]
fn guarded_summary_cleanup_keeps_pending_rows_until_one_delete_succeeds() {
    private_child(
        "squads::store::tests::io_tests::guarded_summary_cleanup_keeps_pending_rows_until_one_delete_succeeds",
        |root| {
            let w = World::new(root);
            let (receiver, keys, sessions) = w.receiver_owner();
            let document = w.document(&keys, &w.payload(1, "Cached"));
            let reply = receiver.receive(&document, w.now).unwrap().unwrap();
            assert!(!reply.response_required());
            drop(reply);
            let ports = Arc::new(Ports {
                now: AtomicU64::new(w.now),
                document: Mutex::new(None),
                calls: Mutex::new(vec![]),
                hide_next_read: AtomicBool::new(false),
            });
            let budget = Arc::new(Budget {
                refuse: AtomicBool::new(true),
                costs: Mutex::new(vec![]),
                now: w.now,
            });
            let runtime = Arc::new(NativeRuntime::new(
                ports.clone(),
                ports.clone(),
                ports.clone(),
                budget.clone(),
            ));
            let provider = crate::cloud_inbox::Provider::new(
                Arc::new(Auth { now: w.now }),
                runtime,
                Model::new(4, 4).unwrap(),
                &w.machine,
            )
            .unwrap();
            let driver = crate::cloud_inbox::InboxDriver::new(receiver, sessions, provider);
            let stop = AtomicBool::new(false);
            let io = IoBudget {
                deadline: Instant::now() + Duration::from_secs(5),
                cancelled: &stop,
            };
            assert!(
                crate::native_sync::complete(driver.reconcile(w.now, &io), &io)
                    .unwrap()
                    .is_err()
            );
            assert!(
                ports.calls.lock().unwrap().is_empty(),
                "guard refusal precedes all cleanup HTTP"
            );
            assert_eq!(
                w.store.lock().unwrap().cloud_expired(w.now).unwrap().len(),
                1
            );
            assert_eq!(
                w.store.lock().unwrap().read().unwrap().len(),
                1,
                "cache remains verified during refusal"
            );
            budget.refuse.store(false, Ordering::Release);
            ports.now.store(w.now + 1001, Ordering::Release);
            crate::native_sync::complete(driver.reconcile(w.now + 1001, &io), &io)
                .unwrap()
                .unwrap();
            assert!(
                w.store
                    .lock()
                    .unwrap()
                    .cloud_expired(w.now + 1001)
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                ports
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(op, _)| *op == Operation::Commit)
                    .count(),
                1,
                "one summary delete, no response document"
            );
            assert_eq!(
                ports
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(op, _)| *op == Operation::Query)
                    .count(),
                1
            );
            assert!(
                budget.costs.lock().unwrap().len() >= 3,
                "refused delete, admitted delete and query share the guard"
            );
            let cached = w.store.lock().unwrap().read().unwrap();
            assert_eq!(
                Summary::read(&cached[0].summary).unwrap().machine_name,
                "Cached"
            );
        },
    );
}
