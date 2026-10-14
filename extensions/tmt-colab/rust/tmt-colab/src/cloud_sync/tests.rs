//! The worker against a scripted Remote on the real bus: activation, binding, the one active
//! page group and its joined shutdown.
use super::{
    Activation, ActivationFailure, Activity, Channels, CloudSync, PageRef, Parts, Refusal,
    fixture::{BINDING, DIGEST, SPACE, ScriptedRemote, activated, friendly, peer},
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tmt_extension_objects::sync::{self, Method};

const FIRST: &str = "10000000-0000-4000-8000-000000000001";
const SECOND: &str = "10000000-0000-4000-8000-000000000002";

fn until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn page(page: &str, epoch: u64) -> PageRef {
    PageRef {
        space: SPACE.into(),
        page: page.into(),
        epoch,
    }
}
struct Rig {
    sync: Option<CloudSync>,
    activity: Activity,
    reports: Arc<Mutex<Vec<String>>>,
    activations: Arc<AtomicUsize>,
}
fn start(
    channels: Arc<super::fixture::FixedChannel>,
    activate: impl Fn() -> Result<Activation, ActivationFailure> + Send + Sync + 'static,
) -> Rig {
    let activity = Activity::default();
    let reports = Arc::new(Mutex::new(Vec::new()));
    let activations = Arc::new(AtomicUsize::new(0));
    let (sink, count) = (Arc::clone(&reports), Arc::clone(&activations));
    let sync = CloudSync::start(Parts {
        channels,
        activity: activity.clone(),
        activate: Arc::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            activate()
        }),
        report: Arc::new(move |line| sink.lock().unwrap().push(line.to_owned())),
        declaration_digest: DIGEST.into(),
        space: SPACE.into(),
    })
    .unwrap();
    Rig {
        sync: Some(sync),
        activity,
        reports,
        activations,
    }
}

#[test]
fn the_worker_binds_follows_the_active_page_and_cancels_on_stop() {
    let mut peer = peer();
    let remote = ScriptedRemote::serve(peer.remote.take().unwrap(), friendly);
    let mut rig = start(Arc::clone(&peer.channels), activated);
    until("bind", || remote.methods() == [Method::Bind]);
    rig.activity.touch(page(FIRST, 1));
    until("first group", || remote.methods().len() == 4);
    rig.activity.touch(page(SECOND, 2));
    until("successor group", || remote.methods().len() == 10);
    rig.sync.take().unwrap().stop();
    let methods = remote.methods();
    use Method::{Bind, Cancel, Subscribe};
    assert_eq!(
        methods,
        [
            Bind, Subscribe, Subscribe, Subscribe, Cancel, Cancel, Cancel, Subscribe, Subscribe,
            Subscribe, Cancel, Cancel, Cancel
        ]
    );
    let calls = remote.calls.lock().unwrap();
    let sync::Call::Bind {
        project,
        extension,
        declaration_digest,
        space,
    } = &calls[0]
    else {
        panic!("bind first")
    };
    assert_eq!(
        (
            project.as_str(),
            extension.as_str(),
            declaration_digest.as_str(),
            space.as_str()
        ),
        ("fixture", "colab", DIGEST, SPACE)
    );
    for call in &calls[1..] {
        match call {
            sync::Call::Subscribe { binding, .. } | sync::Call::Cancel { binding, .. } => {
                assert_eq!(binding, BINDING)
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(rig.reports.lock().unwrap().is_empty());
    assert!(
        peer.channel.active(),
        "stopping the worker keeps the channel"
    );
}

#[test]
fn a_native_unavailable_verdict_is_shown_once_and_activation_is_never_retried() {
    let peer = peer();
    let rig = start(Arc::clone(&peer.channels), || {
        Err(ActivationFailure::Remote(Refusal {
            code: "REMOTE_STATE_UNAVAILABLE".into(),
            message: "Cloud sync is not set up.".into(),
            state: None,
        }))
    });
    until("one report", || rig.reports.lock().unwrap().len() == 1);
    // A new channel, a nudge and several ticks are all triggers; none activates again.
    peer.channels.started.fetch_add(1, Ordering::SeqCst);
    rig.activity.nudge();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(rig.activations.load(Ordering::SeqCst), 1);
    assert_eq!(*rig.reports.lock().unwrap(), ["Cloud sync is not set up."]);
}

#[test]
fn a_deadline_or_missing_door_retries_a_bounded_number_of_times_without_a_warning() {
    let peer = peer();
    let rig = start(Arc::clone(&peer.channels), || Err(ActivationFailure::Local));
    until("three attempts", || {
        rig.activations.load(Ordering::SeqCst) == 3
    });
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        rig.activations.load(Ordering::SeqCst),
        3,
        "the budget is bounded"
    );
    assert!(rig.reports.lock().unwrap().is_empty());
    // A channel nobody activated is a fresh trigger with a fresh budget.
    peer.channels.started.fetch_add(1, Ordering::SeqCst);
    until("a fresh attempt", || {
        rig.activations.load(Ordering::SeqCst) > 3
    });
}

#[test]
fn a_bind_that_disagrees_with_the_frozen_inputs_is_refused_and_ends_the_generation() {
    let mut peer = peer();
    let remote = ScriptedRemote::serve(peer.remote.take().unwrap(), |call, id| {
        match friendly(call, id) {
            sync::Outcome::Binding { mut context } => {
                context.space = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
                sync::Outcome::Binding { context }
            }
            other => other,
        }
    });
    let rig = start(Arc::clone(&peer.channels), activated);
    until("bind attempted", || remote.methods() == [Method::Bind]);
    rig.activity.touch(page(FIRST, 1));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        remote.methods(),
        [Method::Bind],
        "no interest is registered on a refused bind"
    );
    assert!(
        peer.channels
            .client()
            .unwrap()
            .sealed_request(
                sync::Call::Cancel {
                    binding: BINDING.into(),
                    subscription_id: "1".into()
                },
                super::fixture::within()
            )
            .is_err(),
        "a disagreeing bind ends the generation"
    );
}
