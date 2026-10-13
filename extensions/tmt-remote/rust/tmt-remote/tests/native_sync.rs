//! Native REST effects over frozen extension mappings and injected ordinary Auth.
use serde_json::{Value, json};
use sha2::Digest;
use std::{
    collections::BTreeMap,
    future::Future,
    pin::pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tmt_remote::{
    firebase_auth::{
        AuthError, Cancellation, Clock, Http, Jitter, Operation, Request, Response, Time,
    },
    firestore_budget::{Decision, Model, NativeUsage, charge_native},
    native_sync::{NativeBudgetPort, NativeClient, NativeRuntime},
    objects::IoBudget,
    sealed_sync::{
        AuthFailure, AuthIdentity, AuthPort, AuthToken, Change, DiscoveryContinuation,
        DiscoveryQuery, Document, Mutation, Published, Query, SealedSyncPort, SyncContext,
        SyncCursor, SyncError, SyncQueryPage, SyncQueryRequest, Target,
    },
};
const NOW: u64 = 1_800_000_000_000;
struct MemoryNativeBudget(Mutex<NativeUsage>);
impl MemoryNativeBudget {
    fn new(now: u64) -> Self {
        Self(Mutex::new(NativeUsage::new(now)))
    }
    fn reads(&self) -> u64 {
        self.0.lock().unwrap().reads
    }
    fn set_reads(&self, reads: u64) {
        self.0.lock().unwrap().reads = reads;
    }
}
impl NativeBudgetPort for MemoryNativeBudget {
    fn usage(&self, project: &str) -> Result<NativeUsage, SyncError> {
        assert_eq!(project, "demo-remote-1");
        Ok(*self.0.lock().unwrap())
    }
    fn charge(
        &self,
        project: &str,
        model: &Model,
        maximum_reads: u64,
        time: Time,
    ) -> Result<Decision, SyncError> {
        assert_eq!(project, "demo-remote-1");
        charge_native(
            model,
            &mut self.0.lock().unwrap(),
            maximum_reads,
            time.wall_ms,
        )
        .map_err(|_| SyncError::Unavailable)
    }
    fn flush(&self) -> Result<(), SyncError> {
        Ok(())
    }
}
fn ready<T>(f: impl Future<Output = T>) -> T {
    let mut f = pin!(f);
    let w = std::task::Waker::noop();
    match f.as_mut().poll(&mut Context::from_waker(w)) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("injected calls must complete without an executor"),
    }
}
struct Auth {
    calls: AtomicUsize,
    uid: String,
    expired: bool,
    blocked: AtomicBool,
}
impl AuthPort for Auth {
    fn identity(&self) -> AuthIdentity {
        AuthIdentity {
            project: "demo-remote-1".into(),
            uid: "fixture-user".into(),
        }
    }
    fn token<'a>(
        &'a self,
        io: &'a IoBudget<'a>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<AuthToken, AuthFailure>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Box::pin(async move {
            std::future::poll_fn(|_| {
                if self.blocked.load(Ordering::SeqCst) {
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
            .await;
            assert!(io.check().is_ok());
            Ok(AuthToken {
                identity: AuthIdentity {
                    project: "demo-remote-1".into(),
                    uid: self.uid.clone(),
                },
                token: "id-token-canary".into(),
                expires_at_ms: if self.expired { NOW } else { NOW + 3_600_000 },
            })
        })
    }
}
struct WorldClock(AtomicUsize);
impl Clock for WorldClock {
    fn now(&self) -> Time {
        Time {
            wall_ms: NOW + self.0.load(Ordering::Relaxed) as u64,
            monotonic_ms: self.0.load(Ordering::Relaxed) as u64,
        }
    }
    fn wait_until(&self, deadline: u64, cancel: &dyn Cancellation) -> bool {
        if cancel.cancelled() {
            return false;
        }
        self.0.store(deadline as usize, Ordering::Relaxed);
        true
    }
}
struct NoJitter;
impl Jitter for NoJitter {
    fn milliseconds(&self, _: u64) -> u64 {
        0
    }
}
struct Script {
    replies: Mutex<std::collections::VecDeque<Result<Response, AuthError>>>,
    deadlines: Mutex<Vec<u64>>,
    requests: Mutex<Vec<(Operation, String, Value)>>,
    cancel: Option<Arc<AtomicBool>>,
}
impl Script {
    fn new(replies: Vec<Result<Response, AuthError>>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            deadlines: Mutex::new(vec![]),
            requests: Mutex::new(vec![]),
            cancel: None,
        }
    }
}
impl Http for Script {
    fn send(
        &self,
        request: &Request,
        deadline: u64,
        _: &dyn Cancellation,
    ) -> Result<Response, AuthError> {
        self.deadlines.lock().unwrap().push(deadline);
        assert_eq!(request.bearer(), Some("id-token-canary"));
        assert!(!format!("{request:?}").contains("id-token-canary"));
        self.requests.lock().unwrap().push((
            request.operation(),
            request.url().into(),
            if request.body().is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(request.body()).unwrap()
            },
        ));
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::SeqCst);
        }
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected provider effect")
    }
}
fn response(status: u16, body: Value) -> Response {
    Response {
        status,
        body: body.to_string().into_bytes(),
        retry_after_ms: None,
    }
}
fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/sealed_sync/codec-v1.json")).unwrap()
}
fn fields(seq: u64) -> Value {
    let mut fields: Value = serde_json::from_str(fixture()["document"].as_str().unwrap()).unwrap();
    fields["counter"] = json!({"integerValue":seq.to_string()});
    fields
}
fn remote_document(seq: u64) -> Value {
    json!({"name":format!("projects/demo-remote-1/databases/(default)/documents/x/notes/rows/writer-stream+{seq}"),"fields":fields(seq)})
}
fn doc() -> Document {
    let v = fixture();
    Document {
        target: Target {
            resource: "rows".into(),
            id: "writer-stream+1".into(),
        },
        bytes: v["document"].as_str().unwrap().as_bytes().to_vec(),
        digest: v["documentDigest"].as_str().unwrap().into(),
    }
}
fn stream(seq: u64) -> Query {
    Query {
        resource: "rows".into(),
        scope: BTreeMap::from([
            ("tenant".into(), json!({"stringValue":"fixture-space"})),
            ("streamKey".into(), json!({"stringValue":"writer-stream"})),
        ]),
        stream: "writer-stream".into(),
        cursor: Some(SyncCursor {
            stream: "writer-stream".into(),
            seq,
        }),
    }
}
fn query(seq: u64) -> SyncQueryRequest {
    SyncQueryRequest::Stream(stream(seq))
}
fn discovery(continuation: Option<DiscoveryContinuation>) -> SyncQueryRequest {
    SyncQueryRequest::Discovery(DiscoveryQuery {
        resource: "rows".into(),
        shape: "arrivals".into(),
        scope: BTreeMap::from([("tenant".into(), json!({"stringValue":"fixture-space"}))]),
        continuation,
    })
}
fn native_client(script: Arc<Script>, auth: Arc<Auth>) -> (NativeClient, Arc<MemoryNativeBudget>) {
    let (client, budget, _) = native_with_clock(script, auth);
    (client, budget)
}
fn native_with_clock(
    script: Arc<Script>,
    auth: Arc<Auth>,
) -> (NativeClient, Arc<MemoryNativeBudget>, Arc<WorldClock>) {
    let v = fixture();
    native_with_declaration(script, auth, v["declaration"].as_str().unwrap().as_bytes())
}
fn native_with_declaration(
    script: Arc<Script>,
    auth: Arc<Auth>,
    declaration: &[u8],
) -> (NativeClient, Arc<MemoryNativeBudget>, Arc<WorldClock>) {
    let context=SyncContext::bind(json!({"apiKey":"public","appId":"1:123:web:fixture","projectId":"demo-remote-1","authDomain":"demo-remote-1.firebaseapp.com"}),declaration,&sha2::Sha256::digest(declaration).iter().map(|b| format!("{b:02x}")).collect::<String>(),"fixture-space",auth).unwrap();
    let budget = Arc::new(MemoryNativeBudget::new(NOW));
    let clock = Arc::new(WorldClock(AtomicUsize::new(0)));
    let runtime = Arc::new(NativeRuntime::new(
        script,
        clock.clone(),
        Arc::new(NoJitter),
        budget.clone(),
    ));
    (
        NativeClient::new(context, runtime, Model::new(1, 1).unwrap()),
        budget,
        clock,
    )
}
fn mutable_client(script: Arc<Script>, auth: Arc<Auth>) -> (NativeClient, Arc<MemoryNativeBudget>) {
    let mut declaration: Value =
        serde_json::from_str(fixture()["declaration"].as_str().unwrap()).unwrap();
    // A synthetic owner-mutable checkpoint uses the frozen field mapping; it
    // grants no admission and never changes the immutable log declaration.
    declaration["resources"][0]["kind"] = json!("checkpoint");
    declaration["resources"][0]["sync"]["writeModes"] = json!(["create", "put", "delete"]);
    declaration.sort_all_objects();
    let (client, budget, _) =
        native_with_declaration(script, auth, &serde_json::to_vec(&declaration).unwrap());
    (client, budget)
}
fn auth() -> Arc<Auth> {
    Arc::new(Auth {
        calls: AtomicUsize::new(0),
        uid: "fixture-user".into(),
        expired: false,
        blocked: AtomicBool::new(false),
    })
}
fn io(cancel: &AtomicBool) -> IoBudget<'_> {
    IoBudget {
        deadline: Instant::now() + Duration::from_secs(30),
        cancelled: cancel,
    }
}

