//! Exact signed membership bytes and hash-chain fencing. Persistence/application are caller-owned.
use crate::{
    Invalid, Result, crypto,
    framing::{fields, frame, text},
    payload::{self, Payload},
    require, values,
};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
#[derive(Debug, PartialEq, Eq)]
pub struct Header<'a> {
    pub space: &'a str,
    pub revision: &'a str,
    pub previous_hash: &'a [u8; 32],
    pub operation: &'a str,
    pub payload_digest: &'a [u8; 32],
}
pub fn input(v: &Header<'_>) -> Result<Vec<u8>> {
    values::space_id(v.space)?;
    let rev = values::decimal(v.revision, false)?;
    crate::auth::operation(v.operation)?;
    require(rev != 1 || *v.previous_hash == [0; 32])?;
    frame(&[
        b"tmt-colab-membership-v1",
        b"1",
        v.space.as_bytes(),
        v.revision.as_bytes(),
        v.previous_hash,
        v.operation.as_bytes(),
        v.payload_digest,
    ])
}
pub fn decode(bytes: &[u8]) -> Result<Header<'_>> {
    let f = fields(bytes, 7, 1024)?;
    require(f[0] == b"tmt-colab-membership-v1" && f[1] == b"1")?;
    let v = Header {
        space: text(f[2])?,
        revision: text(f[3])?,
        previous_hash: f[4].try_into().map_err(|_| Invalid)?,
        operation: text(f[5])?,
        payload_digest: f[6].try_into().map_err(|_| Invalid)?,
    };
    require(input(&v)? == bytes)?;
    Ok(v)
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    statement: String,
    payload: String,
    signature: String,
}
pub struct Envelope {
    statement: Vec<u8>,
    payload: Vec<u8>,
    signature: [u8; 64],
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub revision: u64,
    pub hash: [u8; 32],
    pub owner_member: OwnerMember,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerMember {
    pub id: String,
    pub signing_key: [u8; 32],
    pub encryption_key: [u8; 32],
}
pub struct Verified<'a> {
    pub header: Header<'a>,
    pub payload: Payload,
    pub head: Head,
}
impl Envelope {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= (payload::MAX_BYTES + 1024) * 4 / 3 + 2048)?;
        let v: Wire = serde_json::from_slice(bytes).map_err(|_| Invalid)?;
        let statement = values::binary(&v.statement, 1024)?;
        let h = decode(&statement)?;
        let payload = values::binary(&v.payload, payload::MAX_BYTES)?;
        require(crypto::digest(&payload) == *h.payload_digest)?;
        payload::decode(h.operation, &payload)?;
        Ok(Self {
            statement,
            payload,
            signature: values::binary(&v.signature, 64)?
                .try_into()
                .map_err(|_| Invalid)?,
        })
    }
    pub fn to_json(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&Wire {
            statement: values::encode_binary(&self.statement),
            payload: values::encode_binary(&self.payload),
            signature: values::encode_binary(&self.signature),
        })
        .map_err(|_| Invalid)
    }
    pub fn hash(&self) -> Result<[u8; 32]> {
        Ok(crypto::digest(&frame(&[
            b"tmt-colab-membership-hash-v1",
            &self.statement,
            &self.signature,
        ])?))
    }
    /// Highest retained head and pinned URL space are trusted caller inputs, never backend hints.
    /// Persist the returned head before dependent state; live-issuer/history/transition policy stays external.
    pub fn verify_next<'a>(
        &'a self,
        space: &str,
        owner: &[u8; 32],
        previous: Option<&Head>,
    ) -> Result<Verified<'a>> {
        let h = decode(&self.statement)?;
        require(
            h.space == space
                && crypto::space_id(owner)? == space
                && crypto::digest(&self.payload) == *h.payload_digest,
        )?;
        let rev = values::decimal(h.revision, false)?;
        require(match previous {
            Some(p) => p.revision.checked_add(1) == Some(rev) && p.hash == *h.previous_hash,
            None => rev == 1 && *h.previous_hash == [0; 32],
        })?;
        crypto::verify_signature(owner, &self.statement, &self.signature)?;
        let payload = payload::decode(h.operation, &self.payload)?;
        validate_scope(&payload, space, h.revision, owner)?;
        let owner_member = owner_binding(&payload, previous)?;
        Ok(Verified {
            header: h,
            payload,
            head: Head {
                revision: rev,
                hash: self.hash()?,
                owner_member,
            },
        })
    }
}
/// Exact already-typed payload bytes are signed once; application/owner mutation locking is external.
pub fn sign(
    space: &str,
    previous: Option<&Head>,
    operation: &str,
    payload: &[u8],
    owner: &SigningKey,
) -> Result<Envelope> {
    require(crypto::space_id(owner.verifying_key().as_bytes())? == space)?;
    let revision = previous
        .map_or(Some(1), |h| h.revision.checked_add(1))
        .ok_or(Invalid)?
        .to_string();
    let previous_hash = previous.map_or(&[0; 32], |h| &h.hash);
    let decoded = payload::decode(operation, payload)?;
    owner_binding(&decoded, previous)?;
    validate_scope(&decoded, space, &revision, owner.verifying_key().as_bytes())?;
    let statement = input(&Header {
        space,
        revision: &revision,
        previous_hash,
        operation,
        payload_digest: &crypto::digest(payload),
    })?;
    let signature = owner.sign(&statement).to_bytes();
    Ok(Envelope {
        statement,
        payload: payload.to_vec(),
        signature,
    })
}

fn validate_scope(payload: &Payload, space: &str, revision: &str, owner: &[u8; 32]) -> Result<()> {
    if let Payload::EpochAdvance(v) = payload {
        require(v.baseline.membership_revision == revision)?;
        let epoch = values::decimal(&v.epoch, false)?;
        for cut in v.cuts.as_slice() {
            require(cut.page_id == v.page_id && values::decimal(&cut.epoch, false)? < epoch)?;
        }
        for w in v.wraps.as_slice() {
            let h = w.header()?;
            require(h.space == space && h.signer_key == *owner)?;
            w.verify_owner(owner)?;
        }
    }
    Ok(())
}

fn owner_binding(payload: &Payload, previous: Option<&Head>) -> Result<OwnerMember> {
    // colab-v1 "Local sign-in and owner-device enrollment": revision 1 pins the
    // editor management principal; later member additions cannot reuse its ID or keys.
    match previous {
        None => {
            let Payload::MemberAdd(v) = payload else {
                return Err(Invalid);
            };
            require(matches!(v.role, payload::Role::Editor))?;
            Ok(OwnerMember {
                id: v.member_id.clone(),
                signing_key: values::binary(&v.sign_key, 32)?
                    .try_into()
                    .map_err(|_| Invalid)?,
                encryption_key: values::binary(&v.enc_key, 32)?
                    .try_into()
                    .map_err(|_| Invalid)?,
            })
        }
        Some(head) => {
            if let Payload::MemberAdd(v) = payload {
                require(
                    v.member_id != head.owner_member.id
                        && values::binary(&v.sign_key, 32)? != head.owner_member.signing_key
                        && values::binary(&v.enc_key, 32)? != head.owner_member.encryption_key,
                )?;
            }
            Ok(head.owner_member.clone())
        }
    }
}
