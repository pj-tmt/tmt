use super::*;
use crate::sealed_sync::*;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};
const GEN: &str = "b32f579c-e87a-4a2c-8c79-e53d95b1584b";
const BINDING: &str = "df7e62f9-4ceb-4ff2-83d1-a6227567194f";
#[test]
fn oversized_provider_page_is_refused_before_fields_are_copied_or_parsed() {
    let fake = Fake::new();
    let mut document = fake.document.clone();
    document.bytes = vec![b'x'; tmt_extension_objects::limits::V3_FRAME_BYTES + 1];
    let page = SyncQueryPage::Stream(Page {
        documents: vec![document],
        cursor: None,
    });
    assert_eq!(
        page_outcome(fake.context(), &page),
        Err(SyncError::CatchUpRequired)
    );
}
pub(super) struct Auth;
impl AuthPort for Auth {
    fn identity(&self) -> AuthIdentity {
        AuthIdentity {
            project: "demo-remote-1".into(),
            uid: "fixture-user".into(),
        }
    }
    fn token<'a>(
        &'a self,
        _: &'a IoBudget<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<AuthToken, AuthFailure>> + Send + 'a>> {
        panic!("no credentials in fake carrier")
    }
}
pub(in crate::object_service) struct Fake {
    context: SyncContext,
    document: Document,
    reads: AtomicUsize,
    queries: AtomicUsize,
    returned_seq: AtomicU64,
    copies: AtomicUsize,
    refuse: AtomicBool,
    pub(in crate::object_service) subscription: Arc<SubState>,
    writes: Mutex<Vec<(String, Target, Vec<u8>)>>,
    publish_result: AtomicUsize,
}
impl Fake {
    pub(in crate::object_service) fn new() -> Self {
        let v: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/sealed_sync/codec-v1.json"
        ))
        .unwrap();
        let context = SyncContext::bind(json!({"apiKey":"public", "appId":"1:123:web:fixture", "projectId":"demo-remote-1", "authDomain":"demo-remote-1.firebaseapp.com"}), v["declaration"].as_str().unwrap().as_bytes(),v["declarationDigest"].as_str().unwrap(),"fixture-space",Arc::new(Auth)).unwrap();
        Self {
            context,
            document: Document {
                target: Target {
                    resource: "rows".into(),
                    id: "writer-stream+1".into(),
                },
                bytes: v["document"].as_str().unwrap().as_bytes().to_vec(),
                digest: v["documentDigest"].as_str().unwrap().into(),
            },
            reads: AtomicUsize::new(0),
            queries: AtomicUsize::new(0),
            returned_seq: AtomicU64::new(1),
            copies: AtomicUsize::new(1),
            refuse: AtomicBool::new(false),
            subscription: Arc::new(SubState::default()),
            writes: Mutex::new(Vec::new()),
            publish_result: AtomicUsize::new(0),
        }
    }
}
impl SealedSyncPort for Fake {
    fn context(&self) -> &SyncContext {
        &self.context
    }
    fn read<'a>(
        &'a self,
        target: &'a Target,
        _: &'a IoBudget<'a>,
    ) -> Pending<'a, Option<Document>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        assert_eq!(*target, self.document.target);
        Box::pin(async {
            if self.refuse.load(Ordering::SeqCst) {
                return Err(SyncError::BudgetExhausted(
                    crate::firestore_budget::Refusal {
                        reset_at_ms: 123456,
                        retry_after_ms: 4000,
                    },
                ));
            }
            Ok(Some(self.document.clone()))
        })
    }
    fn query<'a>(
        &'a self,
        query: &'a SyncQueryRequest,
        _: &'a IoBudget<'a>,
    ) -> Pending<'a, SyncQueryPage> {
        self.queries.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            Ok(match query {
                SyncQueryRequest::Stream(q) => {
                    assert_eq!(q.cursor.as_ref().map(|c| c.seq), Some(0));
                    SyncQueryPage::Stream(Page {
                        documents: vec![self.document.clone(); self.copies.load(Ordering::SeqCst)],
                        cursor: Some(SyncCursor {
                            stream: q.stream.clone(),
                            seq: self.returned_seq.load(Ordering::SeqCst),
                        }),
                    })
                }
                SyncQueryRequest::Discovery(q) => {
                    assert_eq!(q.shape, "arrivals");
                    assert!(!q.scope.contains_key("streamKey"));
                    SyncQueryPage::Discovery(DiscoveryPage {
                        documents: vec![self.document.clone(); self.copies.load(Ordering::SeqCst)],
                        continuation: Some(DiscoveryContinuation("opaque-next".into())),
                    })
                }
            })
        })
    }
    fn subscribe<'a>(
        &'a self,
        interest: &'a SyncInterest,
        _: &'a IoBudget<'a>,
    ) -> Pending<'a, Box<dyn Subscription>> {
        assert_eq!(interest.resource, "rows");
        assert_eq!(interest.scope.len(), 1);
        assert!(!interest.scope.contains_key("streamKey"));
        Box::pin(async {
            Ok(Box::new(FakeSubscription(self.subscription.clone())) as Box<dyn Subscription>)
        })
    }
    fn publish<'a>(
        &'a self,
        mutation: &'a Mutation,
        _: &'a IoBudget<'a>,
    ) -> Pending<'a, Published> {
        let (mode, target, bytes) = match mutation {
            Mutation::Create(doc) => ("create", doc.target.clone(), doc.bytes.clone()),
            Mutation::Put(doc) => ("put", doc.target.clone(), doc.bytes.clone()),
            Mutation::Delete(target) => ("delete", target.clone(), Vec::new()),
        };
        self.writes
            .lock()
            .unwrap()
            .push((mode.into(), target, bytes));
        Box::pin(async {
            match self.publish_result.load(Ordering::SeqCst) {
                0 => Ok(Published::Confirmed),
                1 => Ok(Published::AlreadyPresent),
                2 => Ok(Published::Unknown),
                _ => Err(SyncError::Conflict),
            }
        })
    }
    fn close(&self) {}
}
#[derive(Default)]
pub(in crate::object_service) struct SubState {
    pending: Mutex<(VecDeque<Change>, Option<Waker>)>,
    failures: Mutex<VecDeque<SyncError>>,
    pub(in crate::object_service) closed: AtomicUsize,
}
impl SubState {
    fn changed(&self) {
        let mut state = self.pending.lock().unwrap();
        state.0.push_back(Change::Changed);
        let waker = state.1.take();
        drop(state);
        if let Some(waker) = waker {
            waker.wake()
        }
    }
}
struct FakeSubscription(Arc<SubState>);
impl Subscription for FakeSubscription {
    fn next<'a>(&'a mut self, io: &'a IoBudget<'a>) -> Pending<'a, Change> {
        Box::pin(std::future::poll_fn(move |cx| {
            if io.check().is_err() {
                return Poll::Ready(Err(SyncError::Cancelled));
            }
            if let Some(error) = self.0.failures.lock().unwrap().pop_front() {
                return Poll::Ready(Err(error));
            }
            let mut state = self.0.pending.lock().unwrap();
            if let Some(change) = state.0.pop_front() {
                Poll::Ready(Ok(change))
            } else {
                state.1 = Some(cx.waker().clone());
                Poll::Pending
            }
        }))
    }
    fn close(&mut self) {
        self.0.closed.fetch_add(1, Ordering::SeqCst);
    }
}
fn ready<T>(future: impl Future<Output = T>) -> T {
    let mut cx = Context::from_waker(Waker::noop());
    match std::pin::pin!(future).as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("fake must finish synchronously"),
    }
}
fn request(id: u64, call: wire::Call) -> wire::Frame {
    wire::Frame::Request {
        version: 1,
        generation: GEN.into(),
        request_id: id.to_string(),
        call,
    }
}
fn outcome(frame: Result<wire::Frame, SyncError>) -> wire::Outcome {
    match frame.unwrap() {
        wire::Frame::Result { outcome, .. } => outcome,
        _ => panic!("not result"),
    }
}
fn io(cancelled: &AtomicBool) -> IoBudget<'_> {
    IoBudget {
        deadline: Instant::now() + Duration::from_secs(10),
        cancelled,
    }
}
#[test]
fn bind_read_and_named_queries_preserve_fields_and_consumer_positions() {
    let fake = Arc::new(Fake::new());
    let adapter = Adapter::new(
        fake.clone(),
        Uuid4::parse(GEN).unwrap(),
        Uuid4::parse(BINDING).unwrap(),
    );
    let stop = AtomicBool::new(false);
    let budget = io(&stop);
    let ctx = &fake.context;
    let bound = outcome(ready(adapter.execute(
        &request(
            1,
            wire::Call::Bind {
                project: ctx.identity().project.clone(),
                extension: ctx.extension().into(),
                declaration_digest: ctx.declaration_digest().into(),
                space: ctx.space().into(),
            },
        ),
        &budget,
    )));
    assert!(
        matches!(bound,wire::Outcome::Binding{context} if context.id==BINDING && context.uid=="fixture-user")
    );
    let got = outcome(ready(adapter.execute(
        &request(
            2,
            wire::Call::Read {
                binding: BINDING.into(),
                target: wire::Target {
                    resource: "rows".into(),
                    id: "writer-stream+1".into(),
                },
            },
        ),
        &budget,
    )));
    match got {
        wire::Outcome::Read {
            document: Some(doc),
        } => {
            assert_eq!(
                serde_json::to_vec(&doc.fields).unwrap(),
                fake.document.bytes
            );
            assert_eq!(doc.digest, fake.document.digest)
        }
        _ => panic!("read must return canonical fields"),
    }
    for (id,query) in [(3,wire::Query::Stream{resource:"rows".into(),scope:serde_json::from_value(json!({"tenant":{"stringValue":"fixture-space"},"streamKey":{"stringValue":"writer-stream"}})).unwrap(),stream:"writer-stream".into(),cursor:Some(wire::StreamPosition{stream:"writer-stream".into(),seq:0})}), (4,wire::Query::Discovery{resource:"rows".into(),name:"arrivals".into(),scope:serde_json::from_value(json!({"tenant":{"stringValue":"fixture-space"}})).unwrap(),continuation:None})] {
        let got=outcome(ready(adapter.execute(&request(id,wire::Call::Query{binding:BINDING.into(),query}),&budget)));
        assert!(match got {wire::Outcome::Stream{documents,cursor}=>documents.len()==1 && cursor.unwrap().seq==1,wire::Outcome::Discovery{documents,continuation}=>documents.len()==1 && continuation.as_deref()==Some("opaque-next"),_=>false});
    }
    assert_eq!(fake.reads.load(Ordering::SeqCst), 1);
    assert_eq!(fake.queries.load(Ordering::SeqCst), 2);
}
#[test]
fn mismatched_binding_or_scope_refuses_before_provider_io() {
    let fake = Arc::new(Fake::new());
    let adapter = Adapter::new(
        fake.clone(),
        Uuid4::parse(GEN).unwrap(),
        Uuid4::parse(BINDING).unwrap(),
    );
    let stop = AtomicBool::new(false);
    let budget = io(&stop);
    let got = outcome(ready(adapter.execute(
        &request(
            1,
            wire::Call::Read {
                binding: GEN.into(),
                target: wire::Target {
                    resource: "rows".into(),
                    id: "writer-stream+1".into(),
                },
            },
        ),
        &budget,
    )));
    assert!(matches!(got,wire::Outcome::Error{error} if error.code==wire::Code::Denied));
    let ctx = &fake.context;
    let bound = outcome(ready(adapter.execute(
        &request(
            3,
            wire::Call::Bind {
                project: ctx.identity().project.clone(),
                extension: ctx.extension().into(),
                declaration_digest: ctx.declaration_digest().into(),
                space: ctx.space().into(),
            },
        ),
        &budget,
    )));
    assert!(matches!(bound, wire::Outcome::Binding { .. }));
    let got = outcome(ready(
        adapter.execute(
            &request(
                2,
                wire::Call::Query {
                    binding: BINDING.into(),
                    query: wire::Query::Discovery {
                        resource: "rows".into(),
                        name: "arrivals".into(),
                        scope: serde_json::from_value(
                            json!({"tenant":{"stringValue":"another-space"}}),
                        )
                        .unwrap(),
                        continuation: None,
                    },
                },
            ),
            &budget,
        ),
    ));
    assert!(matches!(got,wire::Outcome::Error{error} if error.code==wire::Code::Denied));
    assert_eq!(fake.reads.load(Ordering::SeqCst), 0);
    assert_eq!(fake.queries.load(Ordering::SeqCst), 0);
}

