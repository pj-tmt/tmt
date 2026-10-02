//! Pure colab-v1 byte/crypto primitives. Syntax and signatures do not grant authority.
//! Callers own log, session, role, epoch, sequence and aggregate-quota admission.
//! Two crypto-only OS-entropy uses: seal object IDs and HPKE ephemeral keys;
//! no filesystem, process or network access, and no long-term key-generation RNG.
pub mod auth;
pub mod bounded;
pub mod certificate;
pub mod crypto;
pub mod framing;
pub mod keys;
pub mod link;
pub mod object;
pub mod payload;
pub mod statement;
pub mod stream_cut;
pub mod values;
pub mod wrap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Invalid;
impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid colab-v1 value or cryptographic proof")
    }
}
impl std::error::Error for Invalid {}
pub type Result<T> = std::result::Result<T, Invalid>;
pub(crate) fn require(valid: bool) -> Result<()> {
    if valid { Ok(()) } else { Err(Invalid) }
}
