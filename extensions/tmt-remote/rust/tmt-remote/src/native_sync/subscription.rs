//! Bounded polling interests and coalesced wake notices. Colab owns cursors.
use super::{
    PollCadence,
    client::{CallCancellation, Consumer},
};
use crate::{
    deploy_plan::sha256_hex,
    limits,
    objects::IoBudget,
    sealed_sync::{Change, Pending, Query, Subscription, SyncCursor, SyncError, validate_query},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, atomic::Ordering},
    time::Instant,
};

pub(super) async fn create_subscription(
    consumer: Arc<Consumer>,
    query: &Query,
    io: &IoBudget<'_>,
) -> Result<Box<dyn Subscription>, SyncError> {
    consumer.check(io)?;
    validate_query(&consumer.context, query)?;
    let mut count = consumer
        .subscriptions
        .lock()
        .map_err(|_| SyncError::Unavailable)?;
    let mut caps = consumer
        .runtime
        .caps
        .lock()
        .map_err(|_| SyncError::Unavailable)?;
    consumer.check(io)?;
    if *count >= limits::NATIVE_SYNC_CONSUMER_SUBSCRIPTIONS
        || caps.subscriptions >= limits::NATIVE_SYNC_SERVE_SUBSCRIPTIONS
    {
        return Err(SyncError::RateLimited);
    }
    *count += 1;
    caps.subscriptions += 1;
    drop(caps);
    drop(count);
    let spec = consumer
        .resource(&query.resource)?
        .sync
        .as_ref()
        .and_then(|s| s.query.as_ref())
        .ok_or(SyncError::Unavailable)?;
    let interest = Arc::new(Mutex::new(Interest {
        resource: query.resource.clone(),
        scope: query
            .scope
            .iter()
            .filter(|(key, _)| *key != &spec.stream_field)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        cursors: BTreeMap::from([(
            query.stream.clone(),
            query.cursor.as_ref().map_or(0, |c| c.seq),
        )]),
        catch_up: false,
        closed: false,
    }));
    let subscription = NativeSubscription {
        consumer: consumer.clone(),
        interest: interest.clone(),
        query: Query {
            resource: query.resource.clone(),
            scope: query.scope.clone(),
            stream: query.stream.clone(),
            cursor: query.cursor.clone(),
        },
        cadence: PollCadence::default(),
        samples: BTreeMap::new(),
        discovered: BTreeMap::new(),
        next_ms: 0,
        next_discovery_ms: consumer
            .runtime
            .clock
            .now()
            .monotonic_ms
            .saturating_add(limits::NATIVE_DISCOVERY_MS),
        poll_index: 0,
        terminal: false,
        registered: true,
    };
    {
        let mut interests = consumer
            .interests
            .lock()
            .map_err(|_| SyncError::Unavailable)?;
        interests.retain(|i| {
            i.upgrade()
                .is_some_and(|i| i.lock().is_ok_and(|i| !i.closed))
        });
        interests.push(Arc::downgrade(&interest));
    }
    Ok(Box::new(subscription) as Box<dyn Subscription>)
}

