//! Ask data codecs for the isolated own projection; no dispatch or persistence.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tmt_colab_model::{Invalid, Result, crypto, framing, values};

pub const INPUT_BYTES: usize = 16 * 1024;
pub const MESSAGE_BYTES: usize = 64 * 1024;
pub const REPLY_BYTES: usize = 16 * 1024;
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedAsk {
    pub operation_id: String,
    pub sender_device: String,
    pub input: String,
    pub signature: String,
    pub final_bytes: String,
}
#[derive(Debug)]
pub struct Intent {
    pub space: String,
    pub page: String,
    pub thread: String,
    pub machine: String,
    pub agent: String,
    pub operation_id: String,
    pub sender_device: String,
    pub grant_expires_at: Option<u64>,
    pub grant_revision: u64,
    pub issued_at: u64,
    pub expires_at: u64,
    pub message: String,
}
fn require(value: bool) -> Result<()> {
    if value { Ok(()) } else { Err(Invalid) }
}
fn utf8(value: &[u8]) -> Result<&str> {
    std::str::from_utf8(value).map_err(|_| Invalid)
}
impl SignedAsk {
    /// Historical expiry is not a read denial. Callers of effectful browser
    /// policy separately enforce the current signed window and live authority.
    pub fn decode(&self) -> Result<Intent> {
        values::generated_id(&self.operation_id)?;
        values::generated_id(&self.sender_device)?;
        require(values::binary(&self.signature, 64)?.len() == 64)?;
        let raw = values::binary(&self.input, INPUT_BYTES)?;
        let f = framing::fields(&raw, 15, INPUT_BYTES)?;
        require(f[0] == b"tmt-colab-send-v1" && f[1] == b"1")?;
        let at = |i: usize| utf8(f[i]);
        values::space_id(at(2)?)?;
        for i in [3, 4, 6, 8, 10] {
            values::generated_id(at(i)?)?;
        }
        values::core_id(at(7)?)?;
        let grant_revision = values::decimal(at(11)?, false)?;
        let grant_expires_at = if at(12)? == "none" {
            None
        } else {
            let expiry = values::decimal(at(12)?, true)?;
            values::time(expiry)?;
            Some(expiry)
        };
        let issued_at = values::decimal(at(13)?, true)?;
        let expires_at = values::decimal(at(14)?, true)?;
        values::time(issued_at)?;
        values::time(expires_at)?;
        require(expires_at > issued_at && expires_at - issued_at <= 86_400_000)?;
        require(at(8)? == self.operation_id && at(10)? == self.sender_device)?;
        let count = u32::from_be_bytes(
            f[5].get(..4)
                .ok_or(Invalid)?
                .try_into()
                .map_err(|_| Invalid)?,
        ) as usize;
        let ids = framing::fields(&f[5][4..], count, 10_240)?;
        let ids: Vec<_> = ids.iter().map(|v| utf8(v)).collect::<Result<_>>()?;
        require(framing::id_list(&ids, false)? == f[5])?;
        let message = values::binary(&self.final_bytes, MESSAGE_BYTES)?;
        require(f[9] == Sha256::digest(&message).as_slice())?;
        Ok(Intent {
            space: at(2)?.into(),
            page: at(3)?.into(),
            thread: at(4)?.into(),
            machine: at(6)?.into(),
            agent: at(7)?.into(),
            operation_id: at(8)?.into(),
            sender_device: at(10)?.into(),
            grant_expires_at,
            grant_revision,
            issued_at,
            expires_at,
            message: utf8(&message)?.into(),
        })
    }
    pub fn verify(&self, public_key: &[u8; 32]) -> Result<Intent> {
        let intent = self.decode()?;
        let signature: [u8; 64] = values::binary(&self.signature, 64)?
            .try_into()
            .map_err(|_| Invalid)?;
        crypto::verify_signature(
            public_key,
            &values::binary(&self.input, INPUT_BYTES)?,
            &signature,
        )?;
        Ok(intent)
    }
}
fn request_id(id: &str) -> Result<()> {
    values::core_id(id.strip_prefix("req_").ok_or(Invalid)?)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AskRecord {
    version: u8,
    kind: String,
    signed: SignedAsk,
    #[serde(rename = "agentName")]
    agent_name: String,
    #[serde(rename = "deviceName")]
    device_name: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateRecord {
    version: u8,
    kind: String,
    operation_id: String,
    revision: String,
    state: String,
    request_id: Option<String>,
    reason: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplyRecord {
    version: u8,
    kind: String,
    operation_id: String,
    request_id: String,
    agent_id: String,
    body: String,
}
/// Typed Ask values share the existing raw own roots. The decoder validates
/// syntax and digest, not keys or live authority; the admitted parent joins
/// records by authenticated writer and checks the intent possession signature.
pub(crate) fn validate_record(root: &str, key: &str, value: &Value) -> Result<()> {
    let kind = value.get("kind").and_then(Value::as_str);
    match kind {
        Some("ask") => {
            let v: AskRecord = serde_json::from_value(value.clone()).map_err(|_| Invalid)?;
            require(
                root == "intents"
                    && v.version == 1
                    && v.kind == "ask"
                    && v.signed.operation_id == key,
            )?;
            require(v.agent_name.len() <= 128 && v.device_name.len() <= 128)?;
            v.signed.decode()?;
        }
        Some("ask-state") => {
            require(value.as_object().is_some_and(|v| {
                v.len() == 7 && v.contains_key("requestId") && v.contains_key("reason")
            }))?;
            let v: StateRecord = serde_json::from_value(value.clone()).map_err(|_| Invalid)?;
            require(
                root == "messages"
                    && v.version == 1
                    && v.kind == "ask-state"
                    && key == format!("{}:{}", v.operation_id, v.revision),
            )?;
            values::generated_id(&v.operation_id)?;
            values::decimal(&v.revision, false)?;
            require(matches!(
                v.state.as_str(),
                "dispatching"
                    | "held"
                    | "accepted"
                    | "uncertain"
                    | "failed"
                    | "refused"
                    | "cancelled"
                    | "expired"
                    | "abandoned"
            ))?;
            require(v.state != "accepted" || v.request_id.is_some())?;
            if let Some(id) = v.request_id {
                request_id(&id)?;
            }
            if let Some(reason) = v.reason {
                require(
                    !reason.is_empty()
                        && reason.len() <= 64
                        && reason.as_bytes()[0].is_ascii_uppercase()
                        && reason
                            .bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'),
                )?;
            }
        }
        Some("ask-reply") => {
            let v: ReplyRecord = serde_json::from_value(value.clone()).map_err(|_| Invalid)?;
            require(
                root == "replies"
                    && v.version == 1
                    && v.kind == "ask-reply"
                    && key == v.operation_id
                    && v.body.len() <= REPLY_BYTES,
            )?;
            values::generated_id(&v.operation_id)?;
            values::core_id(&v.agent_id)?;
            request_id(&v.request_id)?;
        }
        _ => {}
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (SignedAsk, [u8; 32]) {
        let v: Value = serde_json::from_str(include_str!(
            "../../../contracts/vectors/send-preview-v1.json"
        ))
        .unwrap();
        let mut key = [0; 32];
        let hex = v["publicKey"].as_str().unwrap();
        for (i, byte) in key.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap();
        }
        (
            SignedAsk {
                operation_id: "00000000-0000-4000-8000-000000000009".into(),
                sender_device: "00000000-0000-4000-8000-000000000004".into(),
                input: v["input"].as_str().unwrap().into(),
                signature: v["signature"].as_str().unwrap().into(),
                final_bytes: v["finalBytes"].as_str().unwrap().into(),
            },
            key,
        )
    }
    #[test]
    fn independent_send_vector_and_every_field_substitution() {
        let (signed, key) = fixture();
        let intent = signed.verify(&key).unwrap();
        assert!(intent.message.contains("\r\n😀\0"));
        assert_eq!(intent.grant_revision, 1);
        assert_eq!(intent.grant_expires_at, None);
        let input = values::binary(&signed.input, INPUT_BYTES).unwrap();
        let fields = framing::fields(&input, 15, INPUT_BYTES).unwrap();
        let mut offset = 4;
        for f in fields {
            let mut changed = signed.clone();
            let mut bytes = input.clone();
            bytes[offset] ^= 1;
            changed.input = values::encode_binary(&bytes);
            assert!(changed.verify(&key).is_err());
            offset += f.len() + 4;
        }
        let mut changed = signed.clone();
        changed.final_bytes = values::encode_binary(b"other");
        assert!(changed.verify(&key).is_err());
        let mut changed = signed;
        changed.sender_device = intent.agent;
        assert!(changed.verify(&key).is_err());
    }
    #[test]
    fn own_records_are_scoped_bounded_and_strict() {
        let (signed, _) = fixture();
        let ask = json!({"version":1,"kind":"ask","signed":signed,"agentName":"Fixture agent","deviceName":"Fixture browser"});
        let id = signed.operation_id;
        assert!(validate_record("intents", &id, &ask).is_ok());
        for field in ["agentName", "deviceName"] {
            let mut boundary = ask.clone();
            boundary[field] = json!("😀".repeat(32));
            assert!(validate_record("intents", &id, &boundary).is_ok());
            boundary[field] = json!("😀".repeat(33));
            assert!(validate_record("intents", &id, &boundary).is_err());
        }
        assert!(validate_record("replies", &id, &ask).is_err());
        let request = format!("req_{id}");
        let reply = json!({"version":1,"kind":"ask-reply","operationId":id,"requestId":request,"agentId":id,"body":""});
        assert!(validate_record("replies", &id, &reply).is_ok());
        let mut large = reply;
        large["body"] = json!("x".repeat(REPLY_BYTES + 1));
        assert!(validate_record("replies", &id, &large).is_err());
        let state = json!({"version":1,"kind":"ask-state","operationId":id,"revision":"1","state":"accepted","requestId":request,"reason":null});
        assert!(validate_record("messages", &format!("{id}:1"), &state).is_ok());
        assert!(validate_record("messages", &format!("{id}:2"), &state).is_err());
    }
}