fn bound_adapter(fake: Arc<Fake>, budget: &IoBudget<'_>) -> Adapter {
    let adapter = Adapter::new(
        fake.clone(),
        Uuid4::parse(GEN).unwrap(),
        Uuid4::parse(BINDING).unwrap(),
    );
    let ctx = &fake.context;
    assert!(matches!(
        outcome(ready(adapter.execute(
            &request(
                1,
                wire::Call::Bind {
                    project: ctx.identity().project.clone(),
                    extension: ctx.extension().into(),
                    declaration_digest: ctx.declaration_digest().into(),
                    space: ctx.space().into()
                }
            ),
            budget
        ))),
        wire::Outcome::Binding { .. }
    ));
    adapter
}
fn tail_request() -> wire::Frame {
    request(2,wire::Call::Query{binding:BINDING.into(),query:wire::Query::Stream{resource:"rows".into(),scope:serde_json::from_value(json!({"tenant":{"stringValue":"fixture-space"},"streamKey":{"stringValue":"writer-stream"}})).unwrap(),stream:"writer-stream".into(),cursor:Some(wire::StreamPosition{stream:"writer-stream".into(),seq:0})}})
}
#[test]
fn an_omitted_provider_tail_never_advances_the_consumer_cursor() {
    let fake = Arc::new(Fake::new());
    let stop = AtomicBool::new(false);
    let budget = io(&stop);
    let adapter = bound_adapter(fake.clone(), &budget);
    fake.returned_seq.store(99, Ordering::SeqCst);
    assert!(
        matches!(outcome(ready(adapter.execute(&tail_request(),&budget))),wire::Outcome::Error{error} if error.state==Some(wire::State::CatchUpRequired))
    );
}
#[test]
fn oversized_provider_pages_and_budget_refusals_are_typed_without_disclosure() {
    let fake = Arc::new(Fake::new());
    let stop = AtomicBool::new(false);
    let budget = io(&stop);
    let adapter = bound_adapter(fake.clone(), &budget);
    fake.copies.store(257, Ordering::SeqCst);
    assert!(
        matches!(outcome(ready(adapter.execute(&tail_request(),&budget))),wire::Outcome::Error{error} if error.state==Some(wire::State::CatchUpRequired))
    );
    fake.refuse.store(true, Ordering::SeqCst);
    let read = request(
        3,
        wire::Call::Read {
            binding: BINDING.into(),
            target: wire::Target {
                resource: "rows".into(),
                id: "writer-stream+1".into(),
            },
        },
    );
    assert!(
        matches!(outcome(ready(adapter.execute(&read,&budget))),wire::Outcome::Error{error} if error.code==wire::Code::Budget && error.reset_at_ms==Some(123456) && error.retry_after_ms==Some(4000))
    );
}

