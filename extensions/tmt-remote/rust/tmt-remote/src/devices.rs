//! Local device management (`tmt remote devices`). Serve answers it over the
//! control socket; without a running serve the command opens remote state
//! under the serve lock itself, so remote.db keeps a single opener.
use crate::{
    error::RemoteError,
    session::DoorSessions,
    store::{Grant, Store},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub struct Devices {
    store: Arc<Mutex<Store>>,
    /// Live sessions to end on revocation; absent when serve is not running.
    sessions: Option<Arc<DoorSessions>>,
}
impl Devices {
    pub fn new(store: Arc<Mutex<Store>>, sessions: Option<Arc<DoorSessions>>) -> Self {
        Self { store, sessions }
    }
    pub fn list(&self) -> Result<Vec<Grant>, RemoteError> {
        self.store().and_then(|store| store.grants())
    }
    /// Disable the grant before acknowledging, then end the device's session
    /// and tunnels. Revoking twice reports the same revoked grant.
    pub fn revoke(&self, client_id: &str) -> Result<Grant, RemoteError> {
        let grant = self.store()?.revoke(client_id)?.ok_or_else(|| {
            RemoteError::new("REMOTE_DEVICE_NOT_FOUND", "No paired device has that ID.")
        })?;
        if let Some(sessions) = &self.sessions {
            sessions.end_device(client_id);
        }
        Ok(grant)
    }
    fn store(&self) -> Result<std::sync::MutexGuard<'_, Store>, RemoteError> {
        self.store.lock().map_err(|_| {
            RemoteError::new("REMOTE_STATE_UNAVAILABLE", "Remote state is unavailable.")
        })
    }
}
/// A device row as `tmt remote devices --json` reports it. The public key is
/// identified by its four fingerprint words, never printed raw.
pub fn device_json(grant: &Grant) -> Value {
    let words = crate::canonical::fingerprint_words(&grant.public_key)
        .map(|w| w.join(" "))
        .unwrap_or_default();
    json!({
        "clientId": grant.client_id,
        "name": grant.name,
        "kind": grant.kind,
        "origin": grant.origin,
        "words": words,
        "agents": grant.agents,
        "scopes": grant.scopes,
        "mode": grant.mode,
        "issuedAtMs": grant.issued_at_ms,
        "expiresAtMs": grant.expires_at_ms,
        "revision": grant.revision,
        "revoked": grant.disabled,
    })
}
