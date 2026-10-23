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
    /// Run only on the serving owner's joined worker. Cancellation and provider refusals
    /// end this worker; a new owner resumes its persisted checkpoints after restart.
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
            let count = self.reconcile(time.wall_ms, &round).await?;
            cadence.succeeded();
            if count > 0 {
                cadence.activity(time.monotonic_ms);
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
        let peers = self.sessions.with_operation_store(|s| s.cloud_peers())?;
        let from = self.sessions.with_operation_store(|store| {
            peers
                .iter()
                .map(|p| store.cloud_checkpoint(p).map(|c| c.query_from_ms()))
                .collect::<Result<Vec<_>, _>>()
                .map(|v| v.into_iter().min().unwrap_or(0))
        })?;
        let mut cursor = None;
        let mut count = 0;
        let mut snapshot = None;
        loop {
            // A one-document query costs at most 1 delivery + 2 Rules lookups, including idle.
            let page = self.provider.inbox(from, 1, cursor.as_ref(), io).await?;
            snapshot = Some(snapshot.map_or(page.read_ms, |old: u64| old.min(page.read_ms)));
            for document in page.documents {
                if count >= crate::journal::OPERATIONS as usize {
                    return Err(RemoteError::new(
                        "REMOTE_CLOUD_CAPACITY",
                        "Cloud catch-up reached its finite bound; the checkpoint was not advanced.",
                    ));
                }
                count += 1;
                if now.saturating_sub(document.created_ms()) >= crate::journal::RECOVERY {
                    self.provider
                        .delete(document.peer(), "inbox", document.id(), io)
                        .await?;
                    self.provider
                        .delete(document.peer(), "responses", document.id(), io)
                        .await?;
                    self.sessions.with_operation_store(|s| {
                        s.cloud_forget(document.peer(), document.id(), now)
                    })?;
                } else {
                    let frozen = Arc::new(self.receiver.receive(&document, now)?);
                    let publication = Arc::clone(&frozen);
                    let observed = self
                        .provider
                        .create(
                            document.peer(),
                            "responses",
                            document.id(),
                            &frozen.bytes,
                            Box::new(move || {
                                publication
                                    .claim_publication()
                                    .map_err(|_| crate::sealed_sync::SyncError::Unavailable)
                            }),
                            io,
                        )
                        .await?;
                    match observed {
                        Some(response) if response.bytes() == frozen.bytes => {}
                        Some(_) => {
                            return Err(RemoteError::new(
                                "REMOTE_CLOUD_CONFLICT",
                                "The response ID belongs to different ciphertext.",
                            ));
                        }
                        None => {
                            return Err(RemoteError::new(
                                "REMOTE_CLOUD_UNCONFIRMED",
                                "The cloud response could not be confirmed; it will not be sent again.",
                            ));
                        }
                    }
                }
                cursor = Some(document.cursor().clone());
            }
            if !page.full {
                break;
            }
        }
        self.sessions
            .with_operation_store(|s| s.cloud_complete(&peers, snapshot.unwrap_or(0)))?;
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