#[test]
fn an_empty_page_interest_needs_no_discovery_shape_and_owns_no_query_position() {
    let mut fake = Fake::new();
    let v: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/sealed_sync/codec-v1.json"
    ))
    .unwrap();
    let mut declaration: Value = serde_json::from_str(v["declaration"].as_str().unwrap()).unwrap();
    declaration["resources"][0]["sync"]["query"]
        .as_object_mut()
        .unwrap()
        .remove("discovery");
    let bytes = serde_json::to_vec(&declaration).unwrap();
    fake.context = SyncContext::bind(
        fake.context.config().clone(),
        &bytes,
        &crate::deploy_plan::sha256_hex(&bytes),
        "fixture-space",
        Arc::new(Auth),
    )
    .unwrap();
    let fake = Arc::new(fake);
    let stop = AtomicBool::new(false);
    let budget = io(&stop);
    let adapter = bound_adapter(fake.clone(), &budget);
    let id = "2";
    let subscribe = request(
        2,
        wire::Call::Subscribe {
            binding: BINDING.into(),
            interest: wire::Interest {
                resource: "rows".into(),
                scope: serde_json::from_value(json!({"tenant":{"stringValue":"fixture-space"}}))
                    .unwrap(),
            },
        },
    );
    assert!(
        matches!(outcome(ready(adapter.execute(&subscribe,&budget))),wire::Outcome::Subscribed{id:returned} if returned==id)
    );
    fake.subscription.changed();
    assert!(
        adapter.next_notice(Instant::now()).is_none(),
        "register result must precede notices"
    );
    adapter.registered(id);
    assert!(matches!(
        adapter.next_notice(Instant::now() + Duration::from_secs(2)),
        Some(wire::Frame::Notice {
            notice: wire::InterestNotice::Changed,
            ..
        })
    ));
    let cancel = request(
        3,
        wire::Call::Cancel {
            binding: BINDING.into(),
            subscription_id: id.into(),
        },
    );
    assert!(matches!(
        outcome(ready(adapter.execute(&cancel, &budget))),
        wire::Outcome::Cancelled { .. }
    ));
    assert_eq!(fake.subscription.closed.load(Ordering::SeqCst), 1);
    fake.subscription.changed();
    assert!(
        adapter.next_notice(Instant::now()).is_none(),
        "cancel fences late wake"
    );
    assert_eq!(fake.queries.load(Ordering::SeqCst), 0);
}