#[test]
fn read_uses_the_same_frozen_bytes_digest_and_declared_target() {
    let script = Arc::new(Script::new(vec![Ok(response(200, remote_document(1)))]));
    let (client, budget) = native_client(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    let document = ready(client.read(&doc().target, &io)).unwrap().unwrap();
    assert_eq!(document, doc());
    assert_eq!(script.requests.lock().unwrap()[0].0, Operation::Read);
    assert_eq!(budget.reads(), 3);
}
#[test]
fn query_uses_exact_scope_and_contiguous_sequence_not_lexical_ids() {
    let script = Arc::new(Script::new(vec![Ok(response(
        200,
        json!([{"document":remote_document(2)}]),
    ))]));
    let (client, budget) = native_client(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    let SyncQueryPage::Stream(page) = ready(client.query(&query(1), &io)).unwrap() else {
        panic!("wrong page shape")
    };
    assert_eq!(page.documents.len(), 1);
    assert_eq!(page.cursor.unwrap().seq, 2);
    assert_eq!(budget.reads(), 3);
    let requests = script.requests.lock().unwrap();
    let q = &requests[0].2["structuredQuery"];
    assert_eq!(q["limit"], 1);
    assert_eq!(q["orderBy"][0]["field"]["fieldPath"], "counter");
    let filters = q["where"]["compositeFilter"]["filters"].as_array().unwrap();
    assert_eq!(filters.len(), 3);
    assert!(
        filters
            .iter()
            .any(|f| f["fieldFilter"]["op"] == "GREATER_THAN")
    );
}
#[test]
fn a_gap_or_wrong_scope_never_discloses_a_partial_page() {
    for (body, error) in [
        (
            json!([{"document":remote_document(3)}]),
            SyncError::CatchUpRequired,
        ),
        (
            {
                let mut d = remote_document(2);
                d["fields"]["tenant"] = json!({"stringValue":"other-space"});
                json!([{"document":d}])
            },
            SyncError::Denied,
        ),
    ] {
        let script = Arc::new(Script::new(vec![Ok(response(200, body))]));
        let (client, _) = native_client(script, auth());
        let cancel = AtomicBool::new(false);
        let io = io(&cancel);
        assert!(matches!(ready(client.query(&query(1),&io)),Err(e) if e==error));
    }
}
#[test]
fn publish_is_one_create_and_unknown_never_retries() {
    let script = Arc::new(Script::new(vec![Err(AuthError::unavailable())]));
    let (client, budget) = native_client(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    assert!(matches!(
        ready(client.publish(&Mutation::Create(doc()), &io)).unwrap(),
        Published::Unknown
    ));
    let requests = script.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, Operation::Commit);
    assert_eq!(requests[0].2["writes"].as_array().unwrap().len(), 1);
    assert_eq!(
        requests[0].2["writes"][0]["currentDocument"]["exists"],
        false
    );
    assert_eq!(requests[0].2["writes"][0]["update"]["fields"], fields(1));
    assert_eq!(budget.reads(), 3);
}
#[test]
fn invalid_binding_expired_or_changed_auth_stops_before_http() {
    let script = Arc::new(Script::new(vec![]));
    let authority = auth();
    let (client, _) = native_client(script.clone(), authority.clone());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    let mut bad = doc();
    bad.digest = "0".repeat(64);
    assert_eq!(
        ready(client.publish(&Mutation::Create(bad), &io)).err(),
        Some(SyncError::Invalid)
    );
    assert_eq!(authority.calls.load(Ordering::Relaxed), 0);
    for (uid, expired) in [("other-user", false), ("fixture-user", true)] {
        let (client, _) = native_client(
            script.clone(),
            Arc::new(Auth {
                calls: AtomicUsize::new(0),
                uid: uid.into(),
                expired,
                blocked: AtomicBool::new(false),
            }),
        );
        assert!(ready(client.read(&doc().target, &io)).is_err());
        assert!(ready(client.publish(&Mutation::Create(doc()), &io)).is_err());
    }
    assert!(script.requests.lock().unwrap().is_empty());
}
#[test]
fn budget_and_denial_are_typed_and_nothing_is_sent_on_local_refusal() {
    let script = Arc::new(Script::new(vec![Ok(response(
        403,
        json!({"error":{"message":"secret-provider-canary"}}),
    ))]));
    let (client, budget) = native_client(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    assert_eq!(
        ready(client.read(&doc().target, &io)).err(),
        Some(SyncError::Denied)
    );
    budget.set_reads(44_995);
    assert_eq!(client.budget_state().unwrap(), Decision::Warn);
    assert!(matches!(
        ready(client.read(&doc().target, &io)),
        Err(SyncError::BudgetExhausted(_))
    ));
    assert_eq!(script.requests.lock().unwrap().len(), 1);
}
#[test]
fn close_and_cancel_after_possible_write_preserve_unknown() {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut script = Script::new(vec![Ok(response(
        200,
        json!({"writeResults":[{"updateTime":"2026-10-13T00:00:00Z"}]}),
    ))]);
    script.cancel = Some(cancel.clone());
    let script = Arc::new(script);
    let (client, _) = native_client(script.clone(), auth());
    let io = io(&cancel);
    assert!(matches!(
        ready(client.publish(&Mutation::Create(doc()), &io)).unwrap(),
        Published::Unknown
    ));
    client.close();
    assert!(ready(client.read(&doc().target, &io)).is_err());
    assert_eq!(script.requests.lock().unwrap().len(), 1);
}
#[test]
fn polling_returns_catchup_on_gap_and_denial_ends_the_subscription() {
    let script = Arc::new(Script::new(vec![
        Ok(response(200, json!([{"document":remote_document(3)}]))),
        Ok(response(
            403,
            json!({"error":{"message":"PERMISSION_DENIED"}}),
        )),
    ]));
    let (client, _) = native_client(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    let mut subscription = ready(client.subscribe(&stream(1), &io)).unwrap();
    assert!(matches!(
        ready(subscription.next(&io)).unwrap(),
        Change::CatchUpRequired
    ));
    subscription.close();
    let mut denied = ready(client.subscribe(&stream(1), &io)).unwrap();
    assert_eq!(ready(denied.next(&io)).err(), Some(SyncError::Denied));
    assert!(ready(denied.next(&io)).is_err());
    assert_eq!(script.requests.lock().unwrap().len(), 2);
}

#[test]
fn provider_exhaustion_refuses_reads_but_preserves_unknown_writes_and_stops_next_effect() {
    for mutation in [false, true] {
        let script = Arc::new(Script::new(vec![Ok(response(
            429,
            json!({"error":{"status":"RESOURCE_EXHAUSTED","message":"quota-canary"}}),
        ))]));
        let (client, budget) = native_client(script.clone(), auth());
        let cancel = AtomicBool::new(false);
        let io = io(&cancel);
        if mutation {
            assert!(matches!(
                ready(client.publish(&Mutation::Create(doc()), &io)).unwrap(),
                Published::Unknown
            ));
        } else {
            assert!(matches!(
                ready(client.read(&doc().target, &io)),
                Err(SyncError::BudgetExhausted(_))
            ));
        }
        assert!(matches!(
            ready(client.read(&doc().target, &io)),
            Err(SyncError::BudgetExhausted(_))
        ));
        assert_eq!(budget.reads(), 3);
        assert!(matches!(
            client.budget_state().unwrap(),
            Decision::Refuse(_)
        ));
        assert_eq!(script.requests.lock().unwrap().len(), 1);
    }
}

#[test]
fn subscription_capacity_refuses_before_http_and_close_releases_it() {
    let script = Arc::new(Script::new(vec![]));
    let (client, _) = native_client(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    let mut subscriptions = Vec::new();
    for _ in 0..3 {
        subscriptions.push(ready(client.subscribe(&stream(0), &io)).unwrap());
    }
    assert_eq!(
        ready(client.subscribe(&stream(0), &io)).err(),
        Some(SyncError::RateLimited)
    );
    subscriptions[0].close();
    let mut replacement = ready(client.subscribe(&stream(0), &io)).unwrap();
    client.close();
    assert!(ready(replacement.next(&io)).is_err());
    assert!(script.requests.lock().unwrap().is_empty());
}

#[test]
fn provider_page_over_the_carrier_cap_is_never_retained_or_disclosed() {
    let script = Arc::new(Script::new(vec![Ok(Response {
        status: 200,
        body: vec![b' '; 397_313],
        retry_after_ms: None,
    })]));
    let (client, budget) = native_client(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    assert_eq!(
        ready(client.query(&query(0), &io)).err(),
        Some(SyncError::CatchUpRequired)
    );
    assert_eq!(budget.reads(), 3);
    assert_eq!(script.requests.lock().unwrap().len(), 1);
}

#[test]
fn idle_poll_wait_stops_at_the_caller_deadline_without_a_second_request() {
    let script = Arc::new(Script::new(vec![Ok(response(
        200,
        json!([{"readTime":"2026-10-13T00:00:00Z"}]),
    ))]));
    let (client, budget) = native_client(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    let mut subscription = ready(client.subscribe(&stream(0), &io)).unwrap();
    assert_eq!(
        ready(subscription.next(&io)).err(),
        Some(SyncError::Deadline)
    );
    assert_eq!(budget.reads(), 3);
    assert_eq!(script.requests.lock().unwrap().len(), 1);
}

#[test]
fn discovery_pages_are_named_scope_only_and_completion_requires_the_bounded_run_end() {
    let script = Arc::new(Script::new(vec![
        Ok(response(200, json!([{"document":remote_document(1)}]))),
        Ok(response(200, json!([{"readTime":"2026-10-13T00:00:00Z"}]))),
    ]));
    let (client, budget) = native_client(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    let SyncQueryPage::Discovery(first) = ready(client.query(&discovery(None), &io)).unwrap()
    else {
        panic!("wrong page shape")
    };
    assert_eq!(first.documents, vec![doc()]);
    let continuation = first.continuation.unwrap();
    assert!(continuation.0.len() <= 1024);
    let SyncQueryPage::Discovery(last) =
        ready(client.query(&discovery(Some(continuation)), &io)).unwrap()
    else {
        panic!("wrong page shape")
    };
    assert!(last.documents.is_empty());
    assert!(last.continuation.is_none());
    assert_eq!(budget.reads(), 6);
    let requests = script.requests.lock().unwrap();
    let q = &requests[0].2["structuredQuery"];
    let filters = q["where"]["compositeFilter"]["filters"].as_array().unwrap();
    assert_eq!(filters.len(), 2);
    assert!(filters.iter().all(|f| f["fieldFilter"]["op"] == "EQUAL"));
    assert_eq!(q["orderBy"][0]["field"]["fieldPath"], "__name__");
    assert_eq!(
        requests[1].2["structuredQuery"]["startAt"]["values"][0]["referenceValue"],
        remote_document(1)["name"]
    );
}

#[test]
fn later_discovery_pages_cannot_extend_the_original_run_deadline() {
    let script = Arc::new(Script::new(vec![
        Ok(response(200, json!([{"document":remote_document(1)}]))),
        Ok(response(200, json!([{"readTime":"2026-10-13T00:00:00Z"}]))),
    ]));
    let (client, _, clock) = native_with_clock(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let first_io = io(&cancel);
    let SyncQueryPage::Discovery(first) = ready(client.query(&discovery(None), &first_io)).unwrap()
    else {
        panic!("wrong page shape");
    };
    clock.0.store(10_000, Ordering::SeqCst);
    let later_io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(60),
        cancelled: &cancel,
    };
    ready(client.query(&discovery(first.continuation), &later_io)).unwrap();
    assert!(
        script.deadlines.lock().unwrap()[1] <= 30_000,
        "a later caller budget must not renew a listing run"
    );
}

#[test]
fn tampered_continuation_and_wrong_shape_refuse_before_another_effect() {
    let script = Arc::new(Script::new(vec![Ok(response(
        200,
        json!([{"document":remote_document(1)}]),
    ))]));
    let (client, budget) = native_client(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    let SyncQueryPage::Discovery(first) = ready(client.query(&discovery(None), &io)).unwrap()
    else {
        panic!("wrong page shape")
    };
    let mut token = first.continuation.unwrap().0.into_bytes();
    token[0] = if token[0] == b'A' { b'B' } else { b'A' };
    assert!(
        ready(client.query(
            &discovery(Some(DiscoveryContinuation(
                String::from_utf8(token).unwrap()
            ))),
            &io
        ))
        .is_err()
    );
    let SyncQueryRequest::Discovery(mut wrong) = discovery(None) else {
        unreachable!()
    };
    wrong.shape = "invented".into();
    assert_eq!(
        ready(client.query(&SyncQueryRequest::Discovery(wrong), &io)).err(),
        Some(SyncError::Invalid)
    );
    assert_eq!(budget.reads(), 3);
    assert_eq!(script.requests.lock().unwrap().len(), 1);
}

#[test]
fn create_already_present_compares_the_same_frozen_bytes_without_resending() {
    for (seq, expected) in [(1, None), (2, Some(SyncError::Conflict))] {
        let script = Arc::new(Script::new(vec![
            Ok(response(409, json!({"error":{"status":"ALREADY_EXISTS"}}))),
            Ok(response(200, remote_document(seq))),
        ]));
        // Conflict still has the SAME immutable ID, but different frozen fields.
        let mut replacement = remote_document(seq);
        replacement["name"] = remote_document(1)["name"].clone();
        script.replies.lock().unwrap()[1] = Ok(response(200, replacement));
        let (client, budget) = native_client(script.clone(), auth());
        let cancel = AtomicBool::new(false);
        let io = io(&cancel);
        let outcome = ready(client.publish(&Mutation::Create(doc()), &io));
        match expected {
            Some(error) => assert_eq!(outcome.err(), Some(error)),
            None => assert!(matches!(outcome.unwrap(), Published::AlreadyPresent)),
        }
        let requests = script.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].0, Operation::Commit);
        assert_eq!(requests[1].0, Operation::Read);
        assert_eq!(budget.reads(), 6);
    }
}

#[test]
fn absent_write_modes_refuse_put_and_delete_before_auth_or_http() {
    let script = Arc::new(Script::new(vec![]));
    let authority = auth();
    let (client, budget) = native_client(script.clone(), authority.clone());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    for mutation in [Mutation::Put(doc()), Mutation::Delete(doc().target)] {
        assert_eq!(
            ready(client.publish(&mutation, &io)).err(),
            Some(SyncError::Unsupported)
        );
    }
    assert_eq!(authority.calls.load(Ordering::Relaxed), 0);
    assert_eq!(budget.reads(), 0);
    assert!(script.requests.lock().unwrap().is_empty());
}

#[test]
fn discovery_position_expires_or_belongs_to_another_generation_before_http() {
    let script = Arc::new(Script::new(vec![Ok(response(
        200,
        json!([{"document":remote_document(1)}]),
    ))]));
    let (client, budget, clock) = native_with_clock(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    let SyncQueryPage::Discovery(first) = ready(client.query(&discovery(None), &io)).unwrap()
    else {
        panic!("wrong page shape")
    };
    let continuation = first.continuation.unwrap();
    let (other_generation, _) = native_client(script.clone(), auth());
    assert!(
        ready(other_generation.query(
            &discovery(Some(DiscoveryContinuation(continuation.0.clone()))),
            &io
        ))
        .is_err()
    );
    clock.0.store(30_000, Ordering::SeqCst);
    assert_eq!(
        ready(client.query(
            &discovery(Some(DiscoveryContinuation(continuation.0.clone()))),
            &io
        ))
        .err(),
        Some(SyncError::Deadline)
    );
    client.close();
    assert_eq!(
        ready(client.query(&discovery(Some(continuation)), &io)).err(),
        Some(SyncError::Cancelled)
    );
    assert_eq!(budget.reads(), 3);
    assert_eq!(script.requests.lock().unwrap().len(), 1);
}

#[test]
fn notice_is_coalesced_and_sampling_does_not_advance_colabs_cursor() {
    let script = Arc::new(Script::new(vec![
        Ok(response(200, json!([{"document":remote_document(1)}]))),
        Ok(response(200, json!([{"document":remote_document(1)}]))),
    ]));
    let (client, budget) = native_client(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let first_io = io(&cancel);
    let mut subscription = ready(client.subscribe(&stream(0), &first_io)).unwrap();
    assert!(matches!(
        ready(subscription.next(&first_io)).unwrap(),
        Change::Changed
    ));
    let next_io = IoBudget {
        deadline: Instant::now() + Duration::from_secs(6),
        cancelled: &cancel,
    };
    // The repeated sample of seq 1 produces no second hint and remains anchored
    // at Colab's supplied seq 0, rather than advancing on the first wake hint.
    assert_eq!(
        ready(subscription.next(&next_io)).err(),
        Some(SyncError::Deadline)
    );
    assert_eq!(budget.reads(), 6);
    let requests = script.requests.lock().unwrap();
    for request in requests.iter() {
        let filters = request.2["structuredQuery"]["where"]["compositeFilter"]["filters"]
            .as_array()
            .unwrap();
        let seq = filters
            .iter()
            .find(|f| f["fieldFilter"]["op"] == "GREATER_THAN")
            .unwrap();
        assert_eq!(seq["fieldFilter"]["value"]["integerValue"], "0");
    }
}

#[test]
fn periodic_page_discovery_notices_a_new_writer_at_seq_one() {
    let mut arrival = remote_document(1);
    arrival["name"] =
        json!("projects/demo-remote-1/databases/(default)/documents/x/notes/rows/new-writer+1");
    arrival["fields"]["streamKey"] = json!({"stringValue":"new-writer"});
    let script = Arc::new(Script::new(vec![
        Ok(response(200, json!([{"document":arrival}]))),
        Ok(response(200, json!([{"readTime":"2026-10-13T00:00:00Z"}]))),
    ]));
    let (client, budget, clock) = native_with_clock(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let first_io = io(&cancel);
    let mut subscription = ready(client.subscribe(&stream(900), &first_io)).unwrap();
    clock.0.store(60_000, Ordering::SeqCst);
    assert!(matches!(
        ready(subscription.next(&first_io)).unwrap(),
        Change::Changed
    ));
    assert_eq!(budget.reads(), 6);
    let requests = script.requests.lock().unwrap();
    let filters = requests[0].2["structuredQuery"]["where"]["compositeFilter"]["filters"]
        .as_array()
        .unwrap();
    assert!(filters.iter().all(|f| f["fieldFilter"]["op"] == "EQUAL"));
    assert!(
        !filters
            .iter()
            .any(|f| f["fieldFilter"]["field"]["fieldPath"] == "streamKey")
    );
    assert_eq!(
        requests[0].2["structuredQuery"]["orderBy"][0]["field"]["fieldPath"],
        "__name__"
    );
}

#[test]
fn declared_put_and_delete_are_unconditional_single_target_effects() {
    let script = Arc::new(Script::new(vec![
        Ok(response(
            200,
            json!({"writeResults":[{"updateTime":"2026-10-13T00:00:00Z"}]}),
        )),
        Ok(response(
            200,
            json!({"writeResults":[{}],"commitTime":"2026-10-13T00:00:01Z"}),
        )),
    ]));
    let (client, budget) = mutable_client(script.clone(), auth());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    for mutation in [Mutation::Put(doc()), Mutation::Delete(doc().target)] {
        assert!(matches!(
            ready(client.publish(&mutation, &io)).unwrap(),
            Published::Confirmed
        ));
    }
    let requests = script.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        assert_eq!(request.0, Operation::Commit);
        let write = &request.2["writes"][0];
        assert!(write.get("currentDocument").is_none());
        assert!(write.get("updateMask").is_none());
        assert!(write.get("updateTransforms").is_none());
    }
    assert_eq!(requests[0].2["writes"][0]["update"]["fields"], fields(1));
    assert!(
        requests[1].2["writes"][0]["delete"]
            .as_str()
            .unwrap()
            .ends_with("writer-stream+1")
    );
    assert_eq!(budget.reads(), 6);
}

#[test]
fn an_unmapped_delete_reserves_the_resource_until_the_pending_write_is_released() {
    let script = Arc::new(Script::new(vec![]));
    let authority = auth();
    authority.blocked.store(true, Ordering::SeqCst);
    let (client, budget) = mutable_client(script.clone(), authority.clone());
    let cancel = AtomicBool::new(false);
    let io = io(&cancel);
    let mutation = Mutation::Create(doc());
    let mut pending = Box::pin(client.publish(&mutation, &io));
    assert!(matches!(
        pending
            .as_mut()
            .poll(&mut Context::from_waker(std::task::Waker::noop())),
        Poll::Pending
    ));
    assert_eq!(
        ready(client.publish(&Mutation::Delete(doc().target), &io)).err(),
        Some(SyncError::RateLimited)
    );
    drop(pending);
    authority.blocked.store(false, Ordering::SeqCst);
    client.close();
    assert!(ready(client.publish(&Mutation::Delete(doc().target), &io)).is_err());
    assert_eq!(budget.reads(), 0);
    assert!(script.requests.lock().unwrap().is_empty());
}
