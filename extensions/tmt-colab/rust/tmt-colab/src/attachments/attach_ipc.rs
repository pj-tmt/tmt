//! The two root-local routes of a native attach (#2291) through the owned serve socket. The first
//! opens a staging slot and names the file the caller streams into; the second tells the serve to
//! seal, upload and publish what that file holds. The serve never reads a path the caller names
//! and never answers a browser; the authority is the owner-only socket.
use super::ipc::refusal;
use crate::{Result, keyring::Layout, limits, page::Fault};
use serde::{Deserialize, Serialize};
use tmt_colab_model::{
    attachment::{AttachmentSelector, validate_labels},
    values,
};

pub const STAGE_PATH: &str = "/.tmt/colab/local/attachment-stage";
pub const ATTACH_PATH: &str = "/.tmt/colab/local/attachment-attach";
const VERSION: u8 = 1;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StageRequest {
    version: u8,
    page: String,
    filename: String,
    media_type: String,
}
/// Where to stream the file: a slot ID the caller keeps for a resume, and the file the serve will
/// open itself. The directory is owner-only, like everything under the serve's root.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Staged {
    pub slot: String,
    pub path: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AttachRequest {
    version: u8,
    page: String,
    slot: String,
    /// SHA-256 of the staged copy; absent on a resume of a slot that already holds a sealed file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sha256: Option<String>,
}
/// The attachment that is now on the page, with the exact reference `attachment read` takes.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Attached {
    pub attachment_id: String,
    pub descriptor_hash: String,
    pub filename: String,
    pub media_type: String,
    pub plaintext_bytes: u64,
    pub reference: AttachmentSelector,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttachReply {
    attachment: Attached,
}

/// A stage request: the page and the two labels, validated like any descriptor's.
pub(crate) fn parse_stage(body: &[u8]) -> Result<(String, String, String)> {
    let request: StageRequest = serde_json::from_slice(body).map_err(|_| Fault::Invalid)?;
    if request.version != VERSION {
        return Err(Fault::ServerMismatch.into());
    }
    values::generated_id(&request.page).map_err(|_| Fault::Invalid)?;
    validate_labels(&request.filename, &request.media_type).map_err(|_| Fault::Invalid)?;
    Ok((request.page, request.filename, request.media_type))
}
/// An attach request: the page, the slot ID and, for a first attempt, the staged copy's hash.
pub(crate) fn parse_attach(body: &[u8]) -> Result<(String, String, Option<[u8; 32]>)> {
    let request: AttachRequest = serde_json::from_slice(body).map_err(|_| Fault::Invalid)?;
    if request.version != VERSION {
        return Err(Fault::ServerMismatch.into());
    }
    values::generated_id(&request.page).map_err(|_| Fault::Invalid)?;
    let digest = request
        .sha256
        .map(|hex| {
            values::object_id(&hex).map_err(|_| Fault::Invalid)?;
            let mut bytes = [0u8; 32];
            for (index, byte) in bytes.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
                    .map_err(|_| Fault::Invalid)?;
            }
            Ok::<_, Fault>(bytes)
        })
        .transpose()?;
    Ok((request.page, request.slot, digest))
}
pub(crate) fn stage_reply(staged: &Staged) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(staged)?)
}
pub(crate) fn attach_reply(attached: Attached) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&AttachReply {
        attachment: attached,
    })?)
}

