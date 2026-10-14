//! Test peers for the cloud-sync worker: a scripted Remote on the other end of the real bus.
use crate::object_channel::tests::{GENERATION, limit, pair_version};
use crate::{
    SealedClient,
    cloud_sync::{Activation, ActivationFailure, Channels},
    object_channel::ObjectChannel,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tmt_extension_objects::{Bus, ChannelFrame, Uuid4, sync};

pub(crate) const PROJECT: &str = "fixture";
pub(crate) const SPACE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub(crate) const UID: &str = "owner";
pub(crate) const BINDING: &str = "22222222-2222-4222-8222-222222222222";
pub(crate) const DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// The one channel generation of a test, shown to the worker the way the serve shows its own.
pub(crate) struct FixedChannel {
    pub client: Mutex<Option<SealedClient>>,
    pub started: AtomicU64,
}
impl Channels for FixedChannel {
    fn client(&self) -> Option<SealedClient> {
        self.client.lock().unwrap().clone()
    }
    fn started(&self) -> u64 {
        self.started.load(Ordering::Acquire)
    }
}

/// A Remote that answers each sync request from a script and records what it was asked.
pub(crate) struct ScriptedRemote {
    pub calls: Arc<Mutex<Vec<sync::Call>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl ScriptedRemote {
    pub(crate) fn serve(
        remote: Bus,
        mut answer: impl FnMut(&sync::Call, &str) -> sync::Outcome + Send + 'static,
    ) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (record, flag) = (Arc::clone(&calls), Arc::clone(&stop));
        let thread = thread::spawn(move || {
            while !flag.load(Ordering::Acquire) {
                let Ok(stamped) = remote.recv_channel_stamped(Some(
                    std::time::Instant::now() + Duration::from_millis(50),
                )) else {
                    continue;
                };
                let ChannelFrame::Sync(sync::Frame::Request {
                    request_id,
                    generation,
                    call,
                    ..
                }) = stamped.frame
                else {
                    continue;
                };
                let outcome = answer(&call, &request_id);
                record.lock().unwrap().push(call.clone());
                let _ = remote.send_channel(&ChannelFrame::Sync(sync::Frame::Result {
                    version: 1,
                    generation,
                    method: call.method(),
                    outcome,
                    request_id,
                }));
            }
        });
        Self {
            calls,
            stop,
            thread: Some(thread),
        }
    }
    pub(crate) fn methods(&self) -> Vec<sync::Method> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.method())
            .collect()
    }
}
impl Drop for ScriptedRemote {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The scripted answer a well-behaved Remote gives to bind, subscribe and cancel.
pub(crate) fn friendly(call: &sync::Call, request_id: &str) -> sync::Outcome {
    match call {
        sync::Call::Bind { .. } => sync::Outcome::Binding {
            context: sync::BoundContext {
                id: BINDING.into(),
                project: PROJECT.into(),
                extension: "colab".into(),
                declaration_digest: DIGEST.into(),
                space: SPACE.into(),
                uid: UID.into(),
            },
        },
        sync::Call::Subscribe { .. } => sync::Outcome::Subscribed {
            id: request_id.into(),
        },
        sync::Call::Cancel {
            subscription_id, ..
        } => sync::Outcome::Cancelled {
            id: subscription_id.clone(),
        },
        _ => panic!("unscripted request {call:?}"),
    }
}

pub(crate) struct Peer {
    pub channel: ObjectChannel,
    pub channels: Arc<FixedChannel>,
    pub remote: Option<Bus>,
}
pub(crate) fn peer() -> Peer {
    let (channel, remote) = pair_version(true);
    let channels = Arc::new(FixedChannel {
        client: Mutex::new(Some(channel.client())),
        started: AtomicU64::new(1),
    });
    Peer {
        channel,
        channels,
        remote: Some(remote),
    }
}
/// The activation Remote's hidden control exchange would return for this peer's generation.
pub(crate) fn activated() -> Result<Activation, ActivationFailure> {
    Ok(Activation {
        project: PROJECT.into(),
        generation: Uuid4::parse(GENERATION).unwrap(),
    })
}
pub(crate) fn within() -> std::time::Instant {
    limit()
}
