//! The static read behind `tmt colab deploy-declaration --json`: Colab's Firestore backend
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

/// The declaration text this binary declares: the static text, plus, when the binary embeds a
/// hosted inventory, its canonical manifest as a last top-level `hosting` key. The manifest bytes
/// are inserted verbatim, so the declaration digest covers exactly the section Remote compares
/// with `hosting-bundle`. A binary with no hosted inventory declares no `hosting` key.
fn declaration_of(base: &str, manifest: Option<&[u8]>) -> Result<String, Unavailable> {
    let Some(manifest) = manifest else {
        return Ok(base.to_owned());
    };
    let manifest = std::str::from_utf8(manifest).map_err(|_| Unavailable)?;
    let body = base.strip_suffix("\n}\n").ok_or(Unavailable)?;
    let text = format!("{body},\n  \"hosting\": {manifest}\n}}\n");
    if !text.is_ascii() {
        return Err(Unavailable);
    }
    Ok(text)
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

/// The reply for a declaration with this hosted inventory, or none.
fn reply_line_with(manifest: Option<&[u8]>) -> Result<String, Unavailable> {
    reply_of(&declaration_of(DECLARATION, manifest)?, ARTIFACT)
}
/// The one reply this executable makes. `run` prints it and the sync bind takes its digest, so
/// what a bound client names and what Remote reads cannot differ.
fn reply_line() -> Result<String, Unavailable> {
    reply_line_with(crate::hosting::manifest())
}
fn digest_of(line: &str) -> Result<String, Unavailable> {
    let reply: serde_json::Value = serde_json::from_str(line).map_err(|_| Unavailable)?;
    reply["declarationDigest"]
        .as_str()
        .map(str::to_owned)
        .ok_or(Unavailable)
}

/// The digest `deploy-declaration --json` reports for this executable: the one declaration a
/// bound sync client and Remote's deployment must agree on.
pub fn declaration_digest() -> Result<String, Unavailable> {
    digest_of(&reply_line()?)
}

pub fn run() -> crate::Result<()> {
    let line = reply_line()?;
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

    // Independent of the compiled-in inventory: the section is the manifest bytes, verbatim, as the
    // last key, and a binary without one declares exactly the static text.
    const MANIFEST: &str =
        include_str!("../../../contracts/vectors/deploy-declaration-v1.hosting.manifest.json");
    const HOSTING_VECTOR: &str =
        include_str!("../../../contracts/vectors/deploy-declaration-v1.hosting.json");

    #[test]
    fn a_binary_without_a_hosted_inventory_declares_no_hosting_key() {
        assert_eq!(declaration_of(DECLARATION, None).unwrap(), DECLARATION);
        assert!(
            serde_json::from_str::<serde_json::Value>(DECLARATION).unwrap()["hosting"].is_null()
        );
    }

    #[test]
    fn the_hosting_key_is_the_manifest_verbatim_and_the_digest_covers_it() {
        let declaration =
            declaration_of(DECLARATION, Some(MANIFEST.trim_end().as_bytes())).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&declaration).unwrap();
        let base: serde_json::Value = serde_json::from_str(DECLARATION).unwrap();
        let mut without = parsed.clone();
        without.as_object_mut().unwrap().remove("hosting");
        assert_eq!(without, base, "every other key is untouched");
        assert_eq!(
            parsed["hosting"],
            serde_json::from_str::<serde_json::Value>(MANIFEST).unwrap()
        );
        assert!(declaration.starts_with(DECLARATION.strip_suffix("\n}\n").unwrap()));
        // The golden line is byte for byte what the command would print for this manifest.
        let line = reply_of(&declaration, ARTIFACT).unwrap();
        assert_eq!(line, HOSTING_VECTOR);
        let reply: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_ne!(reply["declarationDigest"], digest(DECLARATION));
        assert_eq!(reply["declarationDigest"], digest(&declaration));
    }

    #[test]
    fn a_manifest_that_cannot_be_declared_is_unavailable() {
        assert!(
            declaration_of(DECLARATION, Some(&[0xff, 0xfe])).is_err(),
            "not UTF-8"
        );
        assert!(
            declaration_of(DECLARATION, Some("{\"path\":\"é\"}".as_bytes())).is_err(),
            "not ASCII"
        );
        assert!(
            declaration_of("{}", Some(b"{}")).is_err(),
            "a base that is not the static shape"
        );
        let oversized = format!("{{\"pad\":\"{}\"}}", "x".repeat(DECLARATION_TEXT_BYTES));
        let declaration = declaration_of(DECLARATION, Some(oversized.as_bytes())).unwrap();
        assert!(reply_of(&declaration, ARTIFACT).is_err(), "declaration cap");
    }

    #[test]
    fn the_digest_a_sync_client_binds_is_the_one_the_printed_reply_reports() {
        let reply: serde_json::Value = serde_json::from_str(&reply_line().unwrap()).unwrap();
        let bound = declaration_digest().unwrap();
        assert_eq!(reply["declarationDigest"], bound);
        // And it names the declaration text that reply carries, whatever that text holds.
        assert_eq!(digest(reply["declaration"].as_str().unwrap()), bound);
    }

    #[test]
    fn the_digest_a_sync_client_binds_covers_the_hosted_section_the_reply_carries() {
        let plain = digest_of(&reply_line_with(None).unwrap()).unwrap();
        let hosted =
            digest_of(&reply_line_with(Some(MANIFEST.trim_end().as_bytes())).unwrap()).unwrap();
        assert_ne!(plain, hosted, "the hosted section is part of what is bound");
        let declaration =
            declaration_of(DECLARATION, Some(MANIFEST.trim_end().as_bytes())).unwrap();
        assert_eq!(hosted, digest(&declaration));
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
