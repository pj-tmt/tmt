//! Cloud proof enters the common operation policy without a local browser/session ceremony.
use super::*;
use crate::admission::PermitBinding;
impl DoorSessions {
    pub(crate) fn message_policy(
        &self,
        action: BindingAction,
        grant: &Grant,
        message: &SignedMessage,
    ) -> Result<(), (&'static str, &'static str)> {
        if !action.matches(message) {
            return Err((
                "REMOTE_INPUT_INVALID",
                "Route and signed operation disagree.",
            ));
        }
        let Some(scope) = admission::scope(message.envelope().operation) else {
            return Err((
                "REMOTE_INPUT_INVALID",
                "Operation is not remotely callable.",
            ));
        };
        if scope.is_some_and(|scope| !grant.permits_scope(scope)) {
            return Err((
                "REMOTE_SCOPE_DENIED",
                if scope == Some("talk") {
                    "talk"
                } else {
                    "Device scope does not admit this operation."
                },
            ));
        }
        Ok(())
    }
    /// A retained native key verifies content even after revoke; it grants no operation access.
    pub(crate) fn cloud_peer_key(&self, peer: &str) -> Result<Grant, RemoteError> {
        self.with_operation_store(|store| store.grant(peer))?
            .filter(|grant| grant.kind == "cli" && grant.origin == "cli")
            .ok_or_else(|| RemoteError::new("REMOTE_CLOSED", "Cloud peer authority ended."))
    }
    pub(crate) fn cloud_peer(&self, peer: &str, now: u64) -> Result<Grant, RemoteError> {
        let grant = self.cloud_peer_key(peer)?;
        if !grant.live_at(now) {
            return Err(RemoteError::new(
                "REMOTE_CLOSED",
                "Cloud peer authority ended.",
            ));
        }
        grant.authority()?;
        Ok(grant)
    }
    pub(crate) fn admit_cloud(
        self: &Arc<Self>,
        action: BindingAction,
        verified: crate::cloud_inbox::Verified,
    ) -> Result<MessagePermit, RemoteError> {
        let (message, grant) = verified.into_parts();
        let now = now_ms()?;
        if message.envelope().machine_id != self.machine_id || message.envelope().origin != "cli" {
            return Err(RemoteError::new("REMOTE_CLOSED", "Cloud peer refused."));
        }
        self.with_cloud_store(&grant, now, |store| {
            store.charge_call(&grant.client_id, now)
        })?;
        self.message_policy(action, &grant, &message)
            .map_err(|(code, _)| RemoteError::new(code, "Remote request refused."))?;
        let authority = grant.authority()?;
        Ok(MessagePermit {
            message,
            grant,
            authority,
            sessions: Arc::clone(self),
            busy: Arc::new(AtomicBool::new(true)),
            binding: PermitBinding::Cloud,
        })
    }
    pub(crate) fn with_cloud_store<T>(
        &self,
        expected: &Grant,
        now: u64,
        action: impl FnOnce(&mut Store) -> Result<T, RemoteError>,
    ) -> Result<(T, u64), RemoteError> {
        let live = self.live.lock().map_err(crate::store::database)?;
        if live.stopped {
            return Err(RemoteError::new(
                EndReason::ServeStopped.code(),
                "Door stopped.",
            ));
        }
        let mut store = self.store.lock().map_err(crate::store::database)?;
        if let Some(reason) = EndReason::authority(
            store.grant(&expected.client_id)?.as_ref(),
            expected.revision,
            now,
        ) {
            return Err(RemoteError::new(
                reason.code(),
                "Device authority changed or ended.",
            ));
        }
        Ok((action(&mut store)?, live.generation))
    }
    pub(crate) fn sign_cloud(&self, bytes: &[u8]) -> [u8; 64] {
        self.machine_key.sign(bytes)
    }
    pub(crate) fn cloud_response(
        &self,
        request: &SignedMessage,
        payload: &Value,
    ) -> Result<Vec<u8>, RemoteError> {
        self.signed_response(
            request.envelope(),
            request.envelope().session_id,
            1,
            payload,
            now_ms()?,
        )
    }
}
