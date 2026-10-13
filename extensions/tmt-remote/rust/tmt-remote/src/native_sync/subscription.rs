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
    sealed_sync::{Change, Pending, Subscription, SyncError, SyncInterest, validate_interest},
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
        last_count: None,
        last_notice_ms: None,
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
    last_count: Option<u64>,
    last_notice_ms: Option<u64>,
    terminal: bool,
    registered: bool,
}
impl NativeSubscription {
    async fn poll(&self, io: &IoBudget<'_>) -> Result<u64, SyncError> {
        let resource = self.consumer.resource(&self.interest.resource)?;
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
        let body = json!({"structuredAggregationQuery":{
            "structuredQuery":{
                "from":[{"collectionId":collection}],
                "where":{"compositeFilter":{"op":"AND","filters":filters}}
            },
            "aggregations":[{"count":{},"alias":"count"}]
        }});
        let response = self
            .consumer
            .send(
                Operation::Query,
                format!("https://firestore.googleapis.com/v1/{path}:runAggregationQuery"),
                body.to_string().into_bytes(),
                1 + READ_LOOKUPS,
                io,
            )
            .await?
            .ok_or(SyncError::Unavailable)?;
        super::client::status(response.status)?;
        if response.body.len() > limits::NATIVE_SYNC_PAGE_BYTES {
            return Err(SyncError::CatchUpRequired);
        }
        let reply = wire::strict_json(&response.body).ok_or(SyncError::Unavailable)?;
        let mut count = None;
        for row in reply.as_array().ok_or(SyncError::Unavailable)? {
            let Some(result) = row.get("result") else {
                if !row.get("readTime").is_some_and(Value::is_string) {
                    return Err(SyncError::Unavailable);
                }
                continue;
            };
            let fields = result
                .get("aggregateFields")
                .and_then(Value::as_object)
                .ok_or(SyncError::Unavailable)?;
            let value = fields
                .get("count")
                .and_then(Value::as_object)
                .ok_or(SyncError::Unavailable)?;
            let decimal = value
                .get("integerValue")
                .and_then(Value::as_str)
                .ok_or(SyncError::Unavailable)?;
            let total = decimal.parse::<u64>().map_err(|_| SyncError::Unavailable)?;
            if count.is_some()
                || fields.len() != 1
                || value.len() != 1
                || total > i64::MAX as u64
                || total.to_string() != decimal
            {
                return Err(SyncError::Unavailable);
            }
            count = Some(total);
        }
        self.consumer.check(io)?;
        count.ok_or(SyncError::Unavailable)
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
                let result = self.poll(io).await;
                let now = self.consumer.runtime.clock.now().monotonic_ms;
                match result {
                    Ok(count) => {
                        self.cadence.succeeded();
                        let changed = self.last_count != Some(count);
                        let heartbeat = self.last_notice_ms.is_none_or(|last| {
                            now.saturating_sub(last) >= limits::NATIVE_SYNC_HEARTBEAT_MS
                        });
                        self.last_count = Some(count);
                        if changed {
                            self.cadence.activity(now);
                        }
                        self.next_ms =
                            now.saturating_add(self.cadence.delay_ms(now, |max| {
                                self.consumer.runtime.jitter.milliseconds(max)
                            }));
                        if changed || heartbeat {
                            self.last_notice_ms = Some(now);
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
