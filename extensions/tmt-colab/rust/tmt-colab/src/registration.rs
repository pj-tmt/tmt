//! Owner-browser extension-key registration. Remote authenticates the device;
//! this boundary verifies both extension-key certificates before any signing.
use crate::{
    Result,
    keyring::Keyring,
    store::{
        Store,
        owner::{Device, Mutation, OwnerTransaction},
    },
    transitions::Engine,
};
use serde::{Deserialize, Serialize};
use tmt_colab_model::{certificate, crypto, framing, statement, values};

pub const PATH: &str = "/api/devices/register";
pub const FRESHNESS_MS: u64 = 10 * 60 * 1000;
pub const CERTIFICATE_MS: u64 = 365 * 24 * 60 * 60 * 1000;
pub const RENEWAL_MS: u64 = 30 * 24 * 60 * 60 * 1000;
const GENESIS: &str = "00000000-0000-4000-8000-000000000001";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    Invalid,
    Denied,
    Expired,
    Conflict,
    Unavailable,
    Capacity,
}
impl Code {
    pub fn status(self) -> u16 {
        match self {
            Self::Invalid => 400,
            Self::Denied | Self::Expired => 403,
            Self::Conflict => 409,
            Self::Unavailable | Self::Capacity => 503,
        }
    }
    pub fn text(self) -> &'static str {
        match self {
            Self::Invalid => "INVALID",
            Self::Denied => "DENIED",
            Self::Expired => "EXPIRED",
            Self::Conflict => "CONFLICT",
            Self::Unavailable => "UNAVAILABLE",
            Self::Capacity => "CAPACITY",
        }
    }
}
impl std::fmt::Display for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.text())
    }
}
impl std::error::Error for Code {}
impl From<tmt_colab_model::Invalid> for Code {
    fn from(_: tmt_colab_model::Invalid) -> Self {
        Self::Invalid
    }
}

