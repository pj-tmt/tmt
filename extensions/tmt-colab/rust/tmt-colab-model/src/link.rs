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
    rules_token: [u8; 32],
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
            rules_token: crypto::derive_key(
                seed,
                &[],
                &frame(&[
                    b"tmt-colab-link-rules-token-v1",
                    space.as_bytes(),
                    id.as_bytes(),
                ])?,
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
    /// The bearer token of the link's cloud `links` row, which a holder presents once to
    /// self-admit. It is derived apart from every other key, so the cloud never sees more
    /// than this one value of the seed.
    pub fn rules_token(&self) -> String {
        values::encode_binary(&self.rules_token)
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

/// The largest time the model accepts: a derived device's certificate never expires, because
/// liveness is the link's.
const DEVICE_EXPIRES_AT: u64 = 9_007_199_254_740_991;
/// The chain a link's reader device presents. The device is a pure function of the seed, so the
/// owner who minted the link and every holder derive the same bytes. `revision` and `statement`
/// name the `link.add` statement that introduced the link.
pub fn device_chain(
    seed: &[u8; 32],
    space: &str,
    id: &str,
    revision: &str,
    statement: &[u8; 32],
) -> Result<Vec<u8>> {
    let link = Keys::derive(seed, space, id)?;
    let derive = |label: &[u8]| -> Result<[u8; 32]> {
        Ok(crypto::derive_key(
            seed,
            &[],
            &frame(&[label, space.as_bytes(), id.as_bytes()])?,
        ))
    };
    let signer = SigningKey::from_bytes(&derive(b"tmt-colab-link-device-seed-v1")?);
    let mut name = [0; 16];
    name.copy_from_slice(&derive(b"tmt-colab-link-device-id-v1")?[..16]);
    name[6] = name[6] & 15 | 64;
    name[8] = name[8] & 63 | 128;
    let hex: String = name.iter().map(|b| format!("{b:02x}")).collect();
    let device_id = format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    );
    let signing_key = signer.verifying_key().to_bytes();
    let encryption_key = link.recipient().public_key();
    let device = certificate::Certificate {
        space,
        issuer_kind: "link",
        issuer_id: id,
        device_id: &device_id,
        signing_key: &signing_key,
        encryption_key: &encryption_key,
        membership_revision: revision,
        issued_at: 0,
        expires_at: DEVICE_EXPIRES_AT,
    };
    serde_json::to_vec(&serde_json::json!({
        "version": 1,
        "issuerStatement": values::encode_binary(statement),
        "deviceCertificate": values::encode_binary(&certificate::input(&device)?),
        "issuerSignature": values::encode_binary(&link.certify(&device)?),
    }))
    .map_err(|_| crate::Invalid)
}
