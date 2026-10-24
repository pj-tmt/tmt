//! One finite machine reconciliation. The serving lifecycle owns scheduling/cancel-and-join.
use super::{Receiver, provider::Provider};
use crate::{error::RemoteError, objects::IoBudget, session::DoorSessions};
use std::sync::Arc;
pub struct InboxDriver {
    receiver: Receiver,
    sessions: Arc<DoorSessions>,
    provider: Provider,
}
impl InboxDriver {
    pub fn new(receiver: Receiver, sessions: Arc<DoorSessions>, provider: Provider) -> Self {
        Self {
            receiver,
            sessions,
            provider,
        }
    }
    /// Run only on the serving owner's joined worker. Transient failures back off without
    /// advancing; cancellation ends the worker and a new owner resumes after restart.
    pub async fn run(&self, io: &IoBudget<'_>) -> Result<(), RemoteError> {
        let mut cadence = crate::native_sync::PollCadence::default();
        loop {
            io.check().map_err(|_| {
                RemoteError::new("REMOTE_CLOUD_STOPPED", "Cloud transport stopped.")
            })?;
            let round = IoBudget {
                deadline: io
                    .deadline
                    .min(std::time::Instant::now() + crate::limits::CLOUD_RECONCILE_CALL),
                cancelled: io.cancelled,
            };
            let time = self.provider.now();
            match self.reconcile(time.wall_ms, &round).await {
                Ok(count) => {
                    cadence.succeeded();
                    if count > 0 {
                        cadence.activity(time.monotonic_ms);
                    }
                }
                Err(error) if io.check().is_err() || error.code == "REMOTE_SERVE_STOPPED" => {
                    return Err(error);
                }
                Err(_) => cadence.failed(),
            }
            let delay = poll_delay(&cadence, self.provider.now().monotonic_ms, |max| {
                self.provider.jitter(max)
            });
            self.provider.wait(io, delay)?;
        }
    }
    pub async fn reconcile(&self, now: u64, io: &IoBudget<'_>) -> Result<usize, RemoteError> {
        let expired = self
            .sessions
            .with_operation_store(|s| s.cloud_expired(now))?;
        for (peer, id) in expired {
            self.provider.delete(&peer, "inbox", &id, io).await?;
            self.provider.delete(&peer, "responses", &id, io).await?;
            self.sessions
                .with_operation_store(|s| s.cloud_forget(&peer, &id, now))?;
        }
        let (from, mut cursor, mut snapshot) = self.sessions.with_operation_store(|store| {
            if let Some(checkpoint) = store.cloud_query_checkpoint()? {
                return Ok(match checkpoint.continuation {
                    Some(progress) => (
                        progress.from_ms,
                        Some(progress.cursor),
                        Some(progress.read_ms),
                    ),
                    None => (checkpoint.completed.query_from_ms(), None, None),
                });
            }
            // Old Stores have peer checkpoints only. Once a machine query starts,
            // its own continuation/checkpoint owns progress, even for quiet peers.
            let from = store
                .cloud_peers()?
                .iter()
                // The machine UUID's legacy checkpoint belongs to enrollment (#4788).
                .filter(|peer| peer.as_str() != self.provider.machine())
                .map(|p| store.cloud_checkpoint(p).map(|c| c.query_from_ms()))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .min()
                .unwrap_or(0);
            Ok((from, None, None))
        })?;
        let mut count = 0;
        loop {
            // A one-document query costs at most 1 delivery + 2 Rules lookups, including idle.
            let page = self.provider.inbox(from, 1, cursor.as_ref(), io).await?;
            snapshot = Some(snapshot.map_or(page.read_ms, |old: u64| old.min(page.read_ms)));
            for document in page.documents {
                if count >= crate::journal::OPERATIONS as usize {
                    return Err(RemoteError::new(
                        "REMOTE_CLOUD_CAPACITY",
                        "Cloud catch-up reached its finite bound; progress is saved for the next poll.",
                    ));
                }
                count += 1;
                let delivery = async {
                    if let Some(frozen) = self.receiver.receive(&document, now)? {
                        let frozen = Arc::new(frozen);
                        let publication = Arc::clone(&frozen);
                        let observed = self.provider.create(
                            document.peer(), "responses", document.id(), &frozen.bytes,
                            Box::new(move || publication.claim_publication().map_err(|error| {
                                if super::ingress::permanent(&error.code) {
                                    crate::sealed_sync::SyncError::Denied
                                } else {
                                    crate::sealed_sync::SyncError::Unavailable
                                }
                            })), io,
                        ).await;
                        match observed {
                            Ok(Some(response)) if response.bytes() == frozen.bytes => {},
                            Ok(Some(_)) => return Err(RemoteError::new(
                                "REMOTE_CLOUD_CONFLICT",
                                "The response ID belongs to different ciphertext.",
                            )),
                            Err(error) if error.code != "REMOTE_CLOUD_UNCONFIRMED" => return Err(error),
                            _ => {
                                // The frozen reply and durable attempt remain unconfirmed. Do
                                // not let that one carrier block a fresh signed recovery behind
                                // it. Before the claim, failures still hold the checkpoint.
                                if !frozen.publication_attempted()? {
                                    return Err(RemoteError::new(
                                        "REMOTE_CLOUD_UNCONFIRMED",
                                        "The cloud response could not be confirmed; it will not be sent again.",
                                    ));
                                }
                            }
                        }
                    }
                    Ok(())
                }.await;
                match delivery {
                    Ok(()) => {}
                    Err(error) if super::ingress::permanent(&error.code) => {
                        self.sessions.with_operation_store(|store| {
                            store.cloud_refuse(
                                document.peer(),
                                document.id(),
                                &error.code,
                                None,
                                now,
                            )
                        })?
                    }
                    Err(error) => return Err(error),
                }
                // A later transient failure or finite cap must not erase this durable
                // prefix. Preserve the exact server timestamp and ID tie-breaker.
                self.sessions.with_operation_store(|store| {
                    store.cloud_progress(&super::store::Continuation {
                        from_ms: from,
                        read_ms: snapshot.unwrap_or(page.read_ms),
                        cursor: document.cursor().clone(),
                    })
                })?;
                cursor = Some(document.cursor().clone());
            }
            if !page.full {
                break;
            }
        }
        self.sessions
            .with_operation_store(|s| s.cloud_finish_query(snapshot.unwrap_or(0)))?;
        Ok(count)
    }
}

pub fn poll_delay(
    cadence: &crate::native_sync::PollCadence,
    now: u64,
    jitter: impl FnMut(u64) -> u64,
) -> u64 {
    cadence.delay_ms(now, jitter).clamp(
        crate::limits::CLOUD_POLL_MIN_MS,
        crate::limits::CLOUD_POLL_IDLE_MS,
    )
}
