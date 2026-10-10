//! One bounded root-local attachment read through the owned serve socket. The serve alone holds
//! the established object channel; this client never opens storage or a backend, never retries
//! and treats any doubt about the exchange as `Unavailable`. A read has no effect, so a doubtful
//! answer is simply unanswered.
use crate::{Result, keyring::Layout, limits, page::Fault, page::ipc::WriteError};
use serde::{Deserialize, Serialize};
use tmt_colab_model::{
    attachment::{AttachmentSelector, validate_labels},
    values,
};

pub const PATH: &str = "/.tmt/colab/local/attachment-read";
const VERSION: u8 = 1;

/// The request body. The authority is the owner-only socket, never a caller-named principal.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u8,
    page: String,
    selector: AttachmentSelector,
}
/// Parse a request body into the page and selector it names, validated like any other reference.
pub(crate) fn parse(body: &[u8]) -> Result<(String, AttachmentSelector)> {
    let request: Request = serde_json::from_slice(body).map_err(|_| Fault::Invalid)?;
    if request.version != VERSION {
        return Err(Fault::ServerMismatch.into());
    }
    values::generated_id(&request.page).map_err(|_| Fault::Invalid)?;
    request.selector.validate().map_err(|_| Fault::Invalid)?;
    Ok((request.page, request.selector))
}
/// The verified plaintext of one attachment and the media type its verified descriptor names.
/// A serve that names none (an older build) leaves `application/octet-stream`.
pub struct Verified {
    pub bytes: Vec<u8>,
    pub media_type: String,
}
/// Ask the serve for the verified plaintext of one exact reference. Every refusal carries its
/// code; an answer this build cannot read is `Unavailable`.
pub fn read(layout: &Layout, page: &str, selector: &AttachmentSelector) -> Result<Verified> {
    let body = serde_json::to_vec(&Request {
        version: VERSION,
        page: page.to_owned(),
        selector: selector.clone(),
    })?;
    let socket = crate::ipc::send(layout, PATH, &body).map_err(|error| match error
        .downcast_ref::<crate::ipc::EarlyReply>(
    ) {
        Some(early) => refusal(early.code, &early.body),
        None => Fault::Unavailable.into(),
    })?;
    let reply = crate::ipc::receive_up_to(
        socket,
        limits::ATTACHMENT_READ_REPLY,
        limits::ATTACHMENT_READ_BYTES,
    )
    .map_err(|_| Fault::Unavailable)?;
    if reply.code == 200 {
        // The label is inert and only ever looked up in an allow-list; one that is not a
        // well-formed `type/subtype` reads as no type.
        let media_type = reply
            .content_type
            .filter(|kind| validate_labels("-", kind).is_ok())
            .unwrap_or_else(|| "application/octet-stream".into());
        Ok(Verified {
            bytes: reply.body,
            media_type,
        })
    } else {
        Err(refusal(reply.code, &reply.body))
    }
}
/// The serve's typed refusal, or the plain answer of a server that does not have this route.
pub(super) fn refusal(code: u16, response: &[u8]) -> Box<dyn std::error::Error + Send + Sync> {
    match serde_json::from_slice::<WriteError>(response) {
        Ok(failure) => failure.into(),
        Err(_) if matches!(code, 404 | 413) => Fault::ServerMismatch.into(),
        Err(_) => Fault::Unavailable.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "20000000-0000-4000-8000-000000000091";
    const ID: &str = "20000000-0000-4000-8000-000000000092";
    fn body(version: u8, page: &str, selector: &str) -> Vec<u8> {
        format!(r#"{{"version":{version},"page":"{page}","selector":{selector}}}"#).into_bytes()
    }
    fn document(hash: &str) -> String {
        format!(
            r#"{{"kind":"document-current","attachmentId":"{ID}","descriptorHash":"{hash}","contentRevision":"v1:{}"}}"#,
            "a".repeat(64)
        )
    }

    #[test]
    fn a_valid_request_names_its_page_and_exact_reference() {
        let (page, selector) = parse(&body(1, PAGE, &document(&"b".repeat(64)))).unwrap();
        assert_eq!(page, PAGE);
        assert!(matches!(
            selector,
            AttachmentSelector::DocumentCurrent { .. }
        ));
    }
    #[test]
    fn a_request_is_refused_for_each_malformed_part_in_isolation() {
        let code = |bytes: &[u8]| {
            parse(bytes)
                .unwrap_err()
                .downcast_ref::<Fault>()
                .map(Fault::code)
        };
        let valid = document(&"b".repeat(64));
        assert_eq!(code(&body(2, PAGE, &valid)), Some("COLAB_SERVER_MISMATCH"));
        assert_eq!(
            code(&body(1, "not-a-page", &valid)),
            Some("COLAB_INPUT_INVALID")
        );
        assert_eq!(
            code(&body(1, PAGE, &document("short"))),
            Some("COLAB_INPUT_INVALID")
        );
        let extra = format!(r#"{{"version":1,"page":"{PAGE}","selector":{valid},"agent":"x"}}"#);
        assert_eq!(code(extra.as_bytes()), Some("COLAB_INPUT_INVALID"));
        assert_eq!(code(b"{}"), Some("COLAB_INPUT_INVALID"));
    }
}
