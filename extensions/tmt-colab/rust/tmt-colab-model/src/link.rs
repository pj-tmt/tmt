//! Purpose-separated link keys. A surviving bearer seed remains a capability until link reset.
use crate::{
    Result, certificate, crypto, framing::frame, keys, require, values, wrap::RecipientKey,
};
use ed25519_dalek::{Signer, SigningKey};
pub struct Keys {
    space: String,
    id: String,
    signer: SigningKey,
    recipient: RecipientKey,
    join_proof: [u8; 32],
}
impl Keys {
    pub fn derive(seed: &[u8; 32], space: &str, id: &str) -> Result<Self> {
        values::space_id(space)?;
        values::generated_id(id)?;
        let signing = crypto::derive_key(
            seed,
            &[],
            &frame(&[
                b"tmt-colab-link-signing-seed-v1",
                space.as_bytes(),
                id.as_bytes(),
            ])?,
        );
        let encryption = crypto::derive_key(
            seed,
            &[],
            &frame(&[
                b"tmt-colab-link-encryption-seed-v1",
                space.as_bytes(),
                id.as_bytes(),
            ])?,
        );
        let signer = SigningKey::from_bytes(&signing);
        let recipient = RecipientKey::from_seed(&encryption)?;
        require(recipient.public_key() == keys::x25519_public(&encryption))?;
        Ok(Self {
            space: space.into(),
            id: id.into(),
            signer,
            recipient,
            join_proof: crypto::mac(
                seed,
                &frame(&[b"tmt-colab-join-v1", space.as_bytes(), id.as_bytes()])?,
            ),
        })
    }
    pub fn signing_public(&self) -> [u8; 32] {
        self.signer.verifying_key().to_bytes()
    }
    pub fn recipient(&self) -> &RecipientKey {
        &self.recipient
    }
    pub fn join_proof(&self) -> &[u8; 32] {
        &self.join_proof
    }
    /// Certification has no page/agent authority until the current owner log admits this issuer.
    pub fn certify(&self, device: &certificate::Certificate<'_>) -> Result<[u8; 64]> {
        require(
            device.space == self.space
                && device.issuer_id == self.id
                && device.issuer_kind == "link",
        )?;
        Ok(self.signer.sign(&certificate::input(device)?).to_bytes())
    }
}
