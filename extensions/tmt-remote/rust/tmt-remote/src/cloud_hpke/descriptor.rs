//! The exact v1 public descriptor; the caller owns the authentication pin/revision floor.
use super::{Error, INTEGER_MAX, PROFILE, Result, SuiteKem, decode, require};
use crate::{canonical, crypto};
use hpke::{Deserializable, Kem, Serializable};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Descriptor {
    version: u64,
    profile: String,
    pub(super) machine_id: String,
    key_id: String,
    revision: u64,
    public_key: String,
    not_before_ms: u64,
    expires_at_ms: u64,
    signature: String,
}
#[derive(Clone, Copy)]
pub struct Pin<'a> {
    pub machine_id: &'a str,
    pub authentication_key: &'a [u8; 32],
}
/// Exact admitted descriptor bytes, including equal-revision byte identity.
/// This is not a peer grant; persist its floor at the existing peer/grant owner.
pub struct VerifiedDescriptor {
    pub(crate) descriptor: Descriptor,
    bytes: Vec<u8>,
    pub(crate) auth: [u8; 32],
}
impl VerifiedDescriptor {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn revision(&self) -> u64 {
        self.descriptor.revision
    }
    pub fn key_id(&self) -> &str {
        &self.descriptor.key_id
    }
    pub fn expires_at_ms(&self) -> u64 {
        self.descriptor.expires_at_ms
    }
    pub(crate) fn valid_at(&self, now: u64) -> Result<()> {
        require(now <= INTEGER_MAX)?;
        if now < self.descriptor.not_before_ms {
            return Err(Error::NotYetValid);
        }
        if now >= self.descriptor.expires_at_ms {
            return Err(Error::DescriptorExpired);
        }
        Ok(())
    }
}
impl Descriptor {
    #[allow(clippy::too_many_arguments)]
    pub fn sign(
        machine_id: &str,
        key_id: &str,
        revision: u64,
        public_key: &[u8; 65],
        not_before_ms: u64,
        expires_at_ms: u64,
        sign: impl FnOnce(&[u8]) -> [u8; 64],
    ) -> Result<Vec<u8>> {
        let mut descriptor = Self {
            version: 1,
            profile: PROFILE.into(),
            machine_id: machine_id.into(),
            key_id: key_id.into(),
            revision,
            public_key: canonical::base64url(public_key),
            not_before_ms,
            expires_at_ms,
            signature: String::new(),
        };
        descriptor.signature = canonical::base64url(&sign(&descriptor.input()?));
        serde_json::to_vec(&descriptor).map_err(|_| Error::Invalid)
    }
    pub fn verify(
        bytes: &[u8],
        pin: Pin<'_>,
        now: u64,
        previous: Option<&VerifiedDescriptor>,
    ) -> Result<VerifiedDescriptor> {
        let descriptor: Self = decode(bytes, 4096)?;
        require(descriptor.machine_id == pin.machine_id)?;
        crypto::verify_signature(
            pin.authentication_key,
            &descriptor.input()?,
            &canonical::base64url_bytes(&descriptor.signature, 64)?,
        )
        .map_err(|_| Error::Signature)?;
        if let Some(old) = previous
            && (old.descriptor.machine_id != descriptor.machine_id
                || old.auth != *pin.authentication_key
                || descriptor.revision < old.revision()
                || (descriptor.revision == old.revision() && bytes != old.bytes()))
        {
            return Err(Error::Revision);
        }
        let verified = VerifiedDescriptor {
            descriptor,
            bytes: bytes.to_vec(),
            auth: *pin.authentication_key,
        };
        verified.valid_at(now)?;
        Ok(verified)
    }
    pub(crate) fn read_retained(bytes: &[u8], pin: Pin<'_>) -> Result<VerifiedDescriptor> {
        let descriptor: Self = decode(bytes, 4096)?;
        Self::verify(bytes, pin, descriptor.not_before_ms, None)
    }
    pub(crate) fn public(&self) -> Result<<SuiteKem as Kem>::PublicKey> {
        let raw = canonical::base64url_bytes(&self.public_key, 65)?;
        require(raw.first() == Some(&4))?;
        let point = <SuiteKem as Kem>::PublicKey::from_bytes(&raw).map_err(|_| Error::Key)?;
        require(point.to_bytes().as_slice() == raw)?;
        Ok(point)
    }
    fn input(&self) -> Result<Vec<u8>> {
        require(
            self.version == 1
                && self.profile == PROFILE
                && self.revision > 0
                && self.revision <= INTEGER_MAX
                && self.not_before_ms < self.expires_at_ms
                && self.expires_at_ms <= INTEGER_MAX,
        )?;
        canonical::uuid(&self.machine_id)?;
        canonical::uuid(&self.key_id)?;
        let public = self.public()?.to_bytes();
        Ok(canonical::framed(&[
            b"tmt-cloud-key-v1",
            b"1",
            PROFILE.as_bytes(),
            self.machine_id.as_bytes(),
            self.key_id.as_bytes(),
            self.revision.to_string().as_bytes(),
            public.as_slice(),
            self.not_before_ms.to_string().as_bytes(),
            self.expires_at_ms.to_string().as_bytes(),
        ])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frozen_pre_change_v1_descriptor_matches_independent_signature_input() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/cloud-hpke-v1.json")).unwrap();
        let bytes = v["descriptor"].as_str().unwrap().as_bytes();
        let public: [u8; 32] = canonical::base64url_bytes(v["ownerPublic"].as_str().unwrap(), 32)
            .unwrap()
            .try_into()
            .unwrap();
        let verified = Descriptor::verify(
            bytes,
            Pin {
                machine_id: "11111111-1111-4111-8111-111111111111",
                authentication_key: &public,
            },
            1_800_000_000_000,
            None,
        )
        .unwrap();
        assert_eq!(verified.bytes(), bytes);
        let input = verified.descriptor.input().unwrap();
        let hex: String = input.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, v["descriptorInput"]);
    }
}
