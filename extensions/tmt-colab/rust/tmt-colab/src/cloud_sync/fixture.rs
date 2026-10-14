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

/// An in-memory Firestore for the publisher: create-only conflicts, whole-field puts, and
/// scripted effects that are lost, acknowledged-but-unseen, or refused.
#[derive(Default)]
pub(crate) struct Faults {
    /// The next create is stored, but its answer is lost: `Published::Unknown`.
    pub unknown_after_store: bool,
    /// The next create is not stored and its answer is lost.
    pub unknown_without_store: bool,
    pub refuse_next: Option<sync::Error>,
}
#[derive(Clone, Default)]
pub(crate) struct Firestore {
    pub docs: Arc<
        Mutex<
            std::collections::BTreeMap<
                (String, String),
                serde_json::Map<String, serde_json::Value>,
            >,
        >,
    >,
    pub faults: Arc<Mutex<Faults>>,
    /// Every publish and read in arrival order: (kind, resource, id).
    pub log: Arc<Mutex<Vec<(&'static str, String, String)>>>,
}
impl Firestore {
    pub(crate) fn answer(&self, call: &sync::Call, request_id: &str) -> sync::Outcome {
        match call {
            sync::Call::Read { target, .. } => {
                self.log
                    .lock()
                    .unwrap()
                    .push(("read", target.resource.clone(), target.id.clone()));
                if let Some(error) = self.faults.lock().unwrap().refuse_next.take() {
                    return sync::Outcome::Error { error };
                }
                let document = self
                    .docs
                    .lock()
                    .unwrap()
                    .get(&(target.resource.clone(), target.id.clone()))
                    .map(|fields| seal(&target.resource, &target.id, fields.clone()));
                sync::Outcome::Read { document }
            }
            sync::Call::Publish { mutation, .. } => {
                let (kind, document) = match mutation {
                    sync::Mutation::Create { document } => ("create", document),
                    sync::Mutation::Put { document } => ("put", document),
                    sync::Mutation::Delete { .. } => panic!("no delete"),
                };
                self.log.lock().unwrap().push((
                    kind,
                    document.target.resource.clone(),
                    document.target.id.clone(),
                ));
                let mut faults = self.faults.lock().unwrap();
                if let Some(error) = faults.refuse_next.take() {
                    return sync::Outcome::Error { error };
                }
                let key = (document.target.resource.clone(), document.target.id.clone());
                let mut docs = self.docs.lock().unwrap();
                if kind == "create" && faults.unknown_without_store {
                    faults.unknown_without_store = false;
                    return sync::Outcome::Published {
                        status: sync::Published::Unknown,
                    };
                }
                let status = match (kind, docs.get(&key)) {
                    ("create", Some(existing)) if *existing == document.fields => {
                        sync::Published::AlreadyPresent
                    }
                    ("create", Some(_)) => {
                        return sync::Outcome::Error {
                            error: sync::Error {
                                code: sync::Code::Unavailable,
                                state: Some(sync::State::Conflict),
                                reset_at_ms: None,
                                retry_after_ms: None,
                            },
                        };
                    }
                    _ => {
                        docs.insert(key, document.fields.clone());
                        sync::Published::Confirmed
                    }
                };
                if kind == "create" && faults.unknown_after_store {
                    faults.unknown_after_store = false;
                    return sync::Outcome::Published {
                        status: sync::Published::Unknown,
                    };
                }
                sync::Outcome::Published { status }
            }
            other => friendly(other, request_id),
        }
    }
}
pub(crate) fn document(resource: &str, id: &str, pairs: &[(&str, &str)]) -> sync::Document {
    let fields: serde_json::Map<String, serde_json::Value> = pairs
        .iter()
        .map(|(k, v)| ((*k).into(), serde_json::json!({"stringValue": v})))
        .collect();
    seal(resource, id, fields)
}
/// A document whose digest is the SHA-256 of its canonical fields, as the bus requires.
pub(crate) fn seal(
    resource: &str,
    id: &str,
    fields: serde_json::Map<String, serde_json::Value>,
) -> sync::Document {
    let digest = tmt_extension_objects::Sha256Hex::from_bytes(tmt_colab_model::crypto::digest(
        &serde_json::to_vec(&fields).unwrap(),
    ))
    .to_string();
    sync::Document {
        target: sync::Target {
            resource: resource.into(),
            id: id.into(),
        },
        fields,
        digest,
    }
}

/// A local side that has one unconfirmed update per touch of its page, so the worker's passes
/// are visible as documents the scripted Remote receives.
pub(crate) struct OneUpdate {
    pub next: u64,
    pub passes: Arc<AtomicU64>,
}
impl super::publisher::Local for OneUpdate {
    fn preamble(
        &mut self,
        _: &super::PageRef,
    ) -> Result<Vec<super::publisher::Row>, super::publisher::Fault> {
        self.passes.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }
    fn next(
        &mut self,
        _: &super::PageRef,
    ) -> Result<Option<super::publisher::Data>, super::publisher::Fault> {
        if self.next == 0 {
            return Ok(None);
        }
        Ok(Some(super::publisher::Data {
            documents: vec![document(
                "log",
                &format!("log-{}", self.next),
                &[("seq", "1")],
            )],
            confirm: super::publisher::Confirm::Update {
                stream: "w".into(),
                seq: self.next,
                hash: [0; 32],
            },
        }))
    }
    fn confirm(
        &mut self,
        _: &super::PageRef,
        _: &super::publisher::Confirm,
    ) -> Result<(), super::publisher::Fault> {
        self.next = 0;
        Ok(())
    }
}
