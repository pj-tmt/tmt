//! Local code proof/possession admission. Durable consumption precedes session delivery.
use crate::{
    Result,
    keyring::{Keyring, MemberKeys},
    store::{
        Store,
        auth::{Enrollment, OwnerGenesis},
    },
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::Signer;
use tmt_colab_model::{
    auth::{SignIn, verify_signin},
    crypto, values,
};

pub(crate) struct SignInCode {
    pub id: String,
    pub space: String,
    secret: [u8; 16],
    expires: u64,
    issuer_statement: [u8; 32],
}
impl Drop for SignInCode {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}
pub(crate) struct CertifiedSession {
    pub chain: Vec<u8>,
    pub token: String,
}
impl SignInCode {
    pub(crate) fn issue(
        store: &mut Store,
        keyring: &Keyring,
        member: &MemberKeys,
        now: u64,
    ) -> Result<Self> {
        let space = keyring.space_id.as_str();
        values::space_id(space)?;
        values::time(now)?;
        let expires = now.checked_add(600_000).ok_or("Sign-in expiry overflow.")?;
        values::time(expires)?;
        let mut id = [0; 16];
        getrandom::fill(&mut id)?;
        id[6] = (id[6] & 0x0f) | 0x40;
        id[8] = (id[8] & 0x3f) | 0x80;
        let hex = |part: &[u8]| part.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let id = format!(
            "{}-{}-{}-{}-{}",
            hex(&id[..4]),
            hex(&id[4..6]),
            hex(&id[6..8]),
            hex(&id[8..10]),
            hex(&id[10..])
        );
        let mut secret = [0; 16];
        if let Err(error) = getrandom::fill(&mut secret) {
            secret.fill(0);
            return Err(error.into());
        }
        let mut code = Self {
            id,
            space: space.into(),
            secret,
            expires,
            issuer_statement: [0; 32],
        };
        store.start_auth()?;
        let signing_public = member.signing.verifying_key().to_bytes();
        let payload = serde_json::to_vec(
            &serde_json::json!({"memberId":member.id,"role":"editor","signKey":URL_SAFE_NO_PAD.encode(signing_public),"encKey":URL_SAFE_NO_PAD.encode(member.encryption_public),"pages":[]}),
        )?;
        let envelope = keyring.genesis(&payload)?;
        let verified = envelope.verify_next(space, &keyring.owner_public(), None)?;
        let encoded = envelope.to_json()?;
        store.initialize_owner(OwnerGenesis {
            space,
            envelope: &encoded,
            head: &verified.head,
        })?;
        code.issuer_statement = verified.head.hash;
        store.issue_signin(&code.id, space, now, expires)?;
        Ok(code)
    }
    /// Intentional single-use foreground bootstrap output; never an HTTP transport value.
    pub(crate) fn fragment(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.secret)
    }
    pub(crate) fn enroll(
        &mut self,
        store: &mut Store,
        member: &MemberKeys,
        input: &SignIn<'_>,
        proof: &[u8],
        possession: &[u8],
        now: u64,
    ) -> Result<CertifiedSession> {
        values::time(now)?;
        if now >= self.expires || input.code_id != self.id || input.space != self.space {
            return Err("Sign-in denied.".into());
        }
        verify_signin(&self.secret, input, proof, possession)?;
        let expires = now
            .checked_add(86_400_000)
            .ok_or("Session expiry overflow.")?;
        values::time(expires)?;
        let head = store.membership_head(&self.space)?;
        if head.hash != self.issuer_statement
            || head.owner_member.id != member.id
            || head.owner_member.signing_key != member.signing.verifying_key().to_bytes()
            || head.owner_member.encryption_key != member.encryption_public
        {
            return Err("Sign-in issuer is no longer live.".into());
        }
        let revision = head.revision.to_string();
        let expected = tmt_colab_model::certificate::Certificate {
            space: &self.space,
            issuer_kind: "member",
            issuer_id: &member.id,
            device_id: input.device,
            signing_key: input.signing_key,
            encryption_key: input.encryption_key,
            membership_revision: &revision,
            issued_at: now,
            expires_at: expires,
        };
        let certificate = tmt_colab_model::certificate::input(&expected)?;
        let signature = member.signing.sign(&certificate).to_bytes();
        let chain = serde_json::to_vec(
            &serde_json::json!({"version":1,"issuerStatement":URL_SAFE_NO_PAD.encode(self.issuer_statement),"deviceCertificate":URL_SAFE_NO_PAD.encode(&certificate),"issuerSignature":URL_SAFE_NO_PAD.encode(signature)}),
        )?;
        tmt_colab_model::certificate::Chain::from_json(&chain)?.verify(
            &self.issuer_statement,
            &expected,
            &member.signing.verifying_key().to_bytes(),
        )?;
        let mut token = [0; 32];
        if let Err(error) = getrandom::fill(&mut token) {
            token.fill(0);
            return Err(error.into());
        }
        let token_hash = crypto::digest(&token);
        let result = store.enroll(Enrollment {
            code: &self.id,
            space: &self.space,
            device: input.device,
            signing_key: input.signing_key,
            encryption_key: input.encryption_key,
            certificate: &chain,
            token_hash: &token_hash,
            now,
            expires,
        });
        let encoded = URL_SAFE_NO_PAD.encode(token);
        token.fill(0);
        result?;
        self.secret.fill(0);
        self.expires = 0;
        Ok(CertifiedSession {
            chain,
            token: encoded,
        })
    }
}

#[cfg(test)]
mod tests;
