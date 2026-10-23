//! One private keyring owner; Document commits before any descriptor can be advertised.
use crate::{
    canonical,
    cloud_hpke::{Descriptor, Error, Pin, RecipientKey, VerifiedDescriptor},
    error::RemoteError,
    limits,
    state::{Layout, MachineKey, document_error, document_result},
    wire,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Mutex};
const FILE: &str = "cloud-keys.json";
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredKey {
    descriptor: String,
    ikm: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Keyring {
    version: u64,
    machine_id: String,
    revision: u64,
    keys: Vec<StoredKey>,
}
struct Entry {
    descriptor: VerifiedDescriptor,
    key: RecipientKey,
}
fn error(code: Error) -> RemoteError {
    RemoteError::new(
        code.code(),
        if code == Error::UpgradeRequired {
            "Upgrade Remote to read these cloud encryption keys. Local pairing is unchanged."
        } else {
            "Cloud encryption keys are unavailable; local pairing is unchanged."
        },
    )
}
fn entropy() -> Result<[u8; 48], RemoteError> {
    let mut bytes = [0; 48];
    getrandom::fill(&mut bytes)
        .map_err(|_| RemoteError::new("REMOTE_ENTROPY", "Could not obtain key entropy."))?;
    Ok(bytes)
}
/// Lifecycle integration only. No transport, authority database or independent scheduler.
pub struct CloudKeys {
    layout: Layout,
    machine_id: String,
    auth: MachineKey,
    last_check: Mutex<Option<u64>>,
    admitted: Mutex<Option<Vec<u8>>>,
    renewal_error: Mutex<Option<String>>,
}
impl CloudKeys {
    pub fn new(layout: Layout, machine_id: String, auth: MachineKey) -> Self {
        Self {
            layout,
            machine_id,
            auth,
            last_check: Mutex::new(None),
            admitted: Mutex::new(None),
            renewal_error: Mutex::new(None),
        }
    }
    pub fn maintain(&self, now: u64) -> Result<(), RemoteError> {
        let mut last = self.last_check.lock().map_err(|_| error(Error::Key))?;
        if last.is_some_and(|time| now >= time && now - time < limits::CLOUD_KEY_CHECK_MS) {
            return Ok(());
        }
        *last = Some(now);
        let result = self.renew_at(now, entropy);
        *self.renewal_error.lock().map_err(|_| error(Error::Key))? =
            result.as_ref().err().map(|error| error.code.clone());
        result
    }
    /// Stable maintenance projection; no provider text, repeated warning or new scheduler.
    pub fn renewal_status(&self) -> Result<(), RemoteError> {
        match self
            .renewal_error
            .lock()
            .map_err(|_| error(Error::Key))?
            .as_deref()
        {
            None => Ok(()),
            Some(code) => Err(RemoteError::new(
                code,
                "Cloud encryption renewal is unavailable; local pairing is unchanged.",
            )),
        }
    }
    fn pin<'a>(&'a self, public: &'a [u8; 32]) -> Pin<'a> {
        Pin {
            machine_id: &self.machine_id,
            authentication_key: public,
        }
    }
    fn admit(&self, bytes: &[u8]) -> Result<(Keyring, Vec<Entry>), RemoteError> {
        if bytes.len() > limits::CLOUD_KEYRING_BYTES {
            return Err(error(Error::Bounds));
        }
        let value = wire::strict_json(bytes).ok_or_else(|| error(Error::Invalid))?;
        if value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|v| v != 1)
        {
            return Err(error(Error::UpgradeRequired));
        }
        let ring: Keyring = serde_json::from_value(value).map_err(|_| error(Error::Invalid))?;
        if ring.version != 1
            || ring.machine_id != self.machine_id
            || ring.keys.is_empty()
            || ring.keys.len() > limits::CLOUD_KEYRING_KEYS
        {
            return Err(error(Error::Invalid));
        }
        let mut ids = BTreeSet::new();
        let mut previous_revision = 0;
        let public = self.auth.public();
        let mut entries = vec![];
        for stored in &ring.keys {
            let descriptor =
                Descriptor::read_retained(stored.descriptor.as_bytes(), self.pin(&public))
                    .map_err(error)?;
            let mut ikm =
                canonical::base64url_bytes(&stored.ikm, 32).map_err(|_| error(Error::Invalid))?;
            let key =
                RecipientKey::from_ikm(ikm.as_slice().try_into().map_err(|_| error(Error::Key))?);
            ikm.fill(0);
            let value: serde_json::Value =
                serde_json::from_slice(descriptor.bytes()).map_err(|_| error(Error::Invalid))?;
            if descriptor.revision() <= previous_revision
                || !ids.insert(descriptor.key_id().to_owned())
                || value["publicKey"] != canonical::base64url(&key.public())
            {
                return Err(error(Error::Key));
            }
            previous_revision = descriptor.revision();
            entries.push(Entry { descriptor, key });
        }
        if previous_revision != ring.revision {
            return Err(error(Error::Revision));
        }
        Ok((ring, entries))
    }
    fn read(&self) -> Result<Vec<Entry>, RemoteError> {
        // Only a successfully synced Document outcome can become advertised state.
        let admitted = self.admitted.lock().map_err(|_| error(Error::Key))?;
        self.admit(admitted.as_deref().ok_or_else(|| error(Error::Key))?)
            .map(|(_, entries)| entries)
    }
    /// A future transport uses only freshly admitted keys; absent/expired (retired) IDs refuse.
    pub fn recipient(
        &self,
        key_id: &str,
        now: u64,
    ) -> Result<(RecipientKey, VerifiedDescriptor), RemoteError> {
        let entry = self
            .read()?
            .into_iter()
            .find(|entry| entry.descriptor.key_id() == key_id)
            .ok_or_else(|| error(Error::Key))?;
        entry
            .descriptor
            .valid_at(now)
            .map_err(|_| error(Error::Key))?;
        Ok((entry.key, entry.descriptor))
    }
    /// Only cloud ingress supplies trusted provider createTime. Retention never extends
    /// descriptor validity for a newly created carrier or for advertising/sending.
    pub(crate) fn recipient_for_document(
        &self,
        key_id: &str,
        created_ms: u64,
        now: u64,
    ) -> Result<(RecipientKey, VerifiedDescriptor), RemoteError> {
        let entry = self
            .read()?
            .into_iter()
            .find(|entry| entry.descriptor.key_id() == key_id)
            .ok_or_else(|| error(Error::Key))?;
        if now
            >= entry
                .descriptor
                .expires_at_ms()
                .saturating_add(crate::journal::RECOVERY)
        {
            return Err(error(Error::Key));
        }
        entry.descriptor.valid_at(created_ms).map_err(error)?;
        Ok((entry.key, entry.descriptor))
    }
    pub fn advertised(&self, now: u64) -> Result<VerifiedDescriptor, RemoteError> {
        let entry = self.read()?.pop().ok_or_else(|| error(Error::Key))?;
        entry.descriptor.valid_at(now).map_err(error)?;
        Ok(entry.descriptor)
    }
    fn renew_at(
        &self,
        now: u64,
        mut random: impl FnMut() -> Result<[u8; 48], RemoteError>,
    ) -> Result<(), RemoteError> {
        let document = tmt_extension_state::Document::new(
            &self.layout.directory,
            &self.layout.directory.join(FILE),
            limits::CLOUD_KEYRING_BYTES as u64,
            "Restart tmt remote serve",
        )
        .map_err(document_error)?;
        let mut admitted = self.admitted.lock().map_err(|_| error(Error::Key))?;
        let mut candidate = None;
        let result = document_result(document.update(|current| {
            if self.layout.read_file(FILE)?.is_some() != current.is_some() {
                return Err(error(Error::Key));
            }
            let (mut ring, entries) = if let Some(bytes) = current {
                self.admit(bytes)?
            } else {
                (
                    Keyring {
                        version: 1,
                        machine_id: self.machine_id.clone(),
                        revision: 0,
                        keys: vec![],
                    },
                    vec![],
                )
            };
            let needs_renewal = if let Some(latest) = entries.last() {
                // A backwards wall clock never signs a revision with overlapping invalid time.
                if latest.descriptor.valid_at(now) == Err(Error::NotYetValid) {
                    return Err(error(Error::NotYetValid));
                }
                now >= latest
                    .descriptor
                    .expires_at_ms()
                    .saturating_sub(limits::CLOUD_KEY_RENEW_LEAD_MS)
            } else {
                true
            };
            let before = ring.keys.len();
            let mut retained = vec![];
            for (stored, entry) in ring.keys.into_iter().zip(entries) {
                if now
                    < entry
                        .descriptor
                        .expires_at_ms()
                        .saturating_add(crate::journal::RECOVERY)
                {
                    retained.push(stored);
                }
            }
            if !needs_renewal {
                if retained.len() == before {
                    let bytes = current.expect("admitted entries have bytes").to_vec();
                    candidate = Some(bytes.clone());
                    return Ok(bytes);
                }
                ring.keys = retained;
                let bytes = serde_json::to_vec(&ring).map_err(|_| error(Error::Invalid))?;
                candidate = Some(bytes.clone());
                return Ok(bytes);
            }
            if retained.len() >= limits::CLOUD_KEYRING_KEYS {
                return Err(error(Error::Bounds));
            }
            let mut random = random()?;
            let mut seed: [u8; 32] = random[..32].try_into().map_err(|_| error(Error::Key))?;
            let key = RecipientKey::from_ikm(&seed);
            random[38] = (random[38] & 0x0f) | 0x40;
            random[40] = (random[40] & 0x3f) | 0x80;
            let hex = crate::state::hex(&random[32..]);
            let key_id = format!(
                "{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            );
            let revision = ring
                .revision
                .checked_add(1)
                .ok_or_else(|| error(Error::Revision))?;
            let expires = now
                .checked_add(limits::CLOUD_KEY_LIFETIME_MS)
                .ok_or_else(|| error(Error::Invalid))?;
            let descriptor = Descriptor::sign(
                &self.machine_id,
                &key_id,
                revision,
                &key.public(),
                now,
                expires,
                |bytes| self.auth.sign(bytes),
            )
            .map_err(error)?;
            Descriptor::verify(&descriptor, self.pin(&self.auth.public()), now, None)
                .map_err(error)?;
            retained.push(StoredKey {
                descriptor: String::from_utf8(descriptor).map_err(|_| error(Error::Invalid))?,
                ikm: canonical::base64url(&seed),
            });
            random.fill(0);
            seed.fill(0);
            ring.keys = retained;
            ring.revision = revision;
            let bytes = serde_json::to_vec(&ring).map_err(|_| error(Error::Invalid))?;
            candidate = Some(bytes.clone());
            Ok(bytes)
        }));
        if result.is_ok() {
            *admitted = candidate;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MACHINE: &str = "11111111-1111-4111-8111-111111111111";
    const NOW: u64 = 1_800_000_000_000;
    struct Fixture {
        root: std::path::PathBuf,
        keys: CloudKeys,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::path::PathBuf::from(format!(
                "/tmp/cloud-{}",
                crate::store::uuid_v4().unwrap()
            ));
            let layout = Layout::open(&root).unwrap();
            let auth = MachineKey::open(&layout).unwrap();
            Self {
                root,
                keys: CloudKeys::new(layout, MACHINE.into(), auth),
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }
    #[test]
    fn startup_renews_after_offline_expiry_and_keeps_identity_and_old_v1_bytes() {
        let f = Fixture::new();
        let auth = std::fs::read(f.root.join("remote/machine.key")).unwrap();
        f.keys.renew_at(NOW, || Ok([1; 48])).unwrap();
        let first = f.keys.advertised(NOW).unwrap();
        let first_bytes = first.bytes().to_vec();
        let file = f.root.join("remote/cloud-keys.json");
        let old = std::fs::read(&file).unwrap();
        let reopened = CloudKeys::new(
            Layout::open(&f.root).unwrap(),
            MACHINE.into(),
            MachineKey::open(&f.keys.layout).unwrap(),
        );
        reopened.renew_at(NOW + 1, || panic!("not due")).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), old);
        assert_eq!(reopened.advertised(NOW + 1).unwrap().bytes(), first_bytes);
        let after = NOW + limits::CLOUD_KEY_LIFETIME_MS + 1;
        assert_eq!(
            reopened.advertised(after).err().unwrap().code,
            Error::DescriptorExpired.code()
        );
        reopened.renew_at(after, || Ok([2; 48])).unwrap();
        assert_eq!(reopened.advertised(after).unwrap().revision(), 2);
        assert_eq!(
            reopened
                .recipient(first.key_id(), after)
                .err()
                .unwrap()
                .code,
            Error::Key.code()
        );
        assert_eq!(
            std::fs::read(f.root.join("remote/machine.key")).unwrap(),
            auth
        );
        assert!(!f.root.join("remote/remote.db").exists());
    }
    #[test]
    fn due_renewal_keeps_previous_unexpired_key_and_entropy_failure_keeps_old_state() {
        let f = Fixture::new();
        f.keys.renew_at(NOW, || Ok([1; 48])).unwrap();
        let first = f.keys.advertised(NOW).unwrap();
        let due = first.expires_at_ms() - limits::CLOUD_KEY_RENEW_LEAD_MS;
        f.keys.renew_at(due - 1, || panic!("not due")).unwrap();
        let file = f.root.join("remote/cloud-keys.json");
        let old = std::fs::read(&file).unwrap();
        assert!(f.keys.renew_at(due, || Err(error(Error::Key))).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), old);
        f.keys.renew_at(due, || Ok([2; 48])).unwrap();
        assert_eq!(f.keys.advertised(due).unwrap().revision(), 2);
        let (key, descriptor) = f.keys.recipient(first.key_id(), due).unwrap();
        assert_eq!(descriptor.bytes(), first.bytes());
        assert!(!key.public().is_empty());
        assert!(
            f.keys
                .recipient(first.key_id(), first.expires_at_ms())
                .is_err()
        );
    }
    #[test]
    fn damaged_or_newer_keyring_is_never_reset_and_private_file_policy_is_preserved() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        f.keys.renew_at(NOW, || Ok([1; 48])).unwrap();
        let file = f.root.join("remote/cloud-keys.json");
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        for bytes in [
            b"{\"version\":2}".as_slice(),
            b"{}".as_slice(),
            b"{\"version\":1,\"version\":1}".as_slice(),
        ] {
            std::fs::write(&file, bytes).unwrap();
            assert!(f.keys.renew_at(NOW, || panic!("never repair")).is_err());
            assert_eq!(std::fs::read(&file).unwrap(), bytes);
        }
    }
    #[test]
    fn frozen_v1_keyring_reopens_unchanged_and_write_failure_cannot_advertise_a_new_key() {
        let mut f = Fixture::new();
        std::fs::write(f.root.join("remote/machine.key"), [7; 32]).unwrap();
        f.keys.auth = MachineKey::open(&f.keys.layout).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/cloud-hpke-v1.json")).unwrap();
        let bytes = v["keyring"].as_str().unwrap().as_bytes();
        let file = f.root.join("remote/cloud-keys.json");
        f.keys.layout.file(FILE).unwrap();
        std::fs::write(&file, bytes).unwrap();
        // A real opaque grant-store sentinel proves this owner never rewrites its neighbor.
        let grants = f.root.join("remote/remote.db");
        f.keys.layout.file("remote.db").unwrap();
        std::fs::write(&grants, b"opaque existing grants").unwrap();
        f.keys.renew_at(NOW, || panic!("not due")).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        assert_eq!(
            f.keys.advertised(NOW).unwrap().bytes(),
            v["descriptor"].as_str().unwrap().as_bytes()
        );
        let staging = f.root.join("remote/.cloud-keys.json.tmt-staging");
        std::fs::create_dir(&staging).unwrap();
        let due = NOW + limits::CLOUD_KEY_LIFETIME_MS - limits::CLOUD_KEY_RENEW_LEAD_MS;
        assert!(f.keys.renew_at(due, || Ok([2; 48])).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        assert_eq!(f.keys.advertised(due).unwrap().revision(), 1);
        assert_eq!(std::fs::read(&grants).unwrap(), b"opaque existing grants");
    }
    #[test]
    fn the_existing_maintenance_tick_renews_and_coalesces_between_due_checks() {
        let f = Fixture::new();
        f.keys.maintain(NOW).unwrap();
        let first = f.keys.advertised(NOW).unwrap();
        let path = f.root.join("remote/cloud-keys.json");
        let before = std::fs::read(&path).unwrap();
        f.keys
            .maintain(NOW + limits::CLOUD_KEY_CHECK_MS - 1)
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let due = first.expires_at_ms() - limits::CLOUD_KEY_RENEW_LEAD_MS;
        f.keys.maintain(due).unwrap();
        assert_eq!(f.keys.advertised(due).unwrap().revision(), 2);
        assert!(f.keys.recipient(first.key_id(), due).is_ok());
    }
    #[test]
    fn offline_carrier_opens_under_retained_key_but_post_expiry_creation_refuses() {
        let f = Fixture::new();
        f.keys.renew_at(NOW, || Ok([1; 48])).unwrap();
        let first = f.keys.advertised(NOW).unwrap();
        use ed25519_dalek::{Signer, SigningKey};
        let sender = SigningKey::from_bytes(&[8; 32]);
        let vector: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/cloud-hpke-v1.json")).unwrap();
        let mut request: serde_json::Value =
            serde_json::from_str(vector["request"].as_str().unwrap()).unwrap();
        let created = first.expires_at_ms() - 1;
        request["timestampMs"] = serde_json::json!(created);
        let draft = serde_json::to_vec(&request).unwrap();
        let parsed =
            crate::wire::SignedMessage::decode(&draft, crate::limits::CLOUD_ENVELOPE_BYTES)
                .unwrap();
        request["signature"] = serde_json::json!(crate::canonical::base64url(
            &sender
                .sign(&crate::canonical::envelope(&parsed.envelope()).unwrap())
                .to_bytes()
        ));
        let exact = serde_json::to_vec(&request).unwrap();
        let public = sender.verifying_key().to_bytes();
        let peer = crate::cloud_hpke::Sender {
            client_id: "33333333-3333-4333-8333-333333333333",
            authentication_key: &public,
        };
        let (carrier, _) = crate::cloud_inbox::seal_native(&exact, &first, created, peer, |b| {
            sender.sign(b).to_bytes()
        })
        .unwrap();
        let interval = limits::CLOUD_KEY_LIFETIME_MS - limits::CLOUD_KEY_RENEW_LEAD_MS;
        f.keys.renew_at(NOW + interval, || Ok([2; 48])).unwrap();
        let file = f.root.join("remote/cloud-keys.json");
        let two = std::fs::read(&file).unwrap();
        let reopened = CloudKeys::new(
            Layout::open(&f.root).unwrap(),
            MACHINE.into(),
            MachineKey::open(&f.keys.layout).unwrap(),
        );
        reopened
            .renew_at(NOW + interval + 1, || panic!("unchanged two-key v1 ring"))
            .unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), two);
        let now = NOW + 2 * interval;
        reopened.renew_at(now, || Ok([3; 48])).unwrap();
        let (retained_key, retained_descriptor) = reopened
            .recipient_for_document(first.key_id(), created, now)
            .unwrap();
        let (opened, _) = crate::cloud_hpke::open(
            &carrier,
            crate::limits::CLOUD_ENVELOPE_BYTES,
            &retained_key,
            &retained_descriptor,
            created,
            peer,
        )
        .unwrap();
        assert_eq!(
            opened, exact,
            "carrier created just before expiry opens after renewal"
        );
        assert!(
            reopened
                .recipient_for_document(first.key_id(), first.expires_at_ms(), now)
                .is_err()
        );
        assert!(reopened.recipient(first.key_id(), now).is_err());
        let retired = first.expires_at_ms() + crate::journal::RECOVERY;
        reopened
            .renew_at(retired, || panic!("only pruning, newest key is not due"))
            .unwrap();
        assert!(
            reopened
                .recipient_for_document(first.key_id(), first.expires_at_ms() - 1, retired)
                .is_err()
        );
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert!(
            saved["keys"]
                .as_array()
                .unwrap()
                .iter()
                .all(|k| !k["descriptor"].as_str().unwrap().contains(first.key_id()))
        );
        assert_eq!(
            limits::CLOUD_KEYRING_KEYS as u64,
            (limits::CLOUD_KEY_LIFETIME_MS + crate::journal::RECOVERY).div_ceil(interval)
        );
    }
    #[test]
    fn renewal_failure_is_a_stable_status_until_a_successful_due_tick() {
        let f = Fixture::new();
        f.keys.maintain(NOW).unwrap();
        let file = f.root.join("remote/cloud-keys.json");
        let before = std::fs::read(&file).unwrap();
        std::fs::write(&file, br#"{"version":2}"#).unwrap();
        let failed = NOW + limits::CLOUD_KEY_CHECK_MS;
        assert_eq!(
            f.keys.maintain(failed).unwrap_err().code,
            "REMOTE_CLOUD_UPGRADE_REQUIRED"
        );
        assert_eq!(
            f.keys.renewal_status().unwrap_err().code,
            "REMOTE_CLOUD_UPGRADE_REQUIRED"
        );
        f.keys.maintain(failed + 1).unwrap();
        assert!(
            f.keys.renewal_status().is_err(),
            "a coalesced tick cannot clear the failure"
        );
        std::fs::write(&file, &before).unwrap();
        f.keys
            .maintain(failed + limits::CLOUD_KEY_CHECK_MS)
            .unwrap();
        assert!(f.keys.renewal_status().is_ok());
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }
}
