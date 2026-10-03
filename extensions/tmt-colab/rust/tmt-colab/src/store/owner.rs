//! Owner authority persistence, not session or transition-policy admission.
//! All mutations use one writer transaction; exact results survive lost replies.
mod bootstrap;
pub(crate) mod epoch;
pub use epoch::{Cut, StoredBaseline};

use super::{Store, sequence};
use crate::Result;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use tmt_colab_model::{certificate, crypto, payload, statement, values, wrap};

/// Caller-admitted recipients for scoped bootstrap; never inferred from a frame.
pub enum WrapRecipients {
    Owner,
    Link(String),
    None,
}
pub const MAX_OUTCOME_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum OwnerFault {
    Conflict,
    StaleHead,
    WrongOwner,
    Invalid,
    Capacity,
}
impl std::fmt::Display for OwnerFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Owner store: {self:?}")
    }
}
impl std::error::Error for OwnerFault {}

/// Digest of the exact admitted request, including its sender and context.
/// Zero expected revision is reserved for the initial owner-member statement.
pub struct Mutation<'a> {
    pub operation_id: &'a str,
    pub digest: [u8; 32],
    pub expected_revision: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipient {
    pub kind: String,
    pub id: String,
    pub role: Option<String>,
    pub signing_key: [u8; 32],
    pub encryption_key: [u8; 32],
    pub pages: Vec<String>,
    pub revoked: bool,
}
impl Recipient {
    fn validate(&self) -> Result<()> {
        let (operation, value) = match self.kind.as_str() {
            "member" => (
                "member.add",
                serde_json::json!({
                "memberId": self.id, "role": self.role,
                "signKey": values::encode_binary(&self.signing_key),
                "encKey": values::encode_binary(&self.encryption_key), "pages": self.pages}),
            ),
            "link" => (
                "link.add",
                serde_json::json!({
                "linkId": self.id, "role": self.role,
                "linkSignKey": values::encode_binary(&self.signing_key),
                "linkEncKey": values::encode_binary(&self.encryption_key), "pages": self.pages}),
            ),
            "bridge" if self.role.is_none() => (
                "bridge.add",
                serde_json::json!({
                "machineId": self.id, "machineSignKey": values::encode_binary(&self.signing_key),
                "encKey": values::encode_binary(&self.encryption_key), "pages": self.pages}),
            ),
            _ => return Err(OwnerFault::Invalid.into()),
        };
        payload::decode(operation, &serde_json::to_vec(&value)?)?;
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    /// Exact model certificate-chain transport. The caller verifies its live issuer.
    pub chain: Vec<u8>,
    pub revoked: bool,
}

pub(crate) struct RegistrationRow {
    pub binding: Option<Vec<u8>>,
    pub revoked: bool,
    pub grant_revision: u64,
}

pub struct OwnerTransaction<'a> {
    tx: &'a Transaction<'a>,
    space: &'a str,
    root: &'a [u8; 32],
    head: Option<statement::Head>,
}
impl Store {
    /// Local retained head, not proof of a globally current membership view.
    pub fn owner_head(&self, space: &str, root: &[u8; 32]) -> Result<Option<statement::Head>> {
        if crypto::space_id(root)? != space {
            return Err(OwnerFault::WrongOwner.into());
        }
        read_head(&self.connection, space, root)
    }
    /// Call only after request/session/expiry admission. A replay still requires
    /// that admission, but ignores the now-stale revision and never calls `apply`.
    /// Returning Err (or unwinding) rolls back statements, projections and receipt.
    pub fn owner_transaction(
        &mut self,
        space: &str,
        root: &[u8; 32],
        mutation: Mutation<'_>,
        apply: impl FnOnce(&mut OwnerTransaction<'_>) -> Result<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        values::generated_id(mutation.operation_id)?;
        if crypto::space_id(root)? != space {
            return Err(OwnerFault::WrongOwner.into());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let head = read_head(&tx, space, root)?;
        let saved: Option<(Vec<u8>, Vec<u8>)> = tx
            .query_row(
                "SELECT digest,outcome FROM owner_operations WHERE id=?",
                [mutation.operation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((digest, outcome)) = saved {
            return if digest == mutation.digest {
                Ok(outcome)
            } else {
                Err(OwnerFault::Conflict.into())
            };
        }
        if head.as_ref().map_or(0, |h| h.revision) != mutation.expected_revision {
            return Err(OwnerFault::StaleHead.into());
        }
        let mut owner = OwnerTransaction {
            tx: &tx,
            space,
            root,
            head,
        };
        let outcome = apply(&mut owner)?;
        if owner
            .head
            .as_ref()
            .is_none_or(|h| h.revision <= mutation.expected_revision)
        {
            return Err(OwnerFault::Invalid.into());
        }
        if outcome.len() > MAX_OUTCOME_BYTES {
            return Err(OwnerFault::Capacity.into());
        }
        tx.execute(
            "INSERT INTO owner_operations VALUES (?,?,?)",
            params![mutation.operation_id, mutation.digest.as_slice(), outcome],
        )?;
        tx.commit()?;
        Ok(outcome)
    }
    /// Registration changes a device projection, not membership authority. Genesis
    /// must already have been committed through owner_transaction.
    pub(crate) fn device_transaction(
        &mut self,
        space: &str,
        root: &[u8; 32],
        apply: impl FnOnce(&mut OwnerTransaction<'_>) -> Result<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        if crypto::space_id(root)? != space {
            return Err(OwnerFault::WrongOwner.into());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let head = read_head(&tx, space, root)?.ok_or(OwnerFault::Invalid)?;
        let outcome = apply(&mut OwnerTransaction {
            tx: &tx,
            space,
            root,
            head: Some(head),
        })?;
        tx.commit()?;
        Ok(outcome)
    }
    /// Trusted remote event consumer only; no HTTP route grants this capability.
    /// Unknown IDs alone use this local tombstone; known devices require the engine.
    pub(crate) fn revoke_remote_device(
        &mut self,
        space: &str,
        root: &[u8; 32],
        id: &str,
        grant_revision: u64,
    ) -> Result<bool> {
        values::generated_id(id)?;
        if grant_revision == 0 {
            return Err(OwnerFault::Invalid.into());
        }
        if crypto::space_id(root)? != space {
            return Err(OwnerFault::WrongOwner.into());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        read_head(&tx, space, root)?;
        let revision = sequence(grant_revision);
        let previous: Option<(String, bool)> = tx
            .query_row(
                "SELECT grant_revision,revoked FROM device_registrations WHERE device_id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        // Level-triggered remote replay must not rewrite an equal revision.
        if previous.is_some_and(|(old, revoked)| revoked || old >= revision) {
            return Ok(false);
        }
        let known: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?)",
            [id],
            |r| r.get(0),
        )?;
        if known {
            return Err(OwnerFault::StaleHead.into());
        }
        tx.execute("INSERT INTO device_registrations VALUES (?,NULL,1,?)
            ON CONFLICT(device_id) DO UPDATE SET binding=NULL,revoked=1,grant_revision=excluded.grant_revision", params![id,revision])?;
        tx.commit()?;
        Ok(true)
    }
}
impl OwnerTransaction<'_> {
    /// Create-only page identity, committed with its authority, secret and content.
    pub(crate) fn create_page(&mut self, page: &str) -> Result<()> {
        values::generated_id(page)?;
        if self.page_epoch(page)?.is_some() {
            return Err(OwnerFault::Conflict.into());
        }
        let count: i64 = self
            .tx
            .query_row("SELECT count(*) FROM pages", [], |r| r.get(0))?;
        if count >= crate::limits::PAGES as i64 {
            return Err(OwnerFault::Capacity.into());
        }
        self.tx
            .execute("INSERT INTO pages VALUES (?,'1')", [page])?;
        Ok(())
    }

    pub(crate) fn page_epoch(&self, page: &str) -> Result<Option<String>> {
        Ok(self
            .tx
            .query_row("SELECT epoch FROM pages WHERE page=?", [page], |r| r.get(0))
            .optional()?)
    }
    pub(crate) fn append_content(
        &mut self,
        envelope: &super::Envelope<'_>,
    ) -> Result<super::Accepted> {
        Ok(super::append_in(self.tx, envelope)?.ok_or(super::Fault::Conflict)?)
    }
    pub(crate) fn save_operation(
        &mut self,
        id: &str,
        digest: &[u8; 32],
        outcome: &[u8],
    ) -> Result<()> {
        values::generated_id(id)?;
        if outcome.len() > MAX_OUTCOME_BYTES {
            return Err(OwnerFault::Capacity.into());
        }
        self.tx.execute(
            "INSERT INTO owner_operations VALUES (?,?,?)",
            params![id, digest.as_slice(), outcome],
        )?;
        Ok(())
    }
    pub fn head(&self) -> Option<&statement::Head> {
        self.head.as_ref()
    }

    /// Verifies exact bytes against the pinned owner and the transaction's head.
    /// Semantic transition policy remains the caller's responsibility.
    pub fn append_statement(&mut self, envelope: &statement::Envelope) -> Result<()> {
        let verified = envelope.verify_next(self.space, self.root, self.head.as_ref())?;
        let head = verified.head;
        self.tx.execute(
            "INSERT INTO membership_log VALUES (?,?,?)",
            params![
                sequence(head.revision),
                head.hash.as_slice(),
                envelope.to_json()?
            ],
        )?;
        self.tx.execute(
            "INSERT INTO owner_state VALUES (1,?,?,?,?,?,?,?)
            ON CONFLICT(singleton) DO UPDATE SET revision=excluded.revision,hash=excluded.hash",
            params![
                self.space,
                self.root.as_slice(),
                sequence(head.revision),
                head.hash.as_slice(),
                head.owner_member.id,
                head.owner_member.signing_key.as_slice(),
                head.owner_member.encryption_key.as_slice()
            ],
        )?;
        self.head = Some(head);
        Ok(())
    }
    pub fn statement(&self, revision: u64) -> Result<Option<statement::Envelope>> {
        let bytes: Option<Vec<u8>> = self
            .tx
            .query_row(
                "SELECT envelope FROM membership_log WHERE revision=?",
                [sequence(revision)],
                |r| r.get(0),
            )
            .optional()?;
        bytes
            .map(|b| statement::Envelope::from_json(&b).map_err(Into::into))
            .transpose()
    }
    pub fn put_recipient(&mut self, recipient: &Recipient) -> Result<()> {
        recipient.validate()?;
        self.tx.execute(
            "INSERT INTO recipients VALUES (?,?,?)
            ON CONFLICT(kind,id) DO UPDATE SET record=excluded.record",
            params![recipient.kind, recipient.id, serde_json::to_vec(recipient)?],
        )?;
        Ok(())
    }
    pub fn recipient(&self, kind: &str, id: &str) -> Result<Option<Recipient>> {
        let bytes: Option<Vec<u8>> = self
            .tx
            .query_row(
                "SELECT record FROM recipients WHERE kind=? AND id=?",
                params![kind, id],
                |r| r.get(0),
            )
            .optional()?;
        bytes
            .map(|b| serde_json::from_slice(&b).map_err(Into::into))
            .transpose()
    }
    pub fn put_device(&mut self, device: &Device) -> Result<()> {
        let chain = certificate::Chain::from_json(&device.chain)?;
        let cert = chain.certificate()?;
        if cert.space != self.space {
            return Err(OwnerFault::WrongOwner.into());
        }
        self.tx.execute(
            "INSERT INTO devices VALUES (?,?) ON CONFLICT(id) DO UPDATE SET record=excluded.record",
            params![cert.device_id, serde_json::to_vec(device)?],
        )?;
        Ok(())
    }
    pub fn device(&self, id: &str) -> Result<Option<Device>> {
        let bytes: Option<Vec<u8>> = self
            .tx
            .query_row("SELECT record FROM devices WHERE id=?", [id], |r| r.get(0))
            .optional()?;
        bytes
            .map(|b| serde_json::from_slice(&b).map_err(Into::into))
            .transpose()
    }

    pub(crate) fn registration(&self, id: &str) -> Result<Option<RegistrationRow>> {
        self.tx
            .query_row(
                "SELECT binding,revoked,grant_revision FROM device_registrations WHERE device_id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, String>(2)?)),
            )
            .optional()?
            .map(|(binding, revoked, revision)| -> Result<_> {
                Ok(RegistrationRow {
                    binding,
                    revoked,
                    grant_revision: revision.parse()?,
                })
            })
            .transpose()
    }
    pub(crate) fn put_registration(
        &mut self,
        id: &str,
        binding: &[u8],
        revision: u64,
    ) -> Result<()> {
        self.tx.execute("INSERT INTO device_registrations VALUES (?,?,0,?)
            ON CONFLICT(device_id) DO UPDATE SET binding=excluded.binding,grant_revision=excluded.grant_revision", params![id,binding,sequence(revision)])?;
        Ok(())
    }
    /// Retains an epoch secret without moving the page's current epoch. The
    /// transition caller advances it in this same closure after baseline admission.
    pub fn put_epoch_secret(&mut self, page: &str, epoch: u64, secret: &[u8; 32]) -> Result<()> {
        values::generated_id(page)?;
        if epoch == 0 {
            return Err(OwnerFault::Invalid.into());
        }
        if let Some(old) = self.epoch_secret(page, epoch)? {
            return if old == *secret {
                Ok(())
            } else {
                Err(OwnerFault::Conflict.into())
            };
        }
        self.tx.execute(
            "INSERT INTO epoch_secrets VALUES (?,?,?)",
            params![page, sequence(epoch), secret.as_slice()],
        )?;
        Ok(())
    }
    pub fn epoch_secret(&self, page: &str, epoch: u64) -> Result<Option<[u8; 32]>> {
        let bytes: Option<Vec<u8>> = self
            .tx
            .query_row(
                "SELECT secret FROM epoch_secrets WHERE page=? AND epoch=?",
                params![page, sequence(epoch)],
                |r| r.get(0),
            )
            .optional()?;
        bytes
            .map(|b| b.try_into().map_err(|_| OwnerFault::Invalid.into()))
            .transpose()
    }
    pub fn advance_epoch(&mut self, page: &str, expected: u64) -> Result<()> {
        let next = expected.checked_add(1).ok_or(OwnerFault::Invalid)?;
        if self.epoch_secret(page, next)?.is_none() {
            return Err(OwnerFault::Invalid.into());
        }
        super::advance_epoch(self.tx, page, expected)?;
        Ok(())
    }
    /// Create-only by complete wrap context. Different bytes at the same context
    /// are a conflict, even if the newly randomized wrap decrypts to the same key.
    pub fn put_wrap(&mut self, envelope: &wrap::Envelope) -> Result<()> {
        envelope.verify_owner(self.root)?;
        let h = envelope.header()?;
        let revision = values::decimal(&h.membership_revision, false)?;
        if h.space != self.space || self.head.as_ref().is_none_or(|v| revision > v.revision) {
            return Err(OwnerFault::Invalid.into());
        }
        // Forward joins carry their join revision, not a caller-selected policy.
        // Historical wraps remain valid for recipients who already held them.
        let policy = self.page_policy_at(&h.page, revision)?;
        let epoch = values::decimal(&h.epoch, false)?;
        if policy.deleted
            || epoch > policy.epoch
            || epoch < policy.epoch.saturating_sub(63).max(1)
            || (policy.history_current && epoch != policy.epoch)
        {
            return Err(OwnerFault::Invalid.into());
        }
        if let Some(old) = self.wrap(&h)? {
            return if old == *envelope {
                Ok(())
            } else {
                Err(OwnerFault::Conflict.into())
            };
        }
        self.tx.execute(
            "INSERT INTO wraps VALUES (?,?,?,?,?,?)",
            params![
                h.page,
                sequence(values::decimal(&h.epoch, false)?),
                h.recipient_kind,
                h.recipient_id,
                sequence(revision),
                envelope.to_json()?
            ],
        )?;
        Ok(())
    }
    /// Existing recipient/epoch coverage survives unrelated owner-head changes.
    pub(crate) fn device_wrap_exists(
        &self,
        page: &str,
        epoch: u64,
        id: &str,
        key: &[u8; 32],
    ) -> Result<bool> {
        let bytes: Option<Vec<u8>> = self.tx.query_row(
            "SELECT envelope FROM wraps WHERE page=? AND epoch=? AND kind='device' AND recipient=? ORDER BY revision DESC LIMIT 1",
            params![page,sequence(epoch),id], |r| r.get(0),
        ).optional()?;
        let Some(bytes) = bytes else {
            return Ok(false);
        };
        let wrapped = wrap::Envelope::from_json(&bytes)?;
        wrapped.verify_owner(self.root)?;
        let h = wrapped.header()?;
        if h.space != self.space
            || h.page != page
            || h.epoch != epoch.to_string()
            || h.recipient_kind != "device"
            || h.recipient_id != id
            || h.recipient_key != *key
        {
            return Err(OwnerFault::Conflict.into());
        }
        Ok(true)
    }
    pub fn wrap(&self, header: &wrap::Header) -> Result<Option<wrap::Envelope>> {
        header.encode()?;
        if header.space != self.space || header.signer_key != *self.root {
            return Err(OwnerFault::WrongOwner.into());
        }
        let bytes: Option<Vec<u8>> = self.tx.query_row(
            "SELECT envelope FROM wraps WHERE page=? AND epoch=? AND kind=? AND recipient=? AND revision=?",
            params![header.page, sequence(values::decimal(&header.epoch,false)?), header.recipient_kind,
                header.recipient_id, sequence(values::decimal(&header.membership_revision,false)?)], |r| r.get(0),
        ).optional()?;
        bytes
            .map(|b| wrap::Envelope::from_json(&b).map_err(Into::into))
            .transpose()
    }
}
fn read_head(
    connection: &Connection,
    space: &str,
    root: &[u8; 32],
) -> Result<Option<statement::Head>> {
    let row = connection.query_row(
        "SELECT space,root,revision,hash,member_id,member_sign,member_enc FROM owner_state WHERE singleton=1",
        [], |r| Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,String>(2)?,
            r.get::<_,Vec<u8>>(3)?,r.get::<_,String>(4)?,r.get::<_,Vec<u8>>(5)?,r.get::<_,Vec<u8>>(6)?)),
    ).optional()?;
    let Some((stored_space, stored_root, revision, hash, id, signing, encryption)) = row else {
        return Ok(None);
    };
    if stored_space != space || stored_root != root {
        return Err(OwnerFault::WrongOwner.into());
    }
    let key = |v: Vec<u8>| v.try_into().map_err(|_| OwnerFault::Invalid);
    Ok(Some(statement::Head {
        revision: revision.parse()?,
        hash: key(hash)?,
        owner_member: statement::OwnerMember {
            id,
            signing_key: key(signing)?,
            encryption_key: key(encryption)?,
        },
    }))
}
