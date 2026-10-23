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
    pub fn describe(&self, grant: &Grant) -> Result<Value, RemoteError> {
        let store = self.store()?;
        observed_device(&store, grant)
    }
    pub fn visible(&self) -> Result<Vec<Value>, RemoteError> {
        let store = self.store()?;
        store
            .active_grants()?
            .iter()
            .map(|grant| observed_device(&store, grant))
            .collect()
    }
    pub fn remove_others(&self, keep: &str) -> Result<usize, RemoteError> {
        let removed = self.store()?.remove_others(keep)?;
        self.committed_removals(&removed);
        Ok(removed.len())
    }
    pub(crate) fn committed_removals(&self, clients: &[String]) {
        if let Some(sessions) = &self.sessions {
            for client in clients {
                sessions.end_device(client);
            }
        }
        self.wake();
    }
    /// Disable the grant before acknowledging, then end the device's session
    /// and tunnels. Revoking twice reports the same revoked grant.
    pub fn revoke(&self, client_id: &str) -> Result<Grant, RemoteError> {
        let mut store = self.store()?;
        let client_id = resolve_person(&store, client_id)?;
        let grant = store.revoke(&client_id)?.ok_or_else(|| {
            RemoteError::new("REMOTE_DEVICE_NOT_FOUND", "No paired device has that ID.")
        })?;
        drop(store);
        if let Some(sessions) = &self.sessions {
            sessions.end_device(&client_id);
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
        if before.is_some_and(|g| g.name != grant.name) {
            self.wake();
        }
        Ok(grant)
    }
    pub fn talk(&self, client_id: &str, enabled: bool) -> Result<Grant, RemoteError> {
        let mut store = self.store()?;
        let client_id = resolve_person(&store, client_id)?;
        let before = store.grant(&client_id)?;
        let grant = store.talk(&client_id, enabled)?.ok_or_else(|| {
            RemoteError::new("REMOTE_DEVICE_NOT_FOUND", "No paired device has that ID.")
        })?;
        drop(store);
        if before.is_some_and(|g| g.revision != grant.revision) {
            self.committed_change(&client_id);
        }
        Ok(grant)
    }
    pub fn designate(&self, client_id: &str, origin: &str) -> Result<Grant, RemoteError> {
        self.store()?
            .designate(client_id, origin, crate::pairing::now_ms()?)
    }
    pub fn undesignate(&self) -> Result<(), RemoteError> {
        self.store()?.undesignate()
    }
    /// Effects and receipts are already committed; release Store/live locks first.
    pub(crate) fn committed_change(&self, client_id: &str) {
        if let Some(sessions) = &self.sessions {
            sessions.end_device(client_id);
        }
        self.wake();
    }
    pub(crate) fn wake(&self) {
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

/// Resolve once while holding the Store lock. An exact client ID takes priority;
/// display names select only a single active grant and never pick the first match.
fn resolve_person(store: &Store, reference: &str) -> Result<String, RemoteError> {
    if let Some(grant) = store.grant(reference)? {
        return Ok(grant.client_id);
    }
    let matches: Vec<_> = store
        .active_grants()?
        .into_iter()
        .filter(|g| g.name == reference)
        .collect();
    match matches.as_slice() {
        [grant] => Ok(grant.client_id.clone()),
        [] => Err(RemoteError::new(
            "REMOTE_DEVICE_NOT_FOUND",
            "No paired device has that name or client ID.",
        )),
        _ => Err(RemoteError::new(
            "REMOTE_DEVICE_AMBIGUOUS",
            &format!(
                "More than one device is named {reference:?}. Use a client ID: {}.",
                matches
                    .iter()
                    .map(|g| format!("{:?}", g.client_id))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
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
fn observed_device(store: &Store, grant: &Grant) -> Result<Value, RemoteError> {
    let mut row = device_json(grant);
    row["lastUsedAtMs"] = json!(store.last_used(&grant.client_id)?);
    Ok(row)
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

#[cfg(test)]
mod person_tests {
    use super::*;

    #[test]
    fn unique_person_names_mutate_only_that_grant_and_duplicates_list_ids() {
        let root =
            std::env::temp_dir().join(format!("tmt-person-{}", crate::store::uuid_v4().unwrap()));
        let serving = crate::state::Layout::open(&root)
            .unwrap()
            .serve_lock()
            .unwrap();
        let mut store = Store::open(&serving).unwrap();
        let mut grants = Vec::new();
        for (index, name) in ["Alex", "Sam", "Sam"].into_iter().enumerate() {
            let grant = Grant {
                client_id: crate::store::uuid_v4().unwrap(),
                public_key: ed25519_dalek::SigningKey::from_bytes(&[index as u8 + 1; 32])
                    .verifying_key()
                    .to_bytes(),
                kind: "cli".into(),
                origin: "cli".into(),
                name: name.into(),
                agents: "all".into(),
                scopes: crate::store::DEFAULT_SCOPES.map(str::to_owned).into(),
                mode: "hold".into(),
                issued_at_ms: 1,
                expires_at_ms: None,
                revision: 1,
                disabled: false,
            };
            store.insert_grant(&grant).unwrap();
            grants.push(grant);
        }
        let devices = Devices::new(Arc::new(Mutex::new(store)), None);
        let enabled = devices.talk("Alex", true).unwrap();
        assert_eq!(enabled.client_id, grants[0].client_id);
        assert!(enabled.permits_scope("talk"));
        assert_eq!(enabled.mode, "hold");
        let before = devices.list().unwrap();
        for error in [
            devices.talk("Sam", true).unwrap_err(),
            devices.revoke("Sam").unwrap_err(),
        ] {
            assert_eq!(error.code, "REMOTE_DEVICE_AMBIGUOUS");
            assert!(error.message.contains(&grants[1].client_id));
            assert!(error.message.contains(&grants[2].client_id));
        }
        assert_eq!(devices.list().unwrap(), before);
        assert_eq!(
            devices.revoke("Alex").unwrap().client_id,
            grants[0].client_id
        );
        assert!(devices.revoke(&grants[0].client_id).unwrap().disabled);
        devices.revoke(&grants[1].client_id).unwrap();
        assert_eq!(
            devices.talk("Sam", true).unwrap().client_id,
            grants[2].client_id
        );
        assert_eq!(
            devices.talk("missing", true).unwrap_err().code,
            "REMOTE_DEVICE_NOT_FOUND"
        );
        drop(devices);
        drop(serving);
        std::fs::remove_dir_all(root).unwrap();
    }
}
