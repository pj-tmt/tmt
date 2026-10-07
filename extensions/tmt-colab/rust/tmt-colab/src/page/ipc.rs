//! One bounded request to the existing owned serve socket. Never retries or falls back.
//! A published batch is one `LocalWrite` and one retained original-operation outcome.
use super::{Fault, FrozenPublication, PublicationRecord, Published};
use crate::{
    Result,
    keyring::{Keyring, Layout},
    limits,
    publication::{LocalWrite, Outcome, WriteAction},
};
use serde::{Deserialize, Serialize};
use tmt_colab_model::values;
pub const PATH: &str = "/.tmt/colab/local/page-publish";
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteError {
    error: Detail,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Detail {
    code: String,
    message: String,
}
impl WriteError {
    pub fn code(&self) -> &'static str {
        self.known_code()
            .unwrap_or_else(|| Fault::Unavailable.code())
    }
    fn known_code(&self) -> Option<&'static str> {
        [
            Fault::StaleBase,
            Fault::Invalid,
            Fault::Capacity,
            Fault::Missing,
            Fault::Inactive,
            Fault::Denied,
            Fault::Unavailable,
        ]
        .into_iter()
        .map(|fault| fault.code())
        .find(|code| *code == self.error.code)
    }
    pub(crate) fn status(&self) -> u16 {
        match self.code() {
            "COLAB_DENIED" => 403,
            "COLAB_STALE_BASE" | "COLAB_PAGE_INACTIVE" | "COLAB_STATE_MISSING" => 409,
            "COLAB_CAPACITY" => 413,
            "COLAB_INPUT_INVALID" => 400,
            _ => 503,
        }
    }
    pub(crate) fn from_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self {
            error: Detail {
                code: error
                    .downcast_ref::<Fault>()
                    .unwrap_or(&Fault::Unavailable)
                    .code()
                    .into(),
                message: error.to_string(),
            },
        }
    }
}
impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.error.message)
    }
}
impl std::error::Error for WriteError {}
/// The 200 body of a publish: the exact retained outcome bytes, plus the page revision read by
/// the serve under the lock that excludes every other writer.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Reply {
    outcome: Box<serde_json::value::RawValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
}
pub(crate) fn reply_json(published: &Published) -> Result<Vec<u8>> {
    let outcome = std::str::from_utf8(&published.record.bytes)?.to_owned();
    Ok(serde_json::to_vec(&Reply {
        outcome: serde_json::value::RawValue::from_string(outcome)?,
        revision: published.revision.clone(),
    })?)
}
/// Posts one frozen publication. `Ok(Some)` is the answered publish (committed or rejected);
/// `Err` is a refusal the server reported before any effect. `Ok(None)` is doubt after the
/// request may have been sent: the caller resolves it by original-operation status, never by a
/// resend or an offline writer.
pub fn publish(
    layout: &Layout,
    key: &Keyring,
    frozen: &FrozenPublication,
) -> Result<Option<Published>> {
    let original = frozen.job().key()?;
    let body = LocalWrite {
        version: 2,
        action: WriteAction::Write,
        signed_job: frozen.job().clone(),
        packet: values::encode_binary(frozen.packet()),
        chain: values::encode_binary(frozen.chain()),
    }
    .to_json(&key.local_writer()?.1)?;
    // Nothing was sent: the server acts only on a complete body, so this is a plain refusal.
    let socket = crate::ipc::send(layout, PATH, &body).map_err(|_| Fault::Unavailable)?;
    let Ok((code, response)) = crate::ipc::receive(socket, limits::PUBLISH_REPLY) else {
        return Ok(None);
    };
    if code != 200 {
        let Ok(failure) = serde_json::from_slice::<WriteError>(&response) else {
            return Ok(None);
        };
        if failure.known_code().is_none() {
            return Ok(None);
        }
        return Err(failure.into());
    }
    let Ok(reply) = serde_json::from_slice::<Reply>(&response) else {
        return Ok(None);
    };
    let bytes = reply.outcome.get().as_bytes().to_vec();
    let valid_revision = reply
        .revision
        .as_deref()
        .is_none_or(crate::publication::valid_revision);
    Ok(Outcome::from_json(&bytes, &original, Some(frozen.job()))
        .ok()
        .filter(|outcome| !matches!(outcome, Outcome::Unknown { .. }) && valid_revision)
        .map(|outcome| Published {
            record: PublicationRecord { outcome, bytes },
            revision: reply.revision,
        }))
}
