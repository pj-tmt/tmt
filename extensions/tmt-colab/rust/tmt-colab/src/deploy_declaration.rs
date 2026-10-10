//! The static read behind `tmt colab deploy-declaration --json` (#2397): Colab's Firestore backend
//! declaration and admission Rules, compiled in so the release checksum and receipt cover them.
//! The reply is a pure function of the binary: it reads no state, door, network, clock, current
//! directory or environment, so it works on a machine that never started Colab.
use crate::limits::{DECLARATION_REPLY_BYTES, DECLARATION_TEXT_BYTES};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fmt, io::Write};

const DECLARATION: &str = include_str!("../../../firestore/declaration.json");
const ARTIFACT: &str = include_str!("../../../firestore/admission.rules");

/// The envelope owned by `remote-channel-v1.md`, in its field order.
#[derive(Serialize)]
struct Reply<'a> {
    version: u8,
    extension: &'static str,
    backend: &'static str,
    declaration: &'a str,
    #[serde(rename = "declarationDigest")]
    declaration_digest: String,
    artifact: &'a str,
}

/// Any reason the reply cannot be produced. Remote reads `COLAB_UNAVAILABLE` as "unavailable",
/// never as "no declaration", so this must not become an input error.
#[derive(Debug)]
pub struct Unavailable;

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("The Colab Firestore declaration is unavailable")
    }
}

impl std::error::Error for Unavailable {}

fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The compact reply line for these two texts. The declaration must name the artifact's own
/// digest: a binary built from edited Rules without a regenerated declaration refuses itself.
fn reply_of(declaration: &str, artifact: &str) -> Result<String, Unavailable> {
    if declaration.len() > DECLARATION_TEXT_BYTES || artifact.len() > DECLARATION_TEXT_BYTES {
        return Err(Unavailable);
    }
    let parsed: serde_json::Value = serde_json::from_str(declaration).map_err(|_| Unavailable)?;
    if parsed["admission"]["digest"].as_str() != Some(digest(artifact).as_str()) {
        return Err(Unavailable);
    }
    let mut line = serde_json::to_string(&Reply {
        version: 1,
        extension: "colab",
        backend: "firestore",
        declaration,
        declaration_digest: digest(declaration),
        artifact,
    })
    .map_err(|_| Unavailable)?;
    line.push('\n');
    if line.len() > DECLARATION_REPLY_BYTES {
        return Err(Unavailable);
    }
    Ok(line)
}

pub fn run() -> crate::Result<()> {
    let line = reply_of(DECLARATION, ARTIFACT)?;
    let mut output = tmt_cli_style::stream::stdout(true);
    output.write_all(line.as_bytes()).map_err(|_| Unavailable)?;
    output.flush().map_err(|_| Unavailable)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_texts_make_a_reply_whose_digests_name_exactly_those_bytes() {
        let line = reply_of(DECLARATION, ARTIFACT).unwrap();
        let reply: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["declaration"], DECLARATION);
        assert_eq!(reply["artifact"], ARTIFACT);
        assert_eq!(reply["declarationDigest"], digest(DECLARATION));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(DECLARATION).unwrap()["admission"]["digest"],
            digest(ARTIFACT)
        );
    }

    #[test]
    fn a_reply_that_cannot_be_trusted_is_unavailable() {
        let oversized = " ".repeat(DECLARATION_TEXT_BYTES + 1);
        assert!(reply_of(&oversized, ARTIFACT).is_err(), "declaration cap");
        assert!(reply_of(DECLARATION, &oversized).is_err(), "artifact cap");
        assert!(
            reply_of("not json", ARTIFACT).is_err(),
            "unparseable declaration"
        );
        // Rules edited without regenerating the declaration: the pinned digest disagrees.
        assert!(reply_of(DECLARATION, &format!("{ARTIFACT}// edit\n")).is_err());
    }
}
