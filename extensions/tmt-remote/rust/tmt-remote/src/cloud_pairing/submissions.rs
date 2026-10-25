//! One pending native submission in the existing private connection record.
//! Its presence is the durable pre-I/O claim; no code/key/gate is persisted.
use super::{
    Connection,
    store::{cloud_connection_in, write_cloud_connection_in},
    wire,
};
use crate::{
    error::RemoteError,
    invitations::{Context, port::Submission},
    store::{Store, database},
};
use rusqlite::TransactionBehavior;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingSubmission {
    pub context: Context,
    pub request: Vec<u8>,
}

impl Store {
    /// Returns true only for the first durably claimed original. A repeat is
    /// observation-only, including a crash after this commit but before HTTP.
    /// Confirmed routing/pins remain unchanged until receipt adoption.
    pub fn claim_cloud_submission(
        &mut self,
        desired: &Connection,
        context: &Context,
        submitted: &Submission,
        io: &crate::objects::IoBudget<'_>,
    ) -> Result<bool, RemoteError> {
        crate::invitations::service::check(io)?;
        wire::v2::validate_context(context)?;
        let now = crate::pairing::now_ms()?;
        let project = desired.config["projectId"]
            .as_str()
            .ok_or_else(wire::invalid)?;
        if desired.version != 1
            || desired.owner != context.owner
            || desired.owner_key != context.owner_key
            || desired.peer != submitted.candidate.machine
            || self.machine()?.id != desired.peer
            || project.as_bytes() != context.binding.reference
            || desired.uid.as_bytes() != submitted.candidate.principal
            || !super::uid_ok(&desired.uid)
            || !crate::remote_catalog::name_ok(&desired.name)
            || context.issued_at_ms.saturating_sub(now)
                > crate::session::CLOCK_SKEW.as_millis() as u64
        {
            return Err(wire::invalid());
        }
        if now >= context.expires_at_ms {
            return Err(wire::expired());
        }
        let pending = PendingSubmission {
            context: context.clone(),
            request: wire::v2::submission(context, submitted)?,
        };
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(database)?;
        crate::invitations::service::check(io)?;
        let now = crate::pairing::now_ms()?;
        if now >= context.expires_at_ms {
            return Err(wire::expired());
        }
        let mut record = if let Some(old) = cloud_connection_in(&tx, &desired.owner)? {
            if old.peer != desired.peer {
                return Err(wire::invalid());
            }
            if let Some(claimed) = &old.pending_submission {
                if claimed == &pending {
                    return Ok(false);
                }
                if claimed.context.offer == context.offer {
                    return Err(conflict());
                }
                if now < claimed.context.expires_at_ms {
                    return Err(RemoteError::new(
                        "REMOTE_INVITATION_BUSY",
                        "Another original invitation for this owner is pending. Finish it or wait for its original expiry.",
                    ));
                }
            }
            // A completed submission remains its own immutable original; it
            // cannot become another publication after the pending slot clears.
            let original = crate::wire::strict_json(&old.request);
            if old.confirmed
                && original.as_ref().is_some_and(|body| {
                    body["version"] == 2 && body["context"]["offer"] == context.offer
                })
            {
                if old.request == pending.request {
                    return Ok(false);
                }
                return Err(conflict());
            }
            old
        } else {
            if desired.confirmed
                || !desired.request.is_empty()
                || desired.reroute_token.is_some()
                || !desired.descriptor.is_empty()
                || desired.pending_submission.is_some()
            {
                return Err(wire::invalid());
            }
            if cloud_connection_in(&tx, &desired.name)?.is_some() {
                return Err(conflict());
            }
            desired.clone()
        };
        record.pending_submission = Some(pending);
        write_cloud_connection_in(&tx, &record)?;
        crate::invitations::service::check(io)?;
        if crate::pairing::now_ms()? >= context.expires_at_ms {
            return Err(wire::expired());
        }
        tx.commit().map_err(database)?;
        Ok(true)
    }
}

/// Ordinary uid/descriptor saves cannot discard a claim admitted after their
/// snapshot was read. Only verified receipt adoption may consume this slot.
pub(super) fn retain_pending_in(
    connection: &rusqlite::Connection,
    desired: &Connection,
) -> Result<Connection, RemoteError> {
    let mut record = desired.clone();
    record.pending_submission =
        cloud_connection_in(connection, &desired.owner)?.and_then(|old| old.pending_submission);
    Ok(record)
}
fn conflict() -> RemoteError {
    RemoteError::new(
        "REMOTE_ID_CONFLICT",
        "This invitation ID already names a different signed submission.",
    )
}
