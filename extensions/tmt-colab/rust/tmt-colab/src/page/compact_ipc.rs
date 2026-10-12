//! One bounded request to the owned serve socket: combine this device's own stream. The serve
//! alone holds the writer and the sync mutex while it serves, so the CLI asks and never opens
//! storage. A combine can be repeated safely (a second one finds nothing to do), so a doubtful
//! answer is simply `Unavailable` and the caller asks again.
use super::{Fault, compact::Report};
use crate::{Result, keyring::Layout, limits};
use serde::{Deserialize, Serialize};
use tmt_colab_model::values;

pub const PATH: &str = "/.tmt/colab/local/page-compact";
const VERSION: u8 = 1;

/// The request body. The authority is the owner-only socket, never a caller-named principal.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u8,
    page: String,
}
/// Parse a request body into the page it names.
pub(crate) fn parse(body: &[u8]) -> Result<String> {
    let request: Request = serde_json::from_slice(body).map_err(|_| Fault::Invalid)?;
    if request.version != VERSION {
        return Err(Fault::ServerMismatch.into());
    }
    values::generated_id(&request.page).map_err(|_| Fault::Invalid)?;
    Ok(request.page)
}
/// Ask the serve to combine this device's own stream for `page`. Every refusal carries its code;
/// an answer this build cannot read is `Unavailable`.
pub fn request(layout: &Layout, page: &str) -> Result<Report> {
    let body = serde_json::to_vec(&Request {
        version: VERSION,
        page: page.to_owned(),
    })?;
    let socket = crate::ipc::send(layout, PATH, &body).map_err(|error| match error
        .downcast_ref::<crate::ipc::EarlyReply>(
    ) {
        Some(early) => crate::attachments::ipc::refusal(early.code, &early.body),
        None => Fault::Unavailable.into(),
    })?;
    let (code, response) =
        crate::ipc::receive(socket, limits::COMPACT_REPLY).map_err(|_| Fault::Unavailable)?;
    if code == 200 {
        Ok(serde_json::from_slice(&response).map_err(|_| Fault::Unavailable)?)
    } else {
        Err(crate::attachments::ipc::refusal(code, &response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "20000000-0000-4000-8000-000000000091";

    #[test]
    fn a_request_names_one_generated_page_and_this_version_only() {
        let body = |version: u8, page: &str| {
            format!(r#"{{"version":{version},"page":"{page}"}}"#).into_bytes()
        };
        assert_eq!(parse(&body(1, PAGE)).unwrap(), PAGE);
        let code = |body: &[u8]| {
            parse(body)
                .unwrap_err()
                .downcast_ref::<Fault>()
                .map(Fault::code)
        };
        assert_eq!(code(&body(2, PAGE)), Some("COLAB_SERVER_MISMATCH"));
        assert_eq!(code(&body(1, "page")), Some("COLAB_INPUT_INVALID"));
        assert_eq!(code(b"{}"), Some("COLAB_INPUT_INVALID"));
        assert_eq!(
            code(format!(r#"{{"version":1,"page":"{PAGE}","extra":1}}"#).as_bytes()),
            Some("COLAB_INPUT_INVALID")
        );
    }
}
