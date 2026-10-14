//! The three page interests of the one active page. Notices are wakeups only; the verified
//! Store owns every publication position.
use crate::{ChannelError, SealedClient, sealed_documents};
use std::time::Instant;
use tmt_extension_objects::{Counter, sync};

/// What an interest watches: the page's records (log and checkpoints), or its membership.
#[derive(Clone, Copy)]
enum Watches {
    Records,
    Authority,
}
/// What changed since the last look, by the kind of interest that noticed it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Changes {
    /// The page's membership records changed.
    pub(crate) authority: bool,
    /// Its log or its checkpoints changed.
    pub(crate) records: bool,
}
impl Changes {
    pub(crate) fn any(self) -> bool {
        self.authority || self.records
    }
}

/// A single worker owns the active page group. Confirmed cancellation removes
/// each retained request-ID slot before a successor can register any interest.
pub(crate) struct PageInterests {
    client: SealedClient,
    binding: String,
    ids: Vec<(Counter, Watches)>,
}
impl PageInterests {
    pub(crate) fn new(client: SealedClient, binding: String) -> Self {
        Self {
            client,
            binding,
            ids: Vec::new(),
        }
    }
    pub(crate) fn select(
        &mut self,
        space: &str,
        page: &str,
        epoch: u64,
        deadline: Instant,
    ) -> Result<(), ChannelError> {
        // Validate the successor before disturbing the current page.
        let page_scope = sealed_documents::page_scope(space, page, epoch)
            .map_err(|_| ChannelError::Unavailable)?;
        let authority = sealed_documents::authority_scope(space, page, epoch, "membership", "")
            .map_err(|_| ChannelError::Unavailable)?;
        self.cancel(deadline)?;
        for (resource, scope, watches) in [
            ("log", page_scope.clone(), Watches::Records),
            ("checkpoints", page_scope, Watches::Records),
            ("authority", authority, Watches::Authority),
        ] {
            let reply = self.client.sealed_request(
                sync::Call::Subscribe {
                    binding: self.binding.clone(),
                    interest: sync::Interest {
                        resource: resource.into(),
                        scope,
                    },
                },
                deadline,
            );
            match reply {
                Ok((id, sync::Outcome::Subscribed { id: confirmed }))
                    if confirmed == id.to_string() =>
                {
                    self.ids.push((id, watches))
                }
                _ => {
                    // A partly installed group never lends its slots to another
                    // page. Join cancellations, or invalidate the generation.
                    if self.cancel(deadline).is_err() {
                        self.client.end_generation();
                    }
                    return Err(ChannelError::Unavailable);
                }
            }
        }
        Ok(())
    }
    pub(crate) fn cancel(&mut self, deadline: Instant) -> Result<(), ChannelError> {
        while let Some((id, _)) = self.ids.last().copied() {
            let reply = self.client.sealed_request(
                sync::Call::Cancel {
                    binding: self.binding.clone(),
                    subscription_id: id.to_string(),
                },
                deadline,
            );
            match reply {
                Ok((_, sync::Outcome::Cancelled { id: confirmed }))
                    if confirmed == id.to_string() =>
                {
                    self.ids.pop();
                }
                _ => {
                    self.client.end_generation();
                    self.ids.clear();
                    return Err(ChannelError::Unavailable);
                }
            }
        }
        Ok(())
    }
    /// Coalesced changes do not carry a cursor or make this page current.
    pub(crate) fn changed(&self) -> Result<Changes, ChannelError> {
        let mut changes = Changes::default();
        for (id, watches) in &self.ids {
            match self.client.sealed_notice(*id)? {
                Some(sync::InterestNotice::Changed) => match watches {
                    Watches::Records => changes.records = true,
                    Watches::Authority => changes.authority = true,
                },
                Some(sync::InterestNotice::Terminal { .. }) => {
                    return Err(ChannelError::Unavailable);
                }
                None => {}
            }
        }
        Ok(changes)
    }
}
impl Drop for PageInterests {
    fn drop(&mut self) {
        // Normal lifecycle joins cancel explicitly. If unwinding leaves handles,
        // their generation must not survive to install an overlapping group.
        if !self.ids.is_empty() {
            self.client.end_generation();
        }
    }
}
