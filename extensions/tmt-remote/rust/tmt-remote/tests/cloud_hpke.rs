//! Native cloud crypto: verification establishes bytes, never operation authority.
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};
use tmt_remote::{
    canonical,
    cloud_hpke::{self, Descriptor, Pin, RecipientKey, Sender},
};

const MACHINE: &str = "11111111-1111-4111-8111-111111111111";
const KEY: &str = "22222222-2222-4222-8222-222222222222";
const CLIENT: &str = "33333333-3333-4333-8333-333333333333";
const ID: &str = "44444444-4444-4444-8444-444444444444";
const SESSION: &str = "55555555-5555-4555-8555-555555555555";
const WINDOW: &str = "66666666-6666-4666-8666-666666666666";
const NOW: u64 = 1_800_000_000_000;
fn auth() -> SigningKey {
    SigningKey::from_bytes(&[7; 32])
}
fn sender() -> SigningKey {
    SigningKey::from_bytes(&[8; 32])
}
fn descriptor(key: &RecipientKey, revision: u64) -> Vec<u8> {
    Descriptor::sign(
        MACHINE,
        KEY,
        revision,
        &key.public(),
        NOW - 1,
        NOW + 1000,
        |bytes| auth().sign(bytes).to_bytes(),
    )
    .unwrap()
}
fn pin<'a>(key: &'a [u8; 32]) -> Pin<'a> {
    Pin {
        machine_id: MACHINE,
        authentication_key: key,
    }
}
fn envelope(response: bool) -> Vec<u8> {
    let payload = b"{\"message\": \"exact whitespace\"}";
    let env = canonical::Envelope {
        kind: if response { "response" } else { "request" },
        id: if response { KEY } else { ID },
        correlation_id: response.then_some(ID),
        machine_id: MACHINE,
        window_id: WINDOW,
        client_id: CLIENT,
        session_id: SESSION,
        sequence: "1",
        timestamp_ms: NOW,
        origin: "cli",
        operation: "agents.list",
        payload,
    };
    let sig = if response { auth() } else { sender() }.sign(&canonical::envelope(&env).unwrap());
    serde_json::to_vec_pretty(&json!({"version":1,"profile":"local-v1","kind":env.kind,
        "id":env.id,"correlationId":env.correlation_id,"machineId":MACHINE,"windowId":WINDOW,
        "clientId":CLIENT,"sessionId":SESSION,"sequence":"1","timestampMs":NOW,"origin":"cli",
        "operation":"agents.list","payload":canonical::base64url(payload),
        "signature":canonical::base64url(&sig.to_bytes())}))
    .unwrap()
}
struct TestEntropy([u8; 32]);
impl hpke::rand_core::TryRng for TestEntropy {
    type Error = std::convert::Infallible;
    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(u32::from_le_bytes(self.0[..4].try_into().unwrap()))
    }
    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(u64::from_le_bytes(self.0[..8].try_into().unwrap()))
    }
    fn try_fill_bytes(&mut self, out: &mut [u8]) -> Result<(), Self::Error> {
        out.copy_from_slice(&self.0);
        Ok(())
    }
}
impl hpke::rand_core::TryCryptoRng for TestEntropy {}
#[test]
fn descriptors_are_pinned_fresh_and_monotonic_without_repairing_old_bytes() {
    let key = RecipientKey::from_ikm(&[9; 32]);
    let public = auth().verifying_key().to_bytes();
    let bytes = descriptor(&key, 1);
    let old = Descriptor::verify(&bytes, pin(&public), NOW, None).unwrap();
    assert_eq!(old.bytes(), bytes);
    Descriptor::verify(&bytes, pin(&public), NOW, Some(&old)).unwrap();
    let newer = Descriptor::verify(&descriptor(&key, 2), pin(&public), NOW, Some(&old)).unwrap();
    assert_eq!(
        Descriptor::verify(&bytes, pin(&public), NOW, Some(&newer)).err(),
        Some(cloud_hpke::Error::Revision)
    );
    let changed = Descriptor::sign(
        MACHINE,
        KEY,
        1,
        &RecipientKey::from_ikm(&[10; 32]).public(),
        NOW - 1,
        NOW + 1000,
        |b| auth().sign(b).to_bytes(),
    )
    .unwrap();
    assert_eq!(
        Descriptor::verify(&changed, pin(&public), NOW, Some(&old)).err(),
        Some(cloud_hpke::Error::Revision)
    );
    assert_eq!(
        Descriptor::verify(&bytes, pin(&public), NOW + 1000, None).err(),
        Some(cloud_hpke::Error::DescriptorExpired)
    );
    assert!(Descriptor::verify(&bytes, pin(&[0; 32]), NOW, None).is_err());
    assert_eq!(
        Descriptor::verify(&bytes, pin(&public), NOW - 2, None).err(),
        Some(cloud_hpke::Error::NotYetValid)
    );
    assert!(
        Descriptor::verify(
            &bytes,
            Pin {
                machine_id: CLIENT,
                authentication_key: &public
            },
            NOW,
            None
        )
        .is_err()
    );
    let mut unknown: Value = serde_json::from_slice(&bytes).unwrap();
    unknown["extra"] = json!(0);
    assert!(
        Descriptor::verify(
            &serde_json::to_vec(&unknown).unwrap(),
            pin(&public),
            NOW,
            None
        )
        .is_err()
    );
    let mut newer: Value = serde_json::from_slice(&bytes).unwrap();
    newer["version"] = json!(2);
    assert_eq!(
        Descriptor::verify(
            &serde_json::to_vec(&newer).unwrap(),
            pin(&public),
            NOW,
            None
        )
        .err(),
        Some(cloud_hpke::Error::UpgradeRequired)
    );
    assert!(cloud_hpke::Error::DescriptorExpired.retryable());

    for field in [
        "version",
        "profile",
        "machineId",
        "keyId",
        "revision",
        "publicKey",
        "notBeforeMs",
        "expiresAtMs",
        "signature",
    ] {
        let mut bad: Value = serde_json::from_slice(&bytes).unwrap();
        bad[field] = Value::Null;
        assert!(
            Descriptor::verify(&serde_json::to_vec(&bad).unwrap(), pin(&public), NOW, None)
                .is_err(),
            "{field}"
        );
    }
    let duplicate = String::from_utf8(bytes)
        .unwrap()
        .replacen("{", "{\"version\":1,", 1);
    assert!(Descriptor::verify(duplicate.as_bytes(), pin(&public), NOW, None).is_err());
}
#[test]
fn exact_envelopes_roundtrip_with_one_request_bound_response() {
    let key = RecipientKey::from_ikm(&[9; 32]);
    let public = auth().verifying_key().to_bytes();
    let desc = Descriptor::verify(&descriptor(&key, 1), pin(&public), NOW, None).unwrap();
    let sender_key = sender().verifying_key().to_bytes();
    let peer = Sender {
        client_id: CLIENT,
        authentication_key: &sender_key,
    };
    let plain = envelope(false);
    let (carrier, response_reader) = cloud_hpke::seal(
        &plain,
        4096,
        &desc,
        NOW,
        peer,
        |b| sender().sign(b).to_bytes(),
        &mut TestEntropy([42; 32]),
    )
    .unwrap();
    let (opened, response_writer) =
        cloud_hpke::open(&carrier, 4096, &key, &desc, NOW, peer).unwrap();
    assert_eq!(opened, plain);
    let response = envelope(true);
    let sealed_response = response_writer
        .seal(&response, 4096, |b| auth().sign(b).to_bytes())
        .unwrap();
    assert_eq!(
        response_reader.open(&sealed_response, 4096).unwrap(),
        response
    );
    for field in [
        "header",
        "enc",
        "ciphertext",
        "signature",
        "keyId",
        "profile",
    ] {
        let mut bad: Value = serde_json::from_slice(&carrier).unwrap();
        bad[field] = if field == "header" {
            json!({})
        } else {
            json!("tampered")
        };
        assert!(
            cloud_hpke::open(
                &serde_json::to_vec(&bad).unwrap(),
                4096,
                &key,
                &desc,
                NOW,
                peer
            )
            .is_err(),
            "{field}"
        );
    }
    assert!(
        cloud_hpke::open(
            &carrier,
            4096,
            &RecipientKey::from_ikm(&[11; 32]),
            &desc,
            NOW,
            peer
        )
        .is_err()
    );
    assert!(cloud_hpke::open(&carrier, 4096, &key, &desc, NOW + 1000, peer).is_err());
    assert!(cloud_hpke::open(&carrier, 10, &key, &desc, NOW, peer).is_err());
}

