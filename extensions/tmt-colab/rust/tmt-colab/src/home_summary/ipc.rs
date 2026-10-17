//! One bounded request to the owned serve socket: write or clear the home summary. The serve
//! alone holds the writer while it serves, so the CLI asks and never opens storage. The write is
//! a whole replacement, so repeating it is safe; a doubtful answer is `Unavailable` and the caller
//! asks again.
use super::{HomeSummary, author_valid, lines_valid};
use crate::{Result, keyring::Layout, limits, page::Fault};
use serde::{Deserialize, Serialize};

pub const PATH: &str = "/.tmt/colab/local/home-summary";
const VERSION: u8 = 1;

/// The request body. The authority is the owner-only socket, never a caller-named principal;
/// `author` is the caller's display name and the serve stamps the time itself. No lines clears.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u8,
    #[serde(default)]
    lines: Option<Vec<String>>,
    #[serde(default)]
    author: Option<String>,
}
/// What a request asks for: the two lines and the writer's name, or a clear.
pub struct Write {
    pub lines: Option<[String; 2]>,
    pub author: Option<String>,
}
/// The reply: the summary now stored (none after a clear) and whether the write changed anything.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub summary: Option<HomeSummary>,
    pub changed: bool,
}

/// Parse a request body into the write it names.
pub(crate) fn parse(body: &[u8]) -> Result<Write> {
    let request: Request = serde_json::from_slice(body).map_err(|_| Fault::Invalid)?;
    if request.version != VERSION {
        return Err(Fault::ServerMismatch.into());
    }
    if request
        .author
        .as_deref()
        .is_some_and(|author| !author_valid(author))
    {
        return Err(Fault::Invalid.into());
    }
    let lines = match request.lines {
        None => None,
        Some(lines) if lines_valid(&lines) => {
            Some(<[String; 2]>::try_from(lines).map_err(|_| Fault::Invalid)?)
        }
        Some(_) => return Err(Fault::Invalid.into()),
    };
    Ok(Write {
        lines,
        author: request.author,
    })
}
/// Ask the serve to store `lines` (or clear the summary when none). Every refusal carries its
/// code; an answer this build cannot read is `Unavailable`.
pub fn request(
    layout: &Layout,
    lines: Option<&[String; 2]>,
    author: Option<&str>,
) -> Result<Outcome> {
    let body = serde_json::to_vec(&Request {
        version: VERSION,
        lines: lines.map(|lines| lines.to_vec()),
        author: author.map(str::to_owned),
    })?;
    let socket = crate::ipc::send(layout, PATH, &body).map_err(|error| match error
        .downcast_ref::<crate::ipc::EarlyReply>(
    ) {
        Some(early) => crate::attachments::ipc::refusal(early.code, &early.body),
        None => Fault::Unavailable.into(),
    })?;
    let (code, response) =
        crate::ipc::receive(socket, limits::HOME_SUMMARY_REPLY).map_err(|_| Fault::Unavailable)?;
    if code == 200 {
        Ok(serde_json::from_slice(&response).map_err(|_| Fault::Unavailable)?)
    } else {
        Err(crate::attachments::ipc::refusal(code, &response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(body: &str) -> Option<&'static str> {
        parse(body.as_bytes())
            .err()
            .and_then(|error| error.downcast_ref::<Fault>().map(Fault::code))
    }

    #[test]
    fn a_request_sets_two_valid_lines_or_clears_and_names_this_version_only() {
        let set = parse(br#"{"version":1,"lines":["a","b"],"author":"Ada"}"#).unwrap();
        assert_eq!(set.lines, Some(["a".to_owned(), "b".to_owned()]));
        assert_eq!(set.author.as_deref(), Some("Ada"));
        let clear = parse(br#"{"version":1}"#).unwrap();
        assert!(clear.lines.is_none() && clear.author.is_none());
        assert_eq!(
            code(r#"{"version":2,"lines":["a","b"]}"#),
            Some("COLAB_SERVER_MISMATCH")
        );
    }

    #[test]
    fn a_request_with_the_wrong_lines_or_extra_fields_is_invalid_and_nothing_is_cut() {
        for body in [
            r#"{"version":1,"lines":["a"]}"#,
            r#"{"version":1,"lines":["a","b","c"]}"#,
            r#"{"version":1,"lines":["a",""]}"#,
            r#"{"version":1,"lines":["a","b"],"extra":1}"#,
            r#"{"version":1,"lines":["a","b"],"author":"bad\u0007"}"#,
            "{}",
        ] {
            assert_eq!(code(body), Some("COLAB_INPUT_INVALID"), "{body}");
        }
        let long = format!(r#"{{"version":1,"lines":["{}","b"]}}"#, "x".repeat(25));
        assert_eq!(code(&long), Some("COLAB_INPUT_INVALID"));
    }
}