/// Trusted only when read from remote's header on the owner-only mount socket.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Context {
    device_id: String,
    kind: String,
    origin: String,
    name: String,
    public_key: String,
    owner: bool,
    grant_revision: u64,
}
impl Context {
    fn parse(raw: Option<&str>) -> std::result::Result<Self, Code> {
        let c: Self = serde_json::from_str(raw.ok_or(Code::Denied)?).map_err(|_| Code::Denied)?;
        if !c.owner
            || c.grant_revision == 0
            || !matches!(c.kind.as_str(), "browser" | "addon" | "cli")
        {
            return Err(Code::Denied);
        }
        values::generated_id(&c.device_id).map_err(|_| Code::Denied)?;
        c.key()?;
        Ok(c)
    }
    fn key(&self) -> std::result::Result<[u8; 32], Code> {
        let key = binary::<32>(&self.public_key).map_err(|_| Code::Denied)?;
        crypto::public_key(&key).map_err(|_| Code::Denied)?;
        Ok(key)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Request {
    device_id: String,
    sign: KeyCertificate,
    enc: KeyCertificate,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct KeyCertificate {
    public_key: String,
    issued_at_ms: u64,
    signature: String,
}
fn binary<const N: usize>(text: &str) -> std::result::Result<[u8; N], Code> {
    values::binary(text, N)?
        .try_into()
        .map_err(|_| Code::Invalid)
}
/// Remote owns this layout (canonical::ext_cert); the shared model LP primitive
/// implements its byte rules without importing a Remote behavior dependency.
fn ext_cert(purpose: &str, key: &[u8; 32], issued: u64) -> std::result::Result<Vec<u8>, Code> {
    values::time(issued)?;
    Ok(framing::frame(&[
        b"tmt-ext-cert-v1",
        b"colab",
        purpose.as_bytes(),
        key,
        issued.to_string().as_bytes(),
    ])?)
}
impl KeyCertificate {
    fn verify(
        &self,
        purpose: &str,
        context: &[u8; 32],
        now: u64,
    ) -> std::result::Result<[u8; 32], Code> {
        let key = binary(&self.public_key)?;
        let signature = binary::<64>(&self.signature)?;
        let input = ext_cert(purpose, &key, self.issued_at_ms)?;
        if self.issued_at_ms > now || now - self.issued_at_ms > FRESHNESS_MS {
            return Err(Code::Expired);
        }
        crypto::verify_signature(context, &input, &signature).map_err(|_| Code::Denied)?;
        if purpose == "sign" {
            crypto::public_key(&key)?;
        }
        if purpose == "enc" && key == [0; 32] {
            return Err(Code::Invalid);
        }
        Ok(key)
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DeviceRegistration {
    context: Context,
    signing: [u8; 32],
    encryption: [u8; 32],
    outcome: Vec<u8>,
}

pub struct Registration {
    store: Store,
    keyring: Keyring,
    engine: Engine,
    readers: crate::readers::Sessions,
    reader_clock: std::sync::Arc<dyn Fn() -> std::result::Result<u64, Code> + Send + Sync>,
}
impl Registration {
    pub fn new(
        store: Store,
        keyring: Keyring,
        decoder_program: std::path::PathBuf,
    ) -> Result<Self> {
        Self::with_decoder_config(store, keyring, crate::decoder::Config::new(decoder_program))
    }
    pub fn with_decoder_config(
        store: Store,
        keyring: Keyring,
        decoder_config: crate::decoder::Config,
    ) -> Result<Self> {
        Ok(Self {
            store,
            keyring,
            engine: Engine::with_decoder_config(decoder_config)?,
            readers: Default::default(),
            reader_clock: std::sync::Arc::new(now_ms),
        })
    }
    /// Server-owned reader clock, injected for deterministic expiry verification.
    pub fn with_reader_clock(
        mut self,
        clock: impl Fn() -> std::result::Result<u64, Code> + Send + Sync + 'static,
    ) -> Self {
        self.reader_clock = std::sync::Arc::new(clock);
        self
    }
    pub(crate) fn reader_request(
        &mut self,
        path: &str,
        body: &[u8],
    ) -> std::result::Result<Vec<u8>, Code> {
        let now = (self.reader_clock)()?;
        if path == crate::readers::CHALLENGE_PATH {
            self.readers
                .challenge(&self.store, &self.keyring, body, now)
        } else {
            self.readers
                .exchange(&mut self.store, &self.keyring, body, now)
        }
    }
    pub(crate) fn reader_upgrade(
        &mut self,
        token: &[u8; 32],
    ) -> std::result::Result<(String, String), Code> {
        self.readers
            .upgrade(&self.store, &self.keyring, token, (self.reader_clock)()?)
    }
    pub(crate) fn release_reader(&mut self, id: &str) {
        self.readers.release(id);
    }
    pub fn close(self) -> Result<()> {
        self.store.close()
    }
    /// Read-only owner discovery also works before extension-key registration.
    pub fn session(context: Option<&str>) -> std::result::Result<Vec<u8>, Code> {
        let c = Context::parse(context)?;
        serde_json::to_vec(
            &serde_json::json!({"deviceId":c.device_id,"publicKey":c.public_key,
            "grantRevision":c.grant_revision.to_string(),"name":c.name}),
        )
        .map_err(|_| Code::Unavailable)
    }
    pub fn pages(&self, context: Option<&str>) -> std::result::Result<Vec<u8>, Code> {
        Context::parse(context)?;
        self.store
            .owner_read(&self.keyring.space_id, &self.keyring.owner_public(), |tx| {
                Ok(serde_json::to_vec(&tx.page_list()?)?)
            })
            .map_err(map_error)
    }
    /// The caller serializes management with sync before taking this mutex.
    pub fn apply_owner(
        &mut self,
        request: crate::transitions::OwnerRequest<'_>,
        now: u64,
    ) -> std::result::Result<crate::transitions::Applied, crate::transitions::TransitionError> {
        self.engine
            .apply(&mut self.store, &self.keyring, request, now)
    }
    /// Root-local ciphertext write; caller holds the sync mutex before this service.
    pub(crate) fn page_write(
        &mut self,
        prepared: &crate::page::Prepared,
        now: u64,
    ) -> crate::Result<crate::page::Committed> {
        crate::page::commit(&mut self.store, &self.keyring, prepared, now)
    }
    pub(crate) fn management_device(
        &mut self,
        context: Option<&str>,
        now: u64,
    ) -> std::result::Result<(String, [u8; 32]), Code> {
        let device = Context::parse(context)?.device_id;
        let signing = self.active_device(context, now)?;
        Ok((device, signing))
    }

    /// The caller supplies server time, never a request-selected clock.
    pub fn register(
        &mut self,
        context: Option<&str>,
        body: &[u8],
        now: u64,
    ) -> std::result::Result<Vec<u8>, Code> {
        values::time(now)?;
        if body.len() > crate::limits::HTTP_BODY_BYTES {
            return Err(Code::Invalid);
        }
        let context = Context::parse(context)?;
        let request: Request = serde_json::from_slice(body).map_err(|_| Code::Invalid)?;
        values::generated_id(&request.device_id)?;
        if request.device_id != context.device_id {
            return Err(Code::Denied);
        }
        let remote_key = context.key()?;
        let signing = request.sign.verify("sign", &remote_key, now)?;
        let encryption = request.enc.verify("enc", &remote_key, now)?;
        self.genesis().map_err(map_error)?;
        let keyring = &self.keyring;
        let apply = |tx: &mut OwnerTransaction<'_>| -> Result<Vec<u8>> {
            let (initial, member, membership_revision) = issuer(tx, keyring)?;
            let old_device = tx.device(&context.device_id)?;
            if old_device.as_ref().is_some_and(|d| d.revoked) {
                return Err(Code::Denied.into());
            }
            if let Some(row) = tx.registration(&context.device_id)? {
                if row.revoked || context.grant_revision < row.grant_revision {
                    return Err(Code::Denied.into());
                }
                let mut saved: DeviceRegistration =
                    serde_json::from_slice(&row.binding.ok_or(Code::Unavailable)?)?;
                if saved.context.public_key != context.public_key
                    || saved.signing != signing
                    || saved.encryption != encryption
                {
                    return Err(Code::Conflict.into());
                }
                let device = old_device.as_ref().ok_or(Code::Unavailable)?;
                let chain = certificate::Chain::from_json(&device.chain)?;
                let cert = chain.certificate()?;
                verify_device(
                    &chain,
                    &cert,
                    &initial,
                    &member,
                    &saved,
                    membership_revision,
                    &keyring.space_id,
                )?;
                if now < cert.issued_at {
                    return Err(Code::Expired.into());
                }
                if cert.expires_at.saturating_sub(now) >= RENEWAL_MS {
                    saved.context = context;
                    tx.put_registration(
                        &saved.context.device_id,
                        &serde_json::to_vec(&saved)?,
                        saved.context.grant_revision,
                    )?;
                    forward_owner_wraps(tx, keyring, &saved.context.device_id, &encryption)?;
                    return Ok(saved.outcome);
                }
            } else if old_device.is_some() {
                return Err(Code::Conflict.into());
            }
            let revision = membership_revision.to_string();
            let cert = certificate::Certificate {
                space: &keyring.space_id,
                issuer_kind: "member",
                issuer_id: &member.id,
                device_id: &context.device_id,
                signing_key: &signing,
                encryption_key: &encryption,
                membership_revision: &revision,
                issued_at: now,
                expires_at: now.checked_add(CERTIFICATE_MS).ok_or(Code::Invalid)?,
            };
            let chain = serde_json::json!({
                "version":1,
                "issuerStatement":values::encode_binary(&initial.hash()?),
                "deviceCertificate":values::encode_binary(&certificate::input(&cert)?),
                "issuerSignature":values::encode_binary(&keyring.sign_device_certificate(&cert)?)
            });
            let chain_bytes = serde_json::to_vec(&chain)?;
            certificate::Chain::from_json(&chain_bytes)?.verify(
                &initial.hash()?,
                &cert,
                &member.signing_key,
            )?;
            let statement = serde_json::from_slice::<serde_json::Value>(&initial.to_json()?)?;
            let outcome = serde_json::to_vec(
                &serde_json::json!({"chain":chain,"issuerStatement":statement}),
            )?;
            tx.put_device(&Device {
                chain: chain_bytes,
                revoked: false,
            })?;
            let binding = DeviceRegistration {
                context,
                signing,
                encryption,
                outcome: outcome.clone(),
            };
            tx.put_registration(
                &binding.context.device_id,
                &serde_json::to_vec(&binding)?,
                binding.context.grant_revision,
            )?;
            forward_owner_wraps(tx, keyring, &binding.context.device_id, &encryption)?;
            Ok(outcome)
        };
        self.store
            .device_transaction(&keyring.space_id, &keyring.owner_public(), apply)
            .map_err(map_error)
    }
    fn genesis(&mut self) -> Result<()> {
        let key = &self.keyring;
        if self
            .store
            .owner_head(&key.space_id, &key.owner_public())?
            .is_some()
        {
            return Ok(());
        }
        let (_, payload) = crate::transitions::owner_genesis(key)?;
        self.store.owner_transaction(
            &key.space_id,
            &key.owner_public(),
            Mutation {
                operation_id: GENESIS,
                digest: crypto::digest(&payload),
                expected_revision: 0,
            },
            |tx| {
                Ok(crate::transitions::initialize_owner(tx, key)?
                    .ok_or(Code::Unavailable)?
                    .to_json()?)
            },
        )?;
        Ok(())
    }
    /// Trusted remote consumer: known devices commit cuts/rotation and a tombstone;
    /// unknown IDs only tombstone. Equal/older or already-revoked events return false.
    pub fn revoke(&mut self, device_id: &str, grant_revision: u64) -> Result<bool> {
        values::generated_id(device_id)?;
        if grant_revision == 0 {
            return Err(crate::store::owner::OwnerFault::Invalid.into());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
            .try_into()?;
        for _ in 0..3 {
            let (known, noop) = self.store.owner_read(
                &self.keyring.space_id,
                &self.keyring.owner_public(),
                |tx| {
                    let device = tx.device(device_id)?;
                    let row = tx.registration(device_id)?;
                    Ok((
                        device.is_some(),
                        device.is_some_and(|d| d.revoked)
                            || row.is_some_and(|r| r.revoked || r.grant_revision >= grant_revision),
                    ))
                },
            )?;
            if noop {
                return Ok(false);
            }
            if known {
                return self.engine.remote_revoke(
                    &mut self.store,
                    &self.keyring,
                    device_id,
                    grant_revision,
                    now,
                );
            }
            match self.store.revoke_remote_device(
                &self.keyring.space_id,
                &self.keyring.owner_public(),
                device_id,
                grant_revision,
            ) {
                Err(e)
                    if e.downcast_ref::<crate::store::owner::OwnerFault>()
                        == Some(&crate::store::owner::OwnerFault::StaleHead) =>
                {
                    continue;
                }
                other => return other,
            }
        }
        Err(crate::store::owner::OwnerFault::StaleHead.into())
    }
    /// Admission for socket/sync composition, checked against durable
    /// revocation every time. A forwarded cookie/session alone is insufficient.
    pub fn active_device(
        &mut self,
        context: Option<&str>,
        now: u64,
    ) -> std::result::Result<[u8; 32], Code> {
        values::time(now)?;
        let context = Context::parse(context)?;
        let key = &self.keyring;
        if self
            .store
            .owner_head(&key.space_id, &key.owner_public())
            .map_err(map_error)?
            .is_none()
        {
            return Err(Code::Denied);
        }
        let bytes = self
            .store
            .device_transaction(&key.space_id, &key.owner_public(), |tx| {
                let row = tx.registration(&context.device_id)?.ok_or(Code::Denied)?;
                if row.revoked || context.grant_revision < row.grant_revision {
                    return Err(Code::Denied.into());
                }
                let mut binding: DeviceRegistration =
                    serde_json::from_slice(&row.binding.ok_or(Code::Denied)?)?;
                if binding.context.public_key != context.public_key {
                    return Err(Code::Denied.into());
                }
                let device = tx.device(&context.device_id)?.ok_or(Code::Denied)?;
                if device.revoked {
                    return Err(Code::Denied.into());
                }
                let chain = certificate::Chain::from_json(&device.chain)?;
                let cert = chain.certificate()?;
                let (initial, member, revision) = issuer(tx, key)?;
                verify_device(
                    &chain,
                    &cert,
                    &initial,
                    &member,
                    &binding,
                    revision,
                    &key.space_id,
                )?;
                if now < cert.issued_at || now >= cert.expires_at {
                    return Err(Code::Expired.into());
                }
                if context.grant_revision > row.grant_revision {
                    binding.context = context;
                    tx.put_registration(
                        &binding.context.device_id,
                        &serde_json::to_vec(&binding)?,
                        binding.context.grant_revision,
                    )?;
                }
                Ok(binding.signing.to_vec())
            })
            .map_err(map_error)?;
        bytes.try_into().map_err(|_| Code::Unavailable)
    }
}
/// Mounted owner and read-only reader admission. Upgrade verifies owner binding
/// through active_device or consumes a reader ticket; sync reads live authority.
/// The pinned local management member represents the owner across local pages.
pub struct OwnerAdmission(pub std::sync::Arc<std::sync::Mutex<Registration>>);
impl crate::sync::Admission for OwnerAdmission {
    fn alive(&self, principal: &str) -> std::result::Result<(), crate::sync::Code> {
        let service = self.0.lock().map_err(|_| crate::sync::Code::Denied)?;
        if service.readers.contains(principal) {
            service
                .readers
                .check(
                    principal,
                    None,
                    &service.store,
                    &service.keyring,
                    (service.reader_clock)().map_err(|_| crate::sync::Code::Denied)?,
                )
                .map_err(reader_sync_error)?;
        }
        Ok(())
    }

    fn catchup_context(
        &self,
        principal: &str,
        scope: &crate::sync::SyncScope,
        store: &Store,
    ) -> std::result::Result<crate::sync::CatchupContext, crate::sync::Code> {
        let service = self.0.lock().map_err(|_| crate::sync::Code::Denied)?;
        service
            .store
            .owner_read(&scope.space, &service.keyring.owner_public(), |tx| {
                let head = tx.head().ok_or(crate::sync::Code::Denied)?;
                if tx.page_policy_at(&scope.page, head.revision)?.deleted {
                    return Err(crate::sync::Code::Denied.into());
                }
                Ok(())
            })
            .map_err(|_| crate::sync::Code::Denied)?;
        Ok(crate::sync::CatchupContext {
            membership_head: store
                .owner_head(&scope.space, &service.keyring.owner_public())
                .map_err(|_| crate::sync::Code::Denied)?
                .ok_or(crate::sync::Code::Denied)?,
            baseline: store
                .baseline(
                    &scope.page,
                    values::decimal(&scope.epoch, false).map_err(|_| crate::sync::Code::Denied)?,
                )
                .map_err(|_| crate::sync::Code::Denied)?
                .map(|baseline| baseline.descriptor),
            owner_key: service.keyring.owner_public(),
            recipients: if service.readers.contains(principal) {
                service
                    .readers
                    .check(
                        principal,
                        Some(scope),
                        &service.store,
                        &service.keyring,
                        (service.reader_clock)().map_err(|_| crate::sync::Code::Denied)?,
                    )
                    .map_err(reader_sync_error)?;
                service
                    .readers
                    .recipients(principal)
                    .map_err(reader_sync_error)?
            } else {
                crate::sync::WrapRecipients::Owner
            },
        })
    }

    fn authorize(
        &self,
        principal: &str,
        scope: &crate::sync::SyncScope,
        access: crate::sync::Access<'_>,
    ) -> std::result::Result<[u8; 32], crate::sync::Code> {
        use crate::sync::Code as SyncCode;
        let service = self.0.lock().map_err(|_| SyncCode::Denied)?;
        if scope.space != service.keyring.space_id {
            return Err(SyncCode::Denied);
        }
        let now = now_ms().map_err(|_| SyncCode::Denied)?;
        if service.readers.contains(principal) {
            service
                .readers
                .check(
                    principal,
                    Some(scope),
                    &service.store,
                    &service.keyring,
                    (service.reader_clock)().map_err(|_| SyncCode::Denied)?,
                )
                .map_err(reader_sync_error)?;
            return if matches!(access, crate::sync::Access::Read) {
                Ok([0; 32])
            } else {
                Err(SyncCode::Denied)
            };
        }
        service
            .store
            .owner_read(&scope.space, &service.keyring.owner_public(), |tx| {
                let row = tx.registration(principal)?.ok_or(SyncCode::Denied)?;
                if row.revoked {
                    return Err(SyncCode::Denied.into());
                }
                let binding: DeviceRegistration =
                    serde_json::from_slice(&row.binding.ok_or(SyncCode::Denied)?)?;
                let device = tx.device(principal)?.ok_or(SyncCode::Denied)?;
                if device.revoked {
                    return Err(SyncCode::Denied.into());
                }
                let chain = certificate::Chain::from_json(&device.chain)?;
                let cert = chain.certificate()?;
                let head = tx.head().ok_or(SyncCode::Denied)?;
                let member = &head.owner_member;
                let issuer = tx
                    .recipient("member", &member.id)?
                    .ok_or(SyncCode::Denied)?;
                if issuer.revoked
                    || issuer.role.as_deref() != Some("editor")
                    || issuer.signing_key != member.signing_key
                    || issuer.encryption_key != member.encryption_key
                    || cert.device_id != principal
                    || cert.space != scope.space
                    || cert.issuer_kind != "member"
                    || cert.issuer_id != member.id
                    || cert.signing_key != &binding.signing
                    || cert.encryption_key != &binding.encryption
                    || values::decimal(cert.membership_revision, false)? > head.revision
                {
                    return Err(SyncCode::Denied.into());
                }
                if now < cert.issued_at || now >= cert.expires_at {
                    return Err(SyncCode::Expired.into());
                }
                let policy = tx.page_policy_at(&scope.page, head.revision)?;
                if policy.deleted
                    || (policy.archived
                        && matches!(
                            access,
                            crate::sync::Access::Append(_) | crate::sync::Access::Publish
                        ))
                {
                    return Err(SyncCode::Denied.into());
                }
                if tx.page_epoch(&scope.page)?.as_deref() != Some(scope.epoch.as_str()) {
                    return Err(SyncCode::StaleEpoch.into());
                }
                if let crate::sync::Access::Append(c) = access
                    && (values::decimal(&c.membership_revision, false)? != head.revision
                        || !matches!(c.namespace.as_str(), "content" | "own"))
                {
                    return Err(SyncCode::Denied.into());
                }
                Ok(binding.signing)
            })
            .map_err(|error| {
                error
                    .downcast_ref::<SyncCode>()
                    .copied()
                    .unwrap_or(SyncCode::Denied)
            })
    }
}

/// Registration admission already authenticated the owner device and its pinned keys.
/// Wrap additions share its device transaction; failed capacity/signing publishes none.
fn forward_owner_wraps(
    tx: &mut OwnerTransaction<'_>,
    key: &Keyring,
    device: &str,
    recipient_key: &[u8; 32],
) -> Result<()> {
    let head = tx.head().ok_or(Code::Unavailable)?.clone();
    let pages = tx.pages()?;
    if pages.len() > crate::limits::PAGES {
        return Err(Code::Capacity.into());
    }
    let mut added = 0;
    for page in pages {
        let policy = tx.page_policy_at(&page, head.revision)?;
        if !policy.writable() {
            continue;
        }
        let epochs = tx.recent_secrets(&page, policy.epoch)?;
        for (epoch, mut secret) in epochs {
            let result = (|| -> Result<()> {
                if (policy.history_current && epoch != policy.epoch)
                    || tx.device_wrap_exists(&page, epoch, device, recipient_key)?
                {
                    return Ok(());
                }
                added += 1;
                if added > crate::limits::OWNER_WRAPS {
                    return Err(Code::Capacity.into());
                }
                tx.put_wrap(&key.seal_wrap(
                    &tmt_colab_model::wrap::Header {
                        space: key.space_id.clone(),
                        page: page.clone(),
                        epoch: epoch.to_string(),
                        recipient_kind: "device".into(),
                        recipient_id: device.into(),
                        recipient_key: *recipient_key,
                        signer_key: key.owner_public(),
                        membership_revision: head.revision.to_string(),
                    },
                    &secret,
                )?)?;
                Ok(())
            })();
            secret.fill(0);
            result?;
        }
    }
    Ok(())
}

fn issuer(
    tx: &OwnerTransaction<'_>,
    keyring: &Keyring,
) -> Result<(statement::Envelope, statement::OwnerMember, u64)> {
    let head = tx.head().ok_or(Code::Unavailable)?;
    let member = keyring.management_member()?;
    if head.owner_member != member {
        return Err(Code::Unavailable.into());
    }
    let initial = tx.statement(1)?.ok_or(Code::Unavailable)?;
    let verified = initial.verify_next(&keyring.space_id, &keyring.owner_public(), None)?;
    if verified.head.owner_member != member {
        return Err(Code::Unavailable.into());
    }
    let issuer = tx
        .recipient("member", &member.id)?
        .ok_or(Code::Unavailable)?;
    if issuer.revoked
        || issuer.signing_key != member.signing_key
        || issuer.encryption_key != member.encryption_key
        || issuer.role.as_deref() != Some("editor")
    {
        return Err(Code::Denied.into());
    }
    Ok((initial, member, head.revision))
}
fn verify_device(
    chain: &certificate::Chain,
    cert: &certificate::Certificate<'_>,
    initial: &statement::Envelope,
    member: &statement::OwnerMember,
    binding: &DeviceRegistration,
    revision: u64,
    space: &str,
) -> Result<()> {
    if cert.device_id != binding.context.device_id || cert.space != space {
        return Err(Code::Denied.into());
    }
    if cert.issuer_kind != "member"
        || cert.issuer_id != member.id
        || cert.signing_key != &binding.signing
        || cert.encryption_key != &binding.encryption
        || values::decimal(cert.membership_revision, false)? > revision
    {
        return Err(Code::Denied.into());
    }
    chain.verify(&initial.hash()?, cert, &member.signing_key)?;
    Ok(())
}
fn map_error(error: Box<dyn std::error::Error + Send + Sync>) -> Code {
    error
        .downcast_ref::<Code>()
        .copied()
        .unwrap_or(Code::Unavailable)
}

pub(crate) fn now_ms() -> std::result::Result<u64, Code> {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Code::Unavailable)?
        .as_millis()
        .try_into()
        .map_err(|_| Code::Unavailable)
}

fn reader_sync_error(code: Code) -> crate::sync::Code {
    match code {
        Code::Expired => crate::sync::Code::Expired,
        _ => crate::sync::Code::Denied,
    }
}
