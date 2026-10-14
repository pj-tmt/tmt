//! The worker against a scripted Remote on the real bus: activation, binding, publication of
//! the active page and the bounds on retrying.
use super::{
    Activation, ActivationFailure, ActivePage, Channels, CloudSync, PageRef, Parts, Refusal,
    fixture::{BINDING, DIGEST, OneUpdate, SPACE, ScriptedRemote, activated, friendly, peer},
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
    activity: ActivePage,
    reports: Arc<Mutex<Vec<String>>>,
    activations: Arc<AtomicUsize>,
}
fn start(
    channels: Arc<super::fixture::FixedChannel>,
    activate: impl Fn() -> Result<Activation, ActivationFailure> + Send + Sync + 'static,
) -> Rig {
    let activity = ActivePage::default();
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
        local: Arc::new(move |_| Some(Box::new(OneUpdate { next: 1 }))),
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
fn the_worker_binds_then_publishes_the_active_page_once_per_touch() {
    let mut peer = peer();
    let firestore = super::fixture::Firestore::default();
    let answer = firestore.clone();
    let remote = ScriptedRemote::serve(peer.remote.take().unwrap(), move |call, id| {
        answer.answer(call, id)
    });
    let mut rig = start(Arc::clone(&peer.channels), activated);
    until("bind", || remote.methods().first() == Some(&Method::Bind));
    // Nothing is published before a page is touched.
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(remote.methods(), [Method::Bind]);
    rig.activity.touch(page(FIRST, 1));
    until("first pass", || {
        remote.methods() == [Method::Bind, Method::Publish]
    });
    // The page is current: waking without a new touch sends nothing more.
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(remote.methods().len(), 2);
    rig.sync.take().unwrap().stop();
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
    let sync::Call::Publish { binding, .. } = &calls[1] else {
        panic!("publish second")
    };
    assert_eq!(binding, BINDING);
    assert!(rig.reports.lock().unwrap().is_empty());
    assert!(
        peer.channel.active(),
        "stopping the worker keeps the channel"
    );
}

#[test]
fn a_page_row_refused_by_rules_is_reported_once_and_stops_that_pages_pass() {
    let mut peer = peer();
    let remote = ScriptedRemote::serve(peer.remote.take().unwrap(), |call, id| match call {
        sync::Call::Publish { .. } => sync::Outcome::Error {
            error: sync::Error {
                code: sync::Code::Denied,
                state: None,
                reset_at_ms: None,
                retry_after_ms: None,
            },
        },
        other => friendly(other, id),
    });
    let rig = start(Arc::clone(&peer.channels), activated);
    until("bind", || remote.methods().first() == Some(&Method::Bind));
    rig.activity.touch(page(FIRST, 1));
    until("one report", || rig.reports.lock().unwrap().len() == 1);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        remote
            .methods()
            .iter()
            .filter(|m| **m == Method::Publish)
            .count(),
        1,
        "a stopped page is not retried"
    );
    assert_eq!(rig.reports.lock().unwrap().len(), 1);
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