pub(super) struct Interest {
    pub(super) resource: String,
    pub(super) scope: BTreeMap<String, Value>,
    pub(super) cursors: BTreeMap<String, u64>,
    pub(super) catch_up: bool,
    pub(super) closed: bool,
}
struct NativeSubscription {
    consumer: Arc<Consumer>,
    interest: Arc<Mutex<Interest>>,
    query: Query,
    cadence: PollCadence,
    samples: BTreeMap<String, String>,
    discovered: BTreeMap<String, String>,
    next_ms: u64,
    next_discovery_ms: u64,
    poll_index: usize,
    terminal: bool,
    registered: bool,
}
impl NativeSubscription {
    async fn discover(&mut self, io: &IoBudget<'_>) -> Result<bool, SyncError> {
        use crate::sealed_sync::DiscoveryQuery;
        let spec = self
            .consumer
            .resource(&self.query.resource)?
            .sync
            .as_ref()
            .and_then(|s| s.query.as_ref())
            .ok_or(SyncError::Unavailable)?;
        let Some(shape) = spec.discovery.as_ref() else {
            return Ok(false);
        };
        let stream_field = spec.stream_field.clone();
        let mut request = DiscoveryQuery {
            resource: self.query.resource.clone(),
            shape: shape.name.clone(),
            scope: self
                .interest
                .lock()
                .map_err(|_| SyncError::Unavailable)?
                .scope
                .clone(),
            continuation: None,
        };
        let mut discovered: BTreeMap<String, String> = BTreeMap::new();
        for _ in 0..limits::NATIVE_DISCOVERY_RUN_PAGES {
            let page = super::discovery::query(&self.consumer, &request, io).await?;
            for document in page.documents {
                let fields = crate::sealed_sync::validate_fields(&document.bytes)?;
                let stream = fields[&stream_field]["stringValue"]
                    .as_str()
                    .ok_or(SyncError::Unavailable)?;
                if !discovered.contains_key(stream)
                    && discovered.len() >= limits::NATIVE_SYNC_PAGE_DOCUMENTS
                {
                    return Err(SyncError::CatchUpRequired);
                }
                let previous = discovered.entry(stream.into()).or_default();
                *previous = sha256_hex(format!("{previous}{}", document.digest).as_bytes());
            }
            request.continuation = page.continuation;
            if request.continuation.is_none() {
                if self.discovered.keys().any(|s| !discovered.contains_key(s)) {
                    return Err(SyncError::CatchUpRequired);
                }
                let mut interest = self.interest.lock().map_err(|_| SyncError::Unavailable)?;
                for stream in discovered.keys() {
                    if !interest.cursors.contains_key(stream)
                        && interest.cursors.len() >= limits::NATIVE_SYNC_PAGE_DOCUMENTS
                    {
                        return Err(SyncError::CatchUpRequired);
                    }
                    interest.cursors.entry(stream.clone()).or_default();
                }
                let changed = self.discovered != discovered;
                self.discovered = discovered;
                return Ok(changed);
            }
        }
        Err(SyncError::CatchUpRequired)
    }
    async fn poll(&mut self, io: &IoBudget<'_>) -> Result<bool, SyncError> {
        let spec = self
            .consumer
            .resource(&self.query.resource)?
            .sync
            .as_ref()
            .and_then(|s| s.query.as_ref())
            .ok_or(SyncError::Unavailable)?;
        let stream_field = spec.stream_field.clone();
        let streams: Vec<_> = {
            let interest = self.interest.lock().map_err(|_| SyncError::Unavailable)?;
            if interest.catch_up {
                return Err(SyncError::CatchUpRequired);
            }
            interest
                .cursors
                .iter()
                .map(|(s, c)| (s.clone(), *c))
                .collect()
        };
        for _ in 0..streams.len() {
            let (stream, seq) = &streams[self.poll_index % streams.len()];
            self.poll_index = self.poll_index.wrapping_add(1);
            let mut scope = self.query.scope.clone();
            scope.insert(stream_field.clone(), json!({"stringValue":stream}));
            let query = Query {
                resource: self.query.resource.clone(),
                scope,
                stream: stream.clone(),
                cursor: Some(SyncCursor {
                    stream: stream.clone(),
                    seq: *seq,
                }),
            };
            let page = self.consumer.query(&query, io).await?;
            let fingerprint = sha256_hex(
                &page
                    .documents
                    .iter()
                    .flat_map(|d| d.digest.as_bytes().iter().copied())
                    .collect::<Vec<_>>(),
            );
            let previous = self.samples.insert(stream.clone(), fingerprint.clone());
            let changed = previous.map_or(!page.documents.is_empty(), |previous| {
                previous != fingerprint
            });
            // A public query may have advanced Colab's cursor during this probe;
            // preserve a wake hint and resample that new caller position next time.
            let raced = self
                .interest
                .lock()
                .map_err(|_| SyncError::Unavailable)?
                .cursors
                .get(stream)
                != Some(seq);
            if changed || raced {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
impl Subscription for NativeSubscription {
    fn next<'a>(&'a mut self, io: &'a IoBudget<'a>) -> Pending<'a, Change> {
        Box::pin(async move {
            if self.terminal {
                return Err(SyncError::Unavailable);
            }
            self.consumer.check(io)?;
            let remaining = u64::try_from(
                io.deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis(),
            )
            .unwrap_or(u64::MAX);
            let stop_at = self
                .consumer
                .runtime
                .clock
                .now()
                .monotonic_ms
                .saturating_add(remaining);
            loop {
                let now = self.consumer.runtime.clock.now().monotonic_ms;
                let activity = self.consumer.activity.load(Ordering::SeqCst);
                if activity > 0 {
                    self.cadence.activity(activity - 1);
                }
                if now >= stop_at {
                    return Err(SyncError::Deadline);
                }
                let cancel = CallCancellation {
                    caller: io.cancelled,
                    closed: &self.consumer.closed,
                };
                if self.next_ms > now
                    && !self
                        .consumer
                        .runtime
                        .clock
                        .wait_until(self.next_ms.min(stop_at), &cancel)
                {
                    return Err(SyncError::Cancelled);
                }
                self.consumer.check(io)?;
                let now = self.consumer.runtime.clock.now().monotonic_ms;
                if now >= stop_at {
                    return Err(SyncError::Deadline);
                }
                let due = now >= self.next_discovery_ms;
                let changed = if due {
                    match self.discover(io).await {
                        Ok(changed) => {
                            self.next_discovery_ms = self
                                .consumer
                                .runtime
                                .clock
                                .now()
                                .monotonic_ms
                                .saturating_add(limits::NATIVE_DISCOVERY_MS);
                            Ok(changed)
                        }
                        Err(error) => Err(error),
                    }
                } else {
                    Ok(false)
                };
                let result = match changed {
                    Ok(true) => Ok(true),
                    Ok(false) => self.poll(io).await,
                    Err(error) => Err(error),
                };
                let now = self.consumer.runtime.clock.now().monotonic_ms;
                match result {
                    Ok(changed) => {
                        self.cadence.succeeded();
                        if changed {
                            self.cadence.activity(now);
                        }
                        self.next_ms =
                            now.saturating_add(self.cadence.delay_ms(now, |max| {
                                self.consumer.runtime.jitter.milliseconds(max)
                            }));
                        if changed {
                            return Ok(Change::Changed);
                        }
                    }
                    Err(SyncError::CatchUpRequired) => {
                        self.close();
                        return Ok(Change::CatchUpRequired);
                    }
                    Err(SyncError::Unavailable | SyncError::RateLimited) => {
                        self.cadence.failed();
                        self.next_ms = now
                            .saturating_add(self.cadence.delay_ms(now, |max| {
                                self.consumer.runtime.jitter.milliseconds(max)
                            }))
                            .max(self.consumer.retry_after_ms.load(Ordering::SeqCst));
                        return Err(SyncError::Unavailable);
                    }
                    Err(error) => {
                        self.close();
                        return Err(error);
                    }
                }
            }
        })
    }
    fn close(&mut self) {
        self.terminal = true;
        if let Ok(mut interest) = self.interest.lock() {
            interest.closed = true;
        }
        self.samples.clear();
        self.discovered.clear();
        if self.registered {
            self.registered = false;
            if let Ok(mut count) = self.consumer.subscriptions.lock()
                && *count > 0
            {
                *count -= 1;
                if let Ok(mut caps) = self.consumer.runtime.caps.lock() {
                    caps.subscriptions = caps.subscriptions.saturating_sub(1);
                }
            }
        }
    }
}
impl Drop for NativeSubscription {
    fn drop(&mut self) {
        self.close();
    }
}
