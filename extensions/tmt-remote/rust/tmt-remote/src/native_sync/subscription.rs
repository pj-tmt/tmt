//! Page-level polling hints. No stream positions, document queue or delivery progress.
use super::{
    PollCadence,
    client::{CallCancellation, Consumer},
};
use crate::{
    firebase_auth::Operation,
    firestore_budget::READ_LOOKUPS,
    limits,
    objects::IoBudget,
    sealed_sync::{
        Change, DiscoveryQuery, Pending, Subscription, SyncError, SyncInterest, validate_interest,
    },
    wire,
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Instant,
};

pub(super) async fn create_subscription(
    consumer: Arc<Consumer>,
    interest: &SyncInterest,
    io: &IoBudget<'_>,
) -> Result<Box<dyn Subscription>, SyncError> {
    consumer.check(io)?;
    validate_interest(&consumer.context, interest)?;
    // Validate the carrier-page bound before reserving an interest slot.
    consumer.limit(consumer.resource(&interest.resource)?)?;
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
    Ok(Box::new(NativeSubscription {
        interest: SyncInterest {
            resource: interest.resource.clone(),
            scope: interest.scope.clone(),
        },
        next_discovery_ms: consumer
            .runtime
            .clock
            .now()
            .monotonic_ms
            .saturating_add(limits::NATIVE_DISCOVERY_MS),
        consumer,
        cadence: PollCadence::default(),
        next_ms: 0,
        terminal: false,
        registered: true,
    }))
}

struct NativeSubscription {
    consumer: Arc<Consumer>,
    interest: SyncInterest,
    cadence: PollCadence,
    next_ms: u64,
    next_discovery_ms: u64,
    terminal: bool,
    registered: bool,
}
impl NativeSubscription {
    async fn discover(&self, io: &IoBudget<'_>) -> Result<bool, SyncError> {
        let spec = self
            .consumer
            .resource(&self.interest.resource)?
            .sync
            .as_ref()
            .and_then(|s| s.query.as_ref())
            .ok_or(SyncError::Unavailable)?;
        let Some(shape) = spec.discovery.as_ref() else {
            return Ok(false);
        };
        let mut request = DiscoveryQuery {
            resource: self.interest.resource.clone(),
            shape: shape.name.clone(),
            scope: self.interest.scope.clone(),
            continuation: None,
        };
        let mut records = 0usize;
        for _ in 0..limits::NATIVE_DISCOVERY_RUN_PAGES {
            let page = super::discovery::query(&self.consumer, &request, io).await?;
            records += page.documents.len();
            if records > limits::NATIVE_SYNC_PAGE_DOCUMENTS {
                return Err(SyncError::CatchUpRequired);
            }
            request.continuation = page.continuation;
            if request.continuation.is_none() {
                // A first-seq discovery row may stay unchanged while its stream grows.
                // Completion is a level-triggered wake, never a delivery acknowledgment.
                return Ok(true);
            }
        }
        Err(SyncError::CatchUpRequired)
    }
    async fn poll(&self, io: &IoBudget<'_>) -> Result<bool, SyncError> {
        let resource = self.consumer.resource(&self.interest.resource)?;
        let limit = self.consumer.limit(resource)?;
        let filters: Vec<Value> = self
            .interest
            .scope
            .iter()
            .map(|(key, value)| {
                json!({
                    "fieldFilter":{"field":{"fieldPath":key},"op":"EQUAL","value":value}
                })
            })
            .collect();
        let (parent, collection) = resource
            .path
            .rsplit_once('/')
            .unwrap_or(("", &resource.path));
        let path = format!(
            "{}/x/{}{}",
            self.consumer.root(),
            self.consumer.context.extension(),
            if parent.is_empty() {
                String::new()
            } else {
                format!("/{parent}")
            }
        );
        let body = json!({"structuredQuery":{
            "from":[{"collectionId":collection}],
            "where":{"compositeFilter":{"op":"AND","filters":filters}},
            "orderBy":[{"field":{"fieldPath":"__name__"},"direction":"ASCENDING"}],
            "limit":limit
        }});
        let response = self
            .consumer
            .send(
                Operation::Query,
                format!("https://firestore.googleapis.com/v1/{path}:runQuery"),
                body.to_string().into_bytes(),
                limit as u64 + READ_LOOKUPS,
                io,
            )
            .await?
            .ok_or(SyncError::Unavailable)?;
        super::client::status(response.status)?;
        if response.body.len() > limits::NATIVE_SYNC_PAGE_BYTES {
            return Err(SyncError::CatchUpRequired);
        }
        let reply = wire::strict_json(&response.body).ok_or(SyncError::Unavailable)?;
        let mut found = false;
        for row in reply.as_array().ok_or(SyncError::Unavailable)? {
            let Some(value) = row.get("document") else {
                if !row.get("readTime").is_some_and(Value::is_string) {
                    return Err(SyncError::Unavailable);
                }
                continue;
            };
            if found {
                return Err(SyncError::CatchUpRequired);
            }
            let document = self
                .consumer
                .parse_document(value, &self.interest.resource)?;
            let fields = crate::sealed_sync::validate_fields(&document.bytes)?;
            if self
                .interest
                .scope
                .iter()
                .any(|(key, value)| fields.get(key) != Some(value))
            {
                return Err(SyncError::Denied);
            }
            found = true;
        }
        self.consumer.check(io)?;
        // Only a successful bounded absence observation suppresses Changed.
        // A retained first row is insufficient evidence about later writes.
        Ok(found)
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
