//! Native cloud byte cryptography. Verification grants no authority and performs no I/O.
mod carrier;
mod descriptor;
pub use carrier::{ResponseOpener, ResponseSealer, Sender, open, seal};
pub use descriptor::{Descriptor, Pin, VerifiedDescriptor};
use hpke::{Kem, Serializable};

pub(crate) type SuiteKem = hpke::kem::DhP256HkdfSha256;
pub(crate) type SuiteAead = hpke::aead::AesGcm128;
pub(crate) type SuiteKdf = hpke::kdf::HkdfSha256;
pub(crate) const PROFILE: &str = "cloud-hpke-v1";
pub(crate) const INTEGER_MAX: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    UpgradeRequired,
    DescriptorExpired,
    NotYetValid,
    Revision,
    Signature,
    Key,
    Bounds,
    Envelope,
    Crypto,
}
impl Error {
    pub fn code(self) -> &'static str {
        match self {
            Self::UpgradeRequired => "REMOTE_CLOUD_UPGRADE_REQUIRED",
            Self::DescriptorExpired => "REMOTE_CLOUD_DESCRIPTOR_EXPIRED",
            Self::NotYetValid => "REMOTE_CLOUD_DESCRIPTOR_NOT_YET_VALID",
            Self::Revision => "REMOTE_CLOUD_DESCRIPTOR_REVISION",
            Self::Signature => "REMOTE_CLOUD_SIGNATURE_INVALID",
            Self::Key => "REMOTE_CLOUD_KEY_UNAVAILABLE",
            Self::Bounds => "REMOTE_CLOUD_BOUNDS",
            Self::Envelope => "REMOTE_CLOUD_ENVELOPE_INVALID",
            Self::Crypto | Self::Invalid => "REMOTE_CLOUD_CARRIER_INVALID",
        }
    }
    /// Refresh/observe the descriptor; never resend an operation on this evidence alone.
    pub fn retryable(self) -> bool {
        matches!(self, Self::DescriptorExpired | Self::NotYetValid)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if matches!(self, Self::UpgradeRequired) {
            f.write_str("REMOTE_CLOUD_UPGRADE_REQUIRED: upgrade Remote; pairing is unchanged")
        } else {
            f.write_str(self.code())
        }
    }
}
impl std::error::Error for Error {}
impl From<crate::canonical::InvalidBytes> for Error {
    fn from(_: crate::canonical::InvalidBytes) -> Self {
        Self::Invalid
    }
}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn require(valid: bool) -> Result<()> {
    if valid { Ok(()) } else { Err(Error::Invalid) }
}
pub(crate) fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8], cap: usize) -> Result<T> {
    if bytes.len() > cap {
        return Err(Error::Bounds);
    }
    let value = crate::wire::strict_json(bytes).ok_or(Error::Invalid)?;
    if value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|v| v != 1)
    {
        return Err(Error::UpgradeRequired);
    }
    serde_json::from_value(value).map_err(|_| Error::Invalid)
}
/// Distinct from the stable Ed25519 identity. Its private scalar zeroizes on drop.
/// Derivation input must be 32 fresh CSPRNG bytes; persistence belongs to cloud_keys.
pub struct RecipientKey(pub(crate) <SuiteKem as Kem>::PrivateKey);
impl RecipientKey {
    pub fn from_ikm(ikm: &[u8; 32]) -> Self {
        Self(SuiteKem::derive_keypair(ikm).0)
    }
    pub fn public(&self) -> [u8; 65] {
        SuiteKem::sk_to_pk(&self.0)
            .to_bytes()
            .as_slice()
            .try_into()
            .expect("fixed P-256 public size")
    }
}