#[test]
fn publish_unknown_keeps_the_same_original_and_never_retries_inside_the_carrier() {
    let fake = Arc::new(Fake::new());
    let stop = AtomicBool::new(false);
    let budget = io(&stop);
    let adapter = bound_adapter(fake.clone(), &budget);
    let document = wire::Document {
        target: wire::Target {
            resource: fake.document.target.resource.clone(),
            id: fake.document.target.id.clone(),
        },
        fields: validate_fields(&fake.document.bytes).unwrap(),
        digest: fake.document.digest.clone(),
    };
    let call = wire::Call::Publish {
        binding: BINDING.into(),
        mutation: wire::Mutation::Create { document },
    };
    fake.publish_result.store(2, Ordering::SeqCst);
    assert!(matches!(
        outcome(ready(adapter.execute(&request(2, call.clone()), &budget))),
        wire::Outcome::Published {
            status: wire::Published::Unknown
        }
    ));
    assert_eq!(fake.writes.lock().unwrap().len(), 1, "no carrier retry");
    fake.publish_result.store(1, Ordering::SeqCst);
    assert!(matches!(
        outcome(ready(adapter.execute(&request(3, call), &budget))),
        wire::Outcome::Published {
            status: wire::Published::AlreadyPresent
        }
    ));
    let writes = fake.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0], writes[1]);
    assert_eq!(writes[0].2, fake.document.bytes);
    assert_eq!(fake.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn declared_put_and_delete_use_one_unconditional_port_call_each() {
    let mut fake = Fake::new();
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/sealed_sync/codec-v1.json"
    ))
    .unwrap();
    let mut declaration: Value =
        serde_json::from_str(vectors["declaration"].as_str().unwrap()).unwrap();
    declaration["resources"][0]["kind"] = json!("checkpoint");
    declaration["resources"][0]["sync"]["writeModes"] = json!(["create", "put", "delete"]);
    let bytes = serde_json::to_vec(&declaration).unwrap();
    fake.context = SyncContext::bind(
        fake.context.config().clone(),
        &bytes,
        &crate::deploy_plan::sha256_hex(&bytes),
        "fixture-space",
        Arc::new(Auth),
    )
    .unwrap();
    let fake = Arc::new(fake);
    let stop = AtomicBool::new(false);
    let budget = io(&stop);
    let adapter = bound_adapter(fake.clone(), &budget);
    let target = wire::Target {
        resource: fake.document.target.resource.clone(),
        id: fake.document.target.id.clone(),
    };
    let document = wire::Document {
        target: target.clone(),
        fields: validate_fields(&fake.document.bytes).unwrap(),
        digest: fake.document.digest.clone(),
    };
    for (id, mutation) in [
        (2, wire::Mutation::Put { document }),
        (3, wire::Mutation::Delete { target }),
    ] {
        assert!(matches!(
            outcome(ready(adapter.execute(
                &request(
                    id,
                    wire::Call::Publish {
                        binding: BINDING.into(),
                        mutation
                    }
                ),
                &budget
            ))),
            wire::Outcome::Published {
                status: wire::Published::Confirmed
            }
        ));
    }
    let writes = fake.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].0, "put");
    assert_eq!(writes[0].2, fake.document.bytes);
    assert_eq!(writes[1].0, "delete");
    assert!(writes[1].2.is_empty());
    assert_eq!(
        fake.reads.load(Ordering::SeqCst),
        0,
        "no stamp/preparation read"
    );
}

