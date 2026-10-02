//! Local device management (`tmt remote devices`). Serve answers it over the
//! control socket; without a running serve the command opens remote state
//! under the serve lock itself, so remote.db keeps a single opener.
use crate::{
    error::RemoteError,
    mount::Mounts,
    session::DoorSessions,
    store::{Grant, Store},
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

/// Successful snapshots are replayed too, so a replaced extension socket needs
/// no persistent delivery cursor or registration handshake.
const REPLAY: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(30);

pub struct Devices {
    store: Arc<Mutex<Store>>,
    /// Live sessions to end on revocation; absent when serve is not running.
    sessions: Option<Arc<DoorSessions>>,
    changed: Mutex<bool>,
    ready: Condvar,
}
impl Devices {
    pub fn new(store: Arc<Mutex<Store>>, sessions: Option<Arc<DoorSessions>>) -> Self {
        Self {
            store,
            sessions,
            changed: Mutex::new(false),
            ready: Condvar::new(),
        }
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
        self.wake();
        Ok(grant)
    }
    pub fn rename(&self, client_id: &str, name: &str) -> Result<Grant, RemoteError> {
        let mut store = self.store()?;
        let before = store.grant(client_id)?;
        let grant = store.rename(client_id, name)?.ok_or_else(|| {
            RemoteError::new("REMOTE_DEVICE_NOT_FOUND", "No paired device has that ID.")
        })?;
        drop(store);
        if before.is_some_and(|g| g.revision != grant.revision) {
            if let Some(sessions) = &self.sessions {
                sessions.end_device(client_id);
            }
            self.wake();
        }
        Ok(grant)
    }
    fn wake(&self) {
        if let Ok(mut changed) = self.changed.lock() {
            *changed = true;
            self.ready.notify_one();
        }
    }
    /// One joined worker delivers current grant state outside the store lock
    /// and local command acknowledgment. Failed snapshots back off 1–30 s;
    /// committed mutations wake it immediately. No effects depend on a reply.
    pub fn start_events(
        self: &Arc<Self>,
        mounts: Arc<Mounts>,
    ) -> Result<DeviceEvents, RemoteError> {
        let devices = Arc::clone(self);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("remote-device-events".into())
            .spawn(move || {
                let mut retry = Duration::from_secs(1);
                while !flag.load(Ordering::Acquire) {
                    let mut acknowledged = true;
                    match devices.list() {
                        Ok(grants) => {
                            for grant in grants {
                                if flag.load(Ordering::Acquire) {
                                    break;
                                }
                                acknowledged &= mounts.device_event(&event_json(&grant));
                            }
                        }
                        Err(_) => acknowledged = false,
                    }
                    let wait = if acknowledged {
                        retry = Duration::from_secs(1);
                        REPLAY
                    } else {
                        let wait = retry;
                        retry = (retry * 2).min(RETRY_MAX);
                        wait
                    };
                    let Ok(changed) = devices.changed.lock() else {
                        break;
                    };
                    let Ok((mut changed, _)) =
                        devices.ready.wait_timeout_while(changed, wait, |changed| {
                            !*changed && !flag.load(Ordering::Acquire)
                        })
                    else {
                        break;
                    };
                    *changed = false;
                }
            })
            .map_err(|e| {
                RemoteError::new("REMOTE_IO", &format!("Could not start device events: {e}."))
            })?;
        Ok(DeviceEvents {
            devices: Arc::clone(self),
            stop,
            worker: Some(worker),
        })
    }
    fn store(&self) -> Result<std::sync::MutexGuard<'_, Store>, RemoteError> {
        self.store.lock().map_err(|_| {
            RemoteError::new("REMOTE_STATE_UNAVAILABLE", "Remote state is unavailable.")
        })
    }
}
/// Shutdown wakes the backoff wait and joins the bounded socket attempt before
/// releasing remote state. Dropping during failed startup has the same cleanup.
pub struct DeviceEvents {
    devices: Arc<Devices>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Drop for DeviceEvents {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.devices.wake();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Level-triggered wire state: the latest name or a permanent revoked tombstone.
pub fn event_json(grant: &Grant) -> Value {
    let mut event = json!({
        "type": if grant.disabled { "device.revoked" } else { "device.renamed" },
        "deviceId": grant.client_id,
        "grantRevision": grant.revision,
    });
    if !grant.disabled {
        event["name"] = json!(grant.name);
    }
    event
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
