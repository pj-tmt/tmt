//! One bounded request to the existing owned serve socket. Never retries or falls back.
use super::{Fault, Prepared, Receipt};
use crate::{Result, keyring::Layout, limits};
use serde::{Deserialize, Serialize};
pub const PATH: &str = "/.tmt/colab/local/page-write";
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
pub fn write(layout: &Layout, prepared: &Prepared) -> Result<Receipt> {
    let body = serde_json::to_vec(prepared)?;
    if body.len() > limits::http_body_bytes(PATH) {
        return Err(Fault::Capacity.into());
    }
    let (code, response) = crate::ipc::exchange(layout, PATH, &body)?;
    if code != 200 {
        let failure: WriteError =
            serde_json::from_slice(&response).map_err(|_| Fault::Unavailable)?;
        failure.known_code().ok_or(Fault::Unavailable)?;
        return Err(failure.into());
    }
    let receipt: Receipt = serde_json::from_slice(&response).map_err(|_| Fault::Unavailable)?;
    if receipt.space_id != prepared.space_id
        || receipt.page_id != prepared.page_id
        || receipt.epoch != prepared.epoch
    {
        return Err(Fault::Unavailable.into());
    }
    Ok(receipt)
}
