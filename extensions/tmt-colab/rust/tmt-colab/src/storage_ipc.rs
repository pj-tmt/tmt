//! Strict owner-only storage requests; mounted routes apply owner-device admission.
use crate::{Result, keyring::Layout, limits, page::Fault, storage};
use serde::{Deserialize, Serialize};
use tmt_colab_model::values;

pub const LOCAL_READ: &str = "/.tmt/colab/local/storage";
pub const LOCAL_RECLAIM: &str = "/.tmt/colab/local/reclaim";
pub const BROWSER_READ: &str = "/api/storage";
pub const BROWSER_RECLAIM: &str = "/api/storage/reclaim";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    pub version: u8,
    pub page: String,
    #[serde(default)]
    pub dry_run: bool,
}
pub(crate) fn parse(body: &[u8]) -> Result<Request> {
    let request: Request = serde_json::from_slice(body).map_err(|_| Fault::Invalid)?;
    if request.version != 1 {
        return Err(Fault::ServerMismatch.into());
    }
    values::generated_id(&request.page).map_err(|_| Fault::Invalid)?;
    Ok(request)
}
/// Reclaim through the active owner service; an uncertain reply remains unavailable.
pub fn reclaim(layout: &Layout, page: &str, dry_run: bool) -> Result<storage::Reclaim> {
    let body = serde_json::to_vec(&Request {
        version: 1,
        page: page.into(),
        dry_run,
    })?;
    let socket =
        crate::ipc::send(layout, LOCAL_RECLAIM, &body).map_err(|error| match error
            .downcast_ref::<crate::ipc::EarlyReply>(
        ) {
            Some(early) => crate::attachments::ipc::refusal(early.code, &early.body),
            None => Fault::Unavailable.into(),
        })?;
    let (code, body) =
        crate::ipc::receive(socket, limits::COMPACT_REPLY).map_err(|_| Fault::Unavailable)?;
    if (code == 200 || code == 503)
        && let Ok(report) = serde_json::from_slice::<storage::Reclaim>(&body)
        && report.version == 1
        && report.page == page
        && report.dry_run == dry_run
        && ((code == 200 && report.error.is_none()) || (code == 503 && report.error.is_some()))
    {
        return Ok(report);
    }
    Err(crate::attachments::ipc::refusal(code, &body))
}