#[test]
fn idle_next_deadlines_keep_the_interest_but_a_closed_probe_surfaces_unavailable() {
    for unavailable in [false, true] {
        let fake = Arc::new(Fake::new());
        fake.subscription
            .failures
            .lock()
            .unwrap()
            .push_back(SyncError::Deadline);
        if unavailable {
            fake.subscription
                .failures
                .lock()
                .unwrap()
                .push_back(SyncError::Unavailable)
        } else {
            fake.subscription.changed()
        }
        let stop = AtomicBool::new(false);
        let budget = io(&stop);
        let adapter = bound_adapter(fake.clone(), &budget);
        let subscribe = request(
            2,
            wire::Call::Subscribe {
                binding: BINDING.into(),
                interest: wire::Interest {
                    resource: "rows".into(),
                    scope: serde_json::from_value(
                        json!({"tenant":{"stringValue":"fixture-space"}}),
                    )
                    .unwrap(),
                },
            },
        );
        assert!(matches!(
            outcome(ready(adapter.execute(&subscribe, &budget))),
            wire::Outcome::Subscribed { .. }
        ));
        adapter.registered("2");
        let notice = adapter
            .next_notice(Instant::now() + Duration::from_secs(2))
            .unwrap();
        if unavailable {
            assert!(
                matches!(notice,wire::Frame::Notice{notice:wire::InterestNotice::Terminal{error},..} if error.code==wire::Code::Unavailable && error.state.is_none())
            )
        } else {
            assert!(matches!(
                notice,
                wire::Frame::Notice {
                    notice: wire::InterestNotice::Changed,
                    ..
                }
            ));
            assert_eq!(
                fake.subscription.closed.load(Ordering::SeqCst),
                0,
                "idle deadline does not close interest"
            )
        }
        adapter.close();
        assert_eq!(fake.subscription.closed.load(Ordering::SeqCst), 1);
    }
}
