//! Strict native carriers with consuming response contexts. Replay/frozen-response ownership
//! stays with the caller's existing journal; reopening a carrier is not permission to reseal.
use super::{
    Error, PROFILE, RecipientKey, Result, SuiteAead, SuiteKdf, SuiteKem, VerifiedDescriptor,
    decode, require,
};
use crate::{
    canonical::{self, Envelope},
    crypto,
    wire::SignedMessage,
};
use aes_gcm::{
    Aes128Gcm, KeyInit,
    aead::{Aead, Payload},
};
use hpke::{Deserializable, Kem, Serializable};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
pub struct Sender<'a> {
    /// Native peer identity at the receiver; never a Firebase uid or browser key.
    pub client_id: &'a str,
    pub authentication_key: &'a [u8; 32],
}
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Header {
    version: u64,
    profile: String,
    kind: String,
    id: String,
    correlation_id: Value,
    machine_id: String,
    window_id: String,
    client_id: String,
    session_id: String,
    sequence: String,
    timestamp_ms: u64,
    origin: String,
    operation: String,
}
impl Header {
    fn from_message(message: &SignedMessage) -> Self {
        let e = message.envelope();
        Self {
            version: 1,
            profile: "local-v1".into(),
            kind: e.kind.into(),
            id: e.id.into(),
            correlation_id: e
                .correlation_id
                .map_or(Value::Null, |id| Value::String(id.into())),
            machine_id: e.machine_id.into(),
            window_id: e.window_id.into(),
            client_id: e.client_id.into(),
            session_id: e.session_id.into(),
            sequence: e.sequence.into(),
            timestamp_ms: e.timestamp_ms,
            origin: e.origin.into(),
            operation: e.operation.into(),
        }
    }
    fn input(&self) -> Result<Vec<u8>> {
        require(self.version == 1 && self.profile == "local-v1" && self.origin == "cli")?;
        require(self.correlation_id.is_null() || self.correlation_id.is_string())?;
        Ok(canonical::envelope_header(&Envelope {
            kind: &self.kind,
            id: &self.id,
            correlation_id: self.correlation_id.as_str(),
            machine_id: &self.machine_id,
            window_id: &self.window_id,
            client_id: &self.client_id,
            session_id: &self.session_id,
            sequence: &self.sequence,
            timestamp_ms: self.timestamp_ms,
            origin: &self.origin,
            operation: &self.operation,
            payload: &[],
        })?)
    }
    fn response_to(&self, request: &Self) -> Result<()> {
        require(
            self.kind == "response"
                && self.correlation_id.as_str() == Some(request.id.as_str())
                && self.machine_id == request.machine_id
                && self.window_id == request.window_id
                && self.client_id == request.client_id
                && self.operation == request.operation
                && (self.session_id == request.session_id
                    || (request.kind == "control"
                        && request.operation == "session.open"
                        && request.session_id == "new")),
        )
    }
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Carrier {
    version: u64,
    profile: String,
    key_id: String,
    header: Header,
    enc: String,
    ciphertext: String,
    signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_digest: Option<String>,
}
impl Carrier {
    fn binding(&self) -> Result<Vec<u8>> {
        require(self.version == 1 && self.profile == PROFILE)?;
        canonical::uuid(&self.key_id)?;
        let h = self.header.input()?;
        if h.len() > crate::limits::METADATA_BYTES {
            return Err(Error::Bounds);
        }
        Ok(canonical::framed(&[
            b"tmt-cloud-carrier-v1",
            b"1",
            PROFILE.as_bytes(),
            self.key_id.as_bytes(),
            &h,
        ])?)
    }
    fn aad(&self) -> Result<Vec<u8>> {
        let mut b = self.binding()?;
        canonical::lp(&mut b, &canonical::base64url_bytes(&self.enc, 65)?)?;
        if let Some(digest) = &self.request_digest {
            canonical::lp(&mut b, &canonical::base64url_bytes(digest, 32)?)?;
        }
        Ok(b)
    }
    fn signature_input(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let mut input = self.aad()?;
        canonical::lp(&mut input, ciphertext.len().to_string().as_bytes())?;
        canonical::lp(&mut input, &Sha256::digest(ciphertext))?;
        Ok(input)
    }
    fn write(
        &mut self,
        ct: Vec<u8>,
        limit: usize,
        auth: &[u8; 32],
        sign: impl FnOnce(&[u8]) -> [u8; 64],
    ) -> Result<Vec<u8>> {
        let input = self.signature_input(&ct)?;
        let signature = sign(&input);
        crypto::verify_signature(auth, &input, &signature).map_err(|_| Error::Signature)?;
        self.ciphertext = canonical::base64url(&ct);
        self.signature = canonical::base64url(&signature);
        let bytes = serde_json::to_vec(self).map_err(|_| Error::Invalid)?;
        if bytes.len() > wire_cap(limit)?
            || bytes.len().saturating_sub(self.ciphertext.len()) > crate::limits::METADATA_BYTES
        {
            return Err(Error::Bounds);
        }
        Ok(bytes)
    }
    fn read(
        bytes: &[u8],
        limit: usize,
        response: bool,
        auth: &[u8; 32],
    ) -> Result<(Self, Vec<u8>)> {
        let c: Self = decode(bytes, wire_cap(limit)?)?;
        if bytes.len().saturating_sub(c.ciphertext.len()) > crate::limits::METADATA_BYTES {
            return Err(Error::Bounds);
        }
        // Option alone would conflate an omitted request member and explicit null.
        let value = crate::wire::strict_json(bytes).ok_or(Error::Invalid)?;
        require(if response {
            value.get("requestDigest").is_some_and(Value::is_string)
        } else {
            value.get("requestDigest").is_none()
        })?;
        require((c.header.kind == "response") == response)?;
        c.binding()?;
        let enc = canonical::base64url_bytes(&c.enc, 65)?;
        require(enc.first() == Some(&4))?;
        let point = <SuiteKem as Kem>::EncappedKey::from_bytes(&enc).map_err(|_| Error::Key)?;
        require(point.to_bytes().as_slice() == enc)?;
        let ct = canonical::base64url_decode(&c.ciphertext)?;
        if ct.len() < 16 || ct.len() > limit.checked_add(16).ok_or(Error::Bounds)? {
            return Err(Error::Bounds);
        }
        crypto::verify_signature(
            auth,
            &c.signature_input(&ct)?,
            &canonical::base64url_bytes(&c.signature, 64)?,
        )
        .map_err(|_| Error::Signature)?;
        Ok((c, ct))
    }
}
fn wire_cap(n: usize) -> Result<usize> {
    n.checked_add(16)
        .and_then(|v| v.div_ceil(3).checked_mul(4))
        .and_then(|v| v.checked_add(4 * 65_usize.div_ceil(3) + crate::limits::METADATA_BYTES))
        .ok_or(Error::Bounds)
}
fn message(bytes: &[u8], limit: usize, auth: &[u8; 32]) -> Result<Header> {
    wire_cap(limit)?;
    if bytes.len() > limit {
        return Err(Error::Bounds);
    }
    let signed = SignedMessage::decode_any(bytes, limit).ok_or(Error::Envelope)?;
    crypto::verify_signature(
        auth,
        &canonical::envelope(&signed.envelope())?,
        &signed.signature,
    )
    .map_err(|_| Error::Signature)?;
    let header = Header::from_message(&signed);
    header.input()?;
    Ok(header)
}
struct ResponseSecret {
    key: [u8; 16],
    nonce: [u8; 12],
}
impl Drop for ResponseSecret {
    fn drop(&mut self) {
        self.key.fill(0);
        self.nonce.fill(0);
    }
}
impl ResponseSecret {
    fn export(
        digest: &[u8; 32],
        mut export: impl FnMut(&[u8], &mut [u8]) -> std::result::Result<(), hpke::HpkeError>,
    ) -> Result<Self> {
        let e = canonical::framed(&[b"tmt-cloud-response-v1", digest])?;
        let mut result = Self {
            key: [0; 16],
            nonce: [0; 12],
        };
        let mut key = e.clone();
        canonical::lp(&mut key, b"key")?;
        let mut nonce = e;
        canonical::lp(&mut nonce, b"nonce")?;
        export(&key, &mut result.key).map_err(|_| Error::Crypto)?;
        export(&nonce, &mut result.nonce).map_err(|_| Error::Crypto)?;
        Ok(result)
    }
}
struct ResponseBinding {
    secret: ResponseSecret,
    digest: [u8; 32],
    enc: String,
    key_id: String,
    request: Header,
}
/// Consuming response encryption; persist the returned frozen bytes before publication.
/// ```compile_fail
/// fn reuse(ctx: tmt_remote::cloud_hpke::ResponseSealer) {
///     let _ = ctx.seal(b"{}", 100, |_| [0; 64]);
///     let _ = ctx.seal(b"{}", 100, |_| [0; 64]);
/// }
/// ```
pub struct ResponseSealer {
    binding: ResponseBinding,
    auth: [u8; 32],
}
/// Consuming observation. Lost contexts use the existing ID-based recovery owner, never resealing.
pub struct ResponseOpener {
    binding: ResponseBinding,
    auth: [u8; 32],
}
impl ResponseSealer {
    pub fn seal(
        self,
        exact: &[u8],
        limit: usize,
        sign: impl FnOnce(&[u8]) -> [u8; 64],
    ) -> Result<Vec<u8>> {
        let b = self.binding;
        let header = message(exact, limit, &self.auth)?;
        header.response_to(&b.request)?;
        let mut carrier = Carrier {
            version: 1,
            profile: PROFILE.into(),
            key_id: b.key_id,
            header,
            enc: b.enc,
            ciphertext: String::new(),
            signature: String::new(),
            request_digest: Some(canonical::base64url(&b.digest)),
        };
        let ct = Aes128Gcm::new((&b.secret.key).into())
            .encrypt(
                (&b.secret.nonce).into(),
                Payload {
                    msg: exact,
                    aad: &carrier.aad()?,
                },
            )
            .map_err(|_| Error::Crypto)?;
        carrier.write(ct, limit, &self.auth, sign)
    }
}
impl ResponseOpener {
    pub fn open(self, bytes: &[u8], limit: usize) -> Result<Vec<u8>> {
        let b = self.binding;
        let owner_auth = &self.auth;
        let (carrier, ct) = Carrier::read(bytes, limit, true, owner_auth)?;
        require(
            carrier.key_id == b.key_id
                && carrier.enc == b.enc
                && carrier.request_digest.as_deref()
                    == Some(canonical::base64url(&b.digest).as_str()),
        )?;
        carrier.header.response_to(&b.request)?;
        let exact = Aes128Gcm::new((&b.secret.key).into())
            .decrypt(
                (&b.secret.nonce).into(),
                Payload {
                    msg: &ct,
                    aad: &carrier.aad()?,
                },
            )
            .map_err(|_| Error::Crypto)?;
        require(message(&exact, limit, owner_auth)? == carrier.header)?;
        Ok(exact)
    }
}
/// Caller supplies a fresh CSPRNG; this pure library does not hide entropy failures or perform I/O.
#[allow(clippy::too_many_arguments)]
pub fn seal(
    exact: &[u8],
    limit: usize,
    descriptor: &VerifiedDescriptor,
    now: u64,
    sender: Sender<'_>,
    sign: impl FnOnce(&[u8]) -> [u8; 64],
    rng: &mut impl hpke::rand_core::CryptoRng,
) -> Result<(Vec<u8>, ResponseOpener)> {
    descriptor.valid_at(now)?;
    let header = message(exact, limit, sender.authentication_key)?;
    require(
        matches!(header.kind.as_str(), "request" | "control")
            && header.client_id == sender.client_id
            && header.machine_id == descriptor.descriptor.machine_id,
    )?;
    let mut carrier = Carrier {
        version: 1,
        profile: PROFILE.into(),
        key_id: descriptor.key_id().into(),
        header,
        enc: String::new(),
        ciphertext: String::new(),
        signature: String::new(),
        request_digest: None,
    };
    let (enc, mut context) = hpke::setup_sender_with_rng::<SuiteAead, SuiteKdf, SuiteKem>(
        &hpke::OpModeS::Base,
        &descriptor.descriptor.public()?,
        &carrier.binding()?,
        rng,
    )
    .map_err(|_| Error::Crypto)?;
    carrier.enc = canonical::base64url(enc.to_bytes().as_slice());
    let ct = context
        .seal(exact, &carrier.aad()?)
        .map_err(|_| Error::Crypto)?;
    let digest: [u8; 32] = Sha256::digest(carrier.signature_input(&ct)?).into();
    let secret = ResponseSecret::export(&digest, |info, out| context.export(info, out))?;
    let bytes = carrier.write(ct, limit, sender.authentication_key, sign)?;
    Ok((
        bytes,
        ResponseOpener {
            binding: ResponseBinding {
                secret,
                digest,
                enc: carrier.enc,
                key_id: carrier.key_id,
                request: carrier.header,
            },
            auth: descriptor.auth,
        },
    ))
}
/// Fresh-carrier cryptography only. Before reopening an already-adopted carrier, the caller
/// must return its journal's frozen response, never use another sealer for changed plaintext.
pub fn open(
    bytes: &[u8],
    limit: usize,
    key: &RecipientKey,
    descriptor: &VerifiedDescriptor,
    now: u64,
    sender: Sender<'_>,
) -> Result<(Vec<u8>, ResponseSealer)> {
    descriptor.valid_at(now)?;
    require(key.public().as_slice() == descriptor.descriptor.public()?.to_bytes().as_slice())?;
    let (carrier, ct) = Carrier::read(bytes, limit, false, sender.authentication_key)?;
    require(
        carrier.key_id == descriptor.key_id()
            && carrier.header.client_id == sender.client_id
            && carrier.header.machine_id == descriptor.descriptor.machine_id,
    )?;
    let enc =
        <SuiteKem as Kem>::EncappedKey::from_bytes(&canonical::base64url_bytes(&carrier.enc, 65)?)
            .map_err(|_| Error::Key)?;
    let mut context = hpke::setup_receiver::<SuiteAead, SuiteKdf, SuiteKem>(
        &hpke::OpModeR::Base,
        &key.0,
        &enc,
        &carrier.binding()?,
    )
    .map_err(|_| Error::Crypto)?;
    let exact = context
        .open(&ct, &carrier.aad()?)
        .map_err(|_| Error::Crypto)?;
    require(message(&exact, limit, sender.authentication_key)? == carrier.header)?;
    let digest: [u8; 32] = Sha256::digest(carrier.signature_input(&ct)?).into();
    let secret = ResponseSecret::export(&digest, |info, out| context.export(info, out))?;
    Ok((
        exact,
        ResponseSealer {
            binding: ResponseBinding {
                secret,
                digest,
                enc: carrier.enc,
                key_id: carrier.key_id,
                request: carrier.header,
            },
            auth: descriptor.auth,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/cloud-hpke-v1.json")).unwrap()
    }
    fn raw(v: &Value, key: &str) -> Vec<u8> {
        canonical::base64url_decode(v[key].as_str().unwrap()).unwrap()
    }
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
    #[test]
    fn independent_application_bindings_signatures_and_response_framing() {
        let v = fixture();
        let sender: [u8; 32] = raw(&v, "senderPublic").try_into().unwrap();
        let owner: [u8; 32] = raw(&v, "ownerPublic").try_into().unwrap();
        let (carrier, ct) = Carrier::read(
            v["carrier"].as_str().unwrap().as_bytes(),
            4096,
            false,
            &sender,
        )
        .unwrap();
        assert_eq!(hex(&carrier.header.input().unwrap()), v["headerInput"]);
        assert_eq!(hex(&carrier.binding().unwrap()), v["binding"]);
        assert_eq!(hex(&carrier.aad().unwrap()), v["requestAad"]);
        assert_eq!(
            hex(&carrier.signature_input(&ct).unwrap()),
            v["requestSignatureInput"]
        );
        let digest = Sha256::digest(carrier.signature_input(&ct).unwrap()).into();
        let secret = |s: &str| {
            let text = v[s].as_str().unwrap();
            text.as_bytes()
                .chunks_exact(2)
                .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
                .collect::<Vec<_>>()
        };
        let binding = || ResponseBinding {
            secret: ResponseSecret {
                key: secret("responseKey").try_into().unwrap(),
                nonce: secret("responseNonce").try_into().unwrap(),
            },
            digest,
            enc: carrier.enc.clone(),
            key_id: carrier.key_id.clone(),
            request: carrier.header.clone(),
        };
        let response = v["response"].as_str().unwrap().as_bytes();
        let writer = ResponseSealer {
            binding: binding(),
            auth: owner,
        };
        let bytes = writer
            .seal(response, 4096, |b| {
                SigningKey::from_bytes(&[7; 32]).sign(b).to_bytes()
            })
            .unwrap();
        assert_eq!(bytes, v["responseCarrier"].as_str().unwrap().as_bytes());
        let (answer, ct) = Carrier::read(&bytes, 4096, true, &owner).unwrap();
        assert_eq!(hex(&answer.aad().unwrap()), v["responseAad"]);
        assert_eq!(
            hex(&answer.signature_input(&ct).unwrap()),
            v["responseSignatureInput"]
        );
        let reader = ResponseOpener {
            binding: binding(),
            auth: owner,
        };
        assert_eq!(reader.open(&bytes, 4096).unwrap(), response);
        let mut wrong_request = binding();
        wrong_request.digest[0] ^= 1;
        assert!(
            ResponseOpener {
                binding: wrong_request,
                auth: owner
            }
            .open(&bytes, 4096)
            .is_err()
        );
        let mut wrong_enc = binding();
        wrong_enc.enc = canonical::base64url(&[4; 65]);
        assert!(
            ResponseOpener {
                binding: wrong_enc,
                auth: owner
            }
            .open(&bytes, 4096)
            .is_err()
        );

        let context = canonical::framed(&[b"tmt-cloud-response-v1", &digest]).unwrap();
        for (name, label) in [
            ("exporterKeyContext", b"key".as_slice()),
            ("exporterNonceContext", b"nonce".as_slice()),
        ] {
            let mut bytes = context.clone();
            canonical::lp(&mut bytes, label).unwrap();
            assert_eq!(hex(&bytes), v[name]);
        }
    }
    #[test]
    fn strict_null_unknown_noncanonical_and_signed_tamper_controls() {
        let v = fixture();
        let sender: [u8; 32] = raw(&v, "senderPublic").try_into().unwrap();
        let good: Value = serde_json::from_str(v["carrier"].as_str().unwrap()).unwrap();
        for member in ["requestDigest", "unexpected"] {
            let mut bad = good.clone();
            bad[member] = Value::Null;
            assert!(
                Carrier::read(&serde_json::to_vec(&bad).unwrap(), 4096, false, &sender).is_err()
            );
        }
        for field in ["enc", "ciphertext", "signature"] {
            let mut bad = good.clone();
            bad[field] = Value::String(format!("{}=", bad[field].as_str().unwrap()));
            assert!(
                Carrier::read(&serde_json::to_vec(&bad).unwrap(), 4096, false, &sender).is_err()
            );
        }
        let mut bad = good.clone();
        bad["header"]["operation"] = Value::String("agents.get".into());
        assert!(matches!(
            Carrier::read(&serde_json::to_vec(&bad).unwrap(), 4096, false, &sender),
            Err(Error::Signature)
        ));
        let mut bad = good.clone();
        let mut ct = canonical::base64url_decode(bad["ciphertext"].as_str().unwrap()).unwrap();
        ct[0] ^= 1;
        bad["ciphertext"] = Value::String(canonical::base64url(&ct));
        assert!(matches!(
            Carrier::read(&serde_json::to_vec(&bad).unwrap(), 4096, false, &sender),
            Err(Error::Signature)
        ));
        let duplicate = v["carrier"]
            .as_str()
            .unwrap()
            .replacen("{", "{\"version\":1,", 1);
        assert!(Carrier::read(duplicate.as_bytes(), 4096, false, &sender).is_err());
        assert!(wire_cap(usize::MAX).is_err());
    }
    #[test]
    fn inner_signature_exact_payload_and_response_request_binding_are_required() {
        let v = fixture();
        let sender: [u8; 32] = raw(&v, "senderPublic").try_into().unwrap();
        let owner: [u8; 32] = raw(&v, "ownerPublic").try_into().unwrap();
        let request = v["request"].as_str().unwrap().as_bytes();
        let mut bad: Value = serde_json::from_slice(request).unwrap();
        bad["payload"] = Value::String(canonical::base64url(
            b"{ \"message\": \"exact whitespace\" }",
        ));
        assert!(matches!(
            message(&serde_json::to_vec(&bad).unwrap(), 4096, &sender),
            Err(Error::Signature)
        ));
        let mut bad: Value = serde_json::from_slice(request).unwrap();
        bad["unexpected"] = Value::Null;
        assert!(message(&serde_json::to_vec(&bad).unwrap(), 4096, &sender).is_err());
        let duplicate = std::str::from_utf8(request)
            .unwrap()
            .replacen("{", "{\"version\":1,", 1);
        assert!(message(duplicate.as_bytes(), 4096, &sender).is_err());
        let request = message(request, 4096, &sender).unwrap();
        let response = message(v["response"].as_str().unwrap().as_bytes(), 4096, &owner).unwrap();
        response.response_to(&request).unwrap();
        for field in [
            "correlationId",
            "machineId",
            "windowId",
            "clientId",
            "sessionId",
            "operation",
        ] {
            let mut bad = serde_json::to_value(&response).unwrap();
            bad[field] = Value::String("different".into());
            let bad: Header = serde_json::from_value(bad).unwrap();
            assert!(bad.response_to(&request).is_err(), "{field}");
        }
    }
    #[test]
    fn only_an_actual_session_open_control_may_assign_a_response_session() {
        let v = fixture();
        let sender: [u8; 32] = raw(&v, "senderPublic").try_into().unwrap();
        let owner: [u8; 32] = raw(&v, "ownerPublic").try_into().unwrap();
        let mut request =
            message(v["request"].as_str().unwrap().as_bytes(), 4096, &sender).unwrap();
        let mut response =
            message(v["response"].as_str().unwrap().as_bytes(), 4096, &owner).unwrap();
        request.operation = "session.open".into();
        response.operation = "session.open".into();
        response.session_id = "11111111-1111-4111-8111-111111111111".into();
        assert!(response.response_to(&request).is_err());
        request.kind = "control".into();
        request.session_id = "new".into();
        response.response_to(&request).unwrap();
    }
}