#[test]
fn rfc9180_appendix_a_3_1_base_receiver_and_exporter() {
    use hpke::{Deserializable, Kem as _, Serializable};
    type Kem = hpke::kem::DhP256HkdfSha256;
    fn hex(s: &str) -> Vec<u8> {
        s.as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect()
    }
    let ikm = hex("668b37171f1072f3cf12ea8a236a45df23fc13b82af3609ad1e354f6ef817550");
    let (sk, pk) = Kem::derive_keypair(&ikm);
    assert_eq!(
        sk.to_bytes().as_slice(),
        hex("f3ce7fdae57e1a310d87f1ebbde6f328be0a99cdbcadf4d6589cf29de4b8ffd2")
    );
    assert_eq!(
        pk.to_bytes().as_slice(),
        hex(
            "04fe8c19ce0905191ebc298a9245792531f26f0cece2460639e8bc39cb7f706a826a779b4cf969b8a0e539c7f62fb3d30ad6aa8f80e30f1d128aafd68a2ce72ea0"
        )
    );
    let enc=<Kem as hpke::Kem>::EncappedKey::from_bytes(&hex("04a92719c6195d5085104f469a8b9814d5838ff72b60501e2c4466e5e67b325ac98536d7b61a1af4b78e5b7f951c0900be863c403ce65c9bfcb9382657222d18c4")).unwrap();
    let info = hex("4f6465206f6e2061204772656369616e2055726e");
    let (sent_enc, mut sent_ctx) =
        hpke::setup_sender_with_rng::<hpke::aead::AesGcm128, hpke::kdf::HkdfSha256, Kem>(
            &hpke::OpModeS::Base,
            &pk,
            &info,
            &mut TestEntropy(
                hex("4270e54ffd08d79d5928020af4686d8f6b7d35dbe470265f1f5aa22816ce860e")
                    .try_into()
                    .unwrap(),
            ),
        )
        .unwrap();
    assert_eq!(sent_enc.to_bytes(), enc.to_bytes());
    assert_eq!(
        sent_ctx
            .seal(
                &hex("4265617574792069732074727574682c20747275746820626561757479"),
                &hex("436f756e742d30")
            )
            .unwrap(),
        hex(
            "5ad590bb8baa577f8619db35a36311226a896e7342a6d836d8b7bcd2f20b6c7f9076ac232e3ab2523f39513434"
        )
    );
    let mut ctx = hpke::setup_receiver::<hpke::aead::AesGcm128, hpke::kdf::HkdfSha256, Kem>(
        &hpke::OpModeR::Base,
        &sk,
        &enc,
        &hex("4f6465206f6e2061204772656369616e2055726e"),
    )
    .unwrap();
    let pt=ctx.open(&hex("5ad590bb8baa577f8619db35a36311226a896e7342a6d836d8b7bcd2f20b6c7f9076ac232e3ab2523f39513434"),&hex("436f756e742d30")).unwrap();
    assert_eq!(
        pt,
        hex("4265617574792069732074727574682c20747275746820626561757479")
    );
    for (info, want) in [
        (
            "",
            "5e9bc3d236e1911d95e65b576a8a86d478fb827e8bdfe77b741b289890490d4d",
        ),
        (
            "00",
            "6cff87658931bda83dc857e6353efe4987a201b849658d9b047aab4cf216e796",
        ),
        (
            "54657374436f6e74657874",
            "d8f1ea7942adbba7412c6d431c62d01371ea476b823eb697e1f6e6cae1dab85a",
        ),
    ] {
        let mut exported = [0; 32];
        ctx.export(&hex(info), &mut exported).unwrap();
        let mut sender_exported = [0; 32];
        sent_ctx.export(&hex(info), &mut sender_exported).unwrap();
        assert_eq!(sender_exported, exported);
        assert_eq!(exported.as_slice(), hex(want));
    }
}
