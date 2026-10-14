//! Injected factory over the real shared channel; no native provider wiring.
use super::*;
use crate::sealed_sync::SealedSyncPort;
use tmt_extension_objects::{ChannelFrame, sync as wire};
pub(super) static NOTES: [Extension; 1] = [declared("notes", ObjectDeclaration::Local)];
#[test]
fn a_selected_legacy_version_is_a_typed_native_activation_refusal() {
    use std::io::{Read, Write};
    let env = Env::new();
    let service = env.service(&NOTES, bounds()).unwrap();
    let mounts = env.mounts(&NOTES);
    let path = env.directory("notes").join("door.sock");
    let listener = UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let peer = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        let head = String::from_utf8(head).unwrap();
        assert!(head.contains("tmt-object-channel: 3\r\n"));
        let generation = head
            .lines()
            .find_map(|line| line.strip_prefix("tmt-object-generation: "))
            .unwrap();
        write!(stream, "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: tmt-object-channel-v1\r\ntmt-object-channel: 2\r\ntmt-object-generation: {generation}\r\n\r\n").unwrap();
        let mut rest = Vec::new();
        stream.read_to_end(&mut rest).unwrap();
        assert!(
            rest.is_empty(),
            "no frame or fallback on the refused candidate"
        );
    });
    service
        .register_native(
            "notes",
            registration(
                Arc::new(super::super::sync::tests::Fake::new()),
                Arc::new(AtomicUsize::new(0)),
            ),
        )
        .unwrap();
    let hook = service.reactivation(Arc::new(AtomicBool::new(false)));
    service.with_reactivation(&hook, &mounts, || {
        assert_eq!(
            hook.activate_native("notes", Instant::now() + Duration::from_secs(5))
                .unwrap_err()
                .code,
            "REMOTE_CONTROL_UNSUPPORTED"
        );
    });
    peer.join().unwrap();
    assert!(service.active("notes").is_none());
}
fn send(peer: &Peer, id: u64, call: wire::Call) {
    let buses = peer.buses.lock().unwrap();
    let bus = buses.last().unwrap();
    bus.send_channel(&ChannelFrame::Sync(wire::Frame::Request {
        version: 1,
        generation: bus.generation().to_string(),
        request_id: id.to_string(),
        call,
    }))
    .unwrap();
}
pub(super) fn outcome(peer: &Peer) -> wire::Outcome {
    let buses = peer.buses.lock().unwrap();
    match buses
        .last()
        .unwrap()
        .recv_channel_stamped(Some(Instant::now() + Duration::from_secs(5)))
        .unwrap()
        .frame
    {
        ChannelFrame::Sync(wire::Frame::Result { outcome, .. }) => outcome,
        _ => panic!("expected sync result"),
    }
}
#[test]
fn native_shutdown_joins_idle_provider_interests_before_releasing_the_channel() {
    let env = Env::new();
    let service = env.service(&NOTES, bounds()).unwrap();
    let mounts = env.mounts(&NOTES);
    let peer = Peer::start(&env, "notes", Answer::Expect);
    let fake = Arc::new(super::super::sync::tests::Fake::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let registration = registration(fake.clone(), calls.clone());
    service.register_native("notes", registration).unwrap();
    service
        .activate_native_until(&mounts, "notes", Instant::now() + Duration::from_secs(5))
        .expect("registered v3 must activate");
    peer.ready.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(peer.buses.lock().unwrap()[0].supports_sync());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "activation is not a provider operation"
    );
    let ctx = fake.context();
    send(
        &peer,
        1,
        wire::Call::Bind {
            project: ctx.identity().project.clone(),
            extension: ctx.extension().into(),
            declaration_digest: ctx.declaration_digest().into(),
            space: ctx.space().into(),
        },
    );
    let binding = match outcome(&peer) {
        wire::Outcome::Binding { context } => context.id,
        _ => panic!("bind"),
    };
    send(
        &peer,
        2,
        wire::Call::Subscribe {
            binding,
            interest: wire::Interest {
                resource: "rows".into(),
                scope: serde_json::from_value(
                    serde_json::json!({"tenant":{"stringValue":"fixture-space"}}),
                )
                .unwrap(),
            },
        },
    );
    assert!(matches!(outcome(&peer), wire::Outcome::Subscribed { .. }));
    // The same v3 bus still serves ordinary object admission and config.
    super::request(
        &peer,
        3,
        Origin::LocalExtension,
        Call::Config(config_input()),
    );
    let acquire = callback(&peer);
    decide(&peer, acquire.callback_id.get(), 3, Decision::Allow);
    let disclose = callback(&peer);
    decide(&peer, disclose.callback_id.get(), 3, Decision::Allow);
    assert_eq!(
        result(&peer).outcome,
        Outcome::Success(Success::Config(local_config()))
    );
    service.shutdown();
    assert_eq!(
        fake.subscription.closed.load(Ordering::SeqCst),
        1,
        "provider interest must be closed before shutdown returns"
    );
    assert!(peer.saw_close(0));
}
pub(super) fn registration(
    fake: Arc<super::super::sync::tests::Fake>,
    calls: Arc<AtomicUsize>,
) -> super::super::sync::Registration {
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/sealed_sync/codec-v1.json"
    ))
    .unwrap();
    super::super::sync::Registration::new(
        fake.context().config().clone(),
        vectors["declaration"].as_str().unwrap().as_bytes().to_vec(),
        vectors["declarationDigest"].as_str().unwrap().into(),
        Arc::new(NativeAuth),
        Arc::new(move |input| {
            assert_eq!(input.space, "fixture-space");
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(fake.clone() as Arc<dyn SealedSyncPort>)
        }),
    )
    .unwrap()
}
pub(super) fn bind(peer: &Peer, fake: &super::super::sync::tests::Fake) -> String {
    let ctx = fake.context();
    send(
        peer,
        1,
        wire::Call::Bind {
            project: ctx.identity().project.clone(),
            extension: ctx.extension().into(),
            declaration_digest: ctx.declaration_digest().into(),
            space: ctx.space().into(),
        },
    );
    match outcome(peer) {
        wire::Outcome::Binding { context } => context.id,
        _ => panic!("bind"),
    }
}
pub(super) fn subscribe(peer: &Peer, binding: String) {
    send(
        peer,
        2,
        wire::Call::Subscribe {
            binding,
            interest: wire::Interest {
                resource: "rows".into(),
                scope: serde_json::from_value(
                    serde_json::json!({"tenant":{"stringValue":"fixture-space"}}),
                )
                .unwrap(),
            },
        },
    );
}
#[test]
fn an_unannounced_interest_is_joined_before_a_spent_register_result_is_refused() {
    let env = Env::new();
    let service = env
        .service(
            &NOTES,
            ServiceBounds {
                request: Duration::from_millis(100),
                ..bounds()
            },
        )
        .unwrap();
    let mounts = env.mounts(&NOTES);
    let peer = Peer::start(&env, "notes", Answer::Expect);
    let fake = Arc::new(super::super::sync::tests::Fake::new());
    service
        .register_native(
            "notes",
            registration(fake.clone(), Arc::new(AtomicUsize::new(0))),
        )
        .unwrap();
    let (reached, arrived) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let held = Mutex::new(held);
    let count = AtomicUsize::new(0);
    service.set_hook(Some(Arc::new(move |pause, deadline| {
        if pause == Pause::BeforeResultWrite && count.fetch_add(1, Ordering::SeqCst) == 1 {
            reached.send(deadline).unwrap();
            held.lock().unwrap().recv().unwrap();
        }
    })));
    service
        .activate_native_until(&mounts, "notes", Instant::now() + Duration::from_secs(5))
        .unwrap();
    peer.ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let binding = bind(&peer, fake.as_ref());
    subscribe(&peer, binding);
    let deadline = arrived.recv_timeout(Duration::from_secs(5)).unwrap();
    let (_unused, timer) = mpsc::channel::<()>();
    assert_eq!(
        timer.recv_timeout(deadline.saturating_duration_since(Instant::now())),
        Err(mpsc::RecvTimeoutError::Timeout)
    );
    release.send(()).unwrap();
    assert!(
        matches!(outcome(&peer),wire::Outcome::Error{error} if error.state==Some(wire::State::Deadline))
    );
    assert_eq!(
        fake.subscription.closed.load(Ordering::SeqCst),
        1,
        "unannounced provider interest must not survive the refusal"
    );
}
struct NativeAuth;
impl crate::sealed_sync::AuthPort for NativeAuth {
    fn identity(&self) -> crate::sealed_sync::AuthIdentity {
        crate::sealed_sync::AuthIdentity {
            project: "demo-remote-1".into(),
            uid: "fixture-user".into(),
        }
    }
    fn token<'a>(
        &'a self,
        _: &'a IoBudget<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<crate::sealed_sync::AuthToken, crate::sealed_sync::AuthFailure>,
                > + Send
                + 'a,
        >,
    > {
        panic!("carrier factory must not acquire credentials")
    }
}
#[test]
fn no_factory_refuses_native_activation_without_touching_a_legacy_channel() {
    let env = Env::new();
    let service = env.service(&EXTENSIONS, bounds()).unwrap();
    let mounts = env.mounts(&EXTENSIONS);
    let peer = Peer::start(&env, "colab", Answer::Expect);
    let legacy = service.activate(&mounts, "colab").unwrap();
    peer.ready.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        service.activate_native_until(&mounts, "colab", Instant::now() + Duration::from_secs(5)),
        Err(ActivateError::Unavailable)
    );
    assert_eq!(service.active("colab"), Some(legacy));
    assert_eq!(peer.connections(), 1);
    assert!(!peer.buses.lock().unwrap()[0].supports_sync());
}