/// Ask the serve to open a slot. Nothing is staged by a refusal.
pub fn stage(layout: &Layout, page: &str, filename: &str, media_type: &str) -> Result<Staged> {
    let body = serde_json::to_vec(&StageRequest {
        version: VERSION,
        page: page.to_owned(),
        filename: filename.to_owned(),
        media_type: media_type.to_owned(),
    })?;
    let (code, response) = crate::ipc::exchange(layout, STAGE_PATH, &body).map_err(early)?;
    if code == 200 {
        serde_json::from_slice(&response).map_err(|_| Fault::Unavailable.into())
    } else {
        Err(refusal(code, &response))
    }
}
/// Tell the serve to attach the staged file. Any doubt about the exchange is `Unavailable` and
/// the slot, named by the caller, is what a later explicit resume uses: the caller never opens a
/// second slot for the same file.
pub fn attach(
    layout: &Layout,
    page: &str,
    slot: &str,
    sha256: Option<[u8; 32]>,
) -> Result<Attached> {
    let body = serde_json::to_vec(&AttachRequest {
        version: VERSION,
        page: page.to_owned(),
        slot: slot.to_owned(),
        sha256: sha256.map(|digest| digest.iter().map(|b| format!("{b:02x}")).collect()),
    })?;
    let socket = crate::ipc::send(layout, ATTACH_PATH, &body).map_err(early)?;
    let (code, response) = crate::ipc::receive(socket, limits::ATTACHMENT_ATTACH_REPLY)
        .map_err(|_| Fault::Unavailable)?;
    if code == 200 {
        serde_json::from_slice::<AttachReply>(&response)
            .map(|reply| reply.attachment)
            .map_err(|_| Fault::Unavailable.into())
    } else {
        Err(refusal(code, &response))
    }
}
fn early(
    error: Box<dyn std::error::Error + Send + Sync>,
) -> Box<dyn std::error::Error + Send + Sync> {
    match error.downcast_ref::<crate::ipc::EarlyReply>() {
        Some(reply) => refusal(reply.code, &reply.body),
        None => Fault::Unavailable.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "20000000-0000-4000-8000-000000000091";
    fn code(error: Box<dyn std::error::Error + Send + Sync>) -> Option<&'static str> {
        error.downcast_ref::<Fault>().map(Fault::code)
    }

    #[test]
    fn a_stage_request_validates_the_page_and_both_labels() {
        let body = |version: u8, page: &str, name: &str, kind: &str| {
            format!(
                r#"{{"version":{version},"page":"{page}","filename":"{name}","mediaType":"{kind}"}}"#
            )
            .into_bytes()
        };
        let (page, name, kind) = parse_stage(&body(1, PAGE, "notes.txt", "text/plain")).unwrap();
        assert_eq!(
            (page.as_str(), name.as_str(), kind.as_str()),
            (PAGE, "notes.txt", "text/plain")
        );
        assert_eq!(
            code(parse_stage(&body(2, PAGE, "a", "text/plain")).unwrap_err()),
            Some("COLAB_SERVER_MISMATCH")
        );
        for bad in [
            body(1, "page", "a", "text/plain"),
            body(1, PAGE, "", "text/plain"),
            body(1, PAGE, "a", "TEXT/plain"),
            body(1, PAGE, "a", "text"),
            br#"{"version":1,"page":"20000000-0000-4000-8000-000000000091","filename":"a","mediaType":"text/plain","path":"/etc"}"#.to_vec(),
        ] {
            assert_eq!(code(parse_stage(&bad).unwrap_err()), Some("COLAB_INPUT_INVALID"));
        }
    }
    #[test]
    fn an_attach_request_names_a_slot_and_an_optional_exact_hash() {
        let hash = "ab".repeat(32);
        let first = format!(r#"{{"version":1,"page":"{PAGE}","slot":"s","sha256":"{hash}"}}"#);
        let (_, slot, digest) = parse_attach(first.as_bytes()).unwrap();
        assert_eq!((slot.as_str(), digest), ("s", Some([0xab; 32])));
        let resume = format!(r#"{{"version":1,"page":"{PAGE}","slot":"s"}}"#);
        assert_eq!(parse_attach(resume.as_bytes()).unwrap().2, None);
        for bad in [
            format!(r#"{{"version":1,"page":"{PAGE}","slot":"s","sha256":"short"}}"#),
            format!(
                r#"{{"version":1,"page":"{PAGE}","slot":"s","sha256":"{}"}}"#,
                "AB".repeat(32)
            ),
            format!(r#"{{"version":1,"page":"{PAGE}","slot":"s","path":"/x"}}"#),
        ] {
            assert_eq!(
                code(parse_attach(bad.as_bytes()).unwrap_err()),
                Some("COLAB_INPUT_INVALID")
            );
        }
    }
}
