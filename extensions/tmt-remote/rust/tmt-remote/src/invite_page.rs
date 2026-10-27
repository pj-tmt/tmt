//! Fixed, instructions-only invitation entry. No provider I/O or enrollment.
use crate::{deploy_plan::sha256_hex, deploy_run::DeployRecord, hosting::HostingFile};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::Digest;

#[cfg(test)]
mod tests;

const HTML: &str = include_str!("../assets/join.html");
const BOOTSTRAP: &str = include_str!("../assets/join.js");
const SCRIPT: &str = include_str!("../assets/invite-v1.js");
const STYLE: &str = concat!(
    include_str!("../../../../../design/browser-ui/generated/static.css"),
    "\n",
    include_str!("../assets/pages.css"),
);
pub(crate) const ROUTE: &str = "^/join$";

pub(crate) fn document() -> String {
    HTML.replace("__TMT_JOIN_BOOTSTRAP__", BOOTSTRAP)
}
pub(crate) fn policy() -> String {
    let document = document();
    let inline = document
        .split_once("<script>")
        .expect("join bootstrap")
        .1
        .split_once("</script>")
        .expect("join bootstrap end")
        .0;
    let hash = STANDARD.encode(sha2::Sha256::digest(inline.as_bytes()));
    format!(
        "default-src 'none'; script-src 'self' 'sha256-{hash}'; style-src 'self'; img-src data:; connect-src 'none'; base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'"
    )
}

/// A separate resource set: no dependency on an extension's shell/SDK version.
pub(crate) fn assets() -> Vec<(HostingFile, String)> {
    [
        ("/join.html", document(), "text/html", Some(policy())),
        ("/sdk/invite-v1.js", SCRIPT.into(), "text/javascript", None),
        ("/sdk/invite.css", STYLE.into(), "text/css", None),
    ]
    .into_iter()
    .map(|(path, raw, content_type, csp)| {
        (
            HostingFile {
                path: path.into(),
                sha256: sha256_hex(raw.as_bytes()),
                length: raw.len() as u64,
                content_type: content_type.into(),
                csp,
            },
            raw,
        )
    })
    .collect()
}
pub(crate) fn resource(path: &str) -> Option<(String, String, Option<String>)> {
    if !reserved(path) {
        return None;
    }
    let path = if path == "/join" { "/join.html" } else { path };
    assets()
        .into_iter()
        .find(|(file, _)| file.path == path)
        .map(|(file, raw)| (file.content_type, raw, file.csp))
}
pub(crate) fn reserved(path: &str) -> bool {
    matches!(
        path,
        "/join" | "/join.html" | "/sdk/invite-v1.js" | "/sdk/invite.css"
    )
}

/// Exact captured resources and route headers must be covered by the publication.
/// A complete pre-I4 deployment is intentionally insufficient. No provider probe.
pub(crate) fn link(record: &DeployRecord, config: &Value, token: &str) -> Option<String> {
    let publication = record.verified_publication()?;
    if publication.public_config() != config {
        return None;
    }
    let checkpoint = record.run.as_ref()?.hosting.as_ref()?;
    let view: crate::deploy_run::DeployView =
        serde_json::from_value(checkpoint.envelope.clone()).ok()?;
    let run = record.run.as_ref()?;
    if !publication.valid_for(&view, checkpoint, &run.plan_digest) {
        return None;
    }
    let content = &view.hosting.as_ref()?.content;
    let files = content["files"].as_array()?;
    let resources = assets();
    for (expected, _) in &resources {
        if !files.iter().any(|file| {
            file["path"] == expected.path
                && file["rawDigest"] == expected.sha256
                && file["rawLength"] == expected.length
                && file["contentType"] == expected.content_type
        }) {
            return None;
        }
    }
    let config = &content["config"];
    let headers = config["headers"].as_array()?;
    for (file, _) in &resources {
        if !headers.iter().any(|rule| {
            rule["glob"] == file.path && rule["headers"] == crate::hosting::response_headers(file)
        }) {
            return None;
        }
    }
    let expected = crate::hosting::response_headers(&resources[0].0);
    if !headers
        .iter()
        .any(|rule| rule["regex"] == ROUTE && rule["headers"] == expected)
        || !config["rewrites"]
            .as_array()?
            .iter()
            .any(|rule| *rule == json!({"regex":ROUTE,"path":"/join.html"}))
    {
        return None;
    }
    Some(format!("{}/join#{token}", publication.entry_url()))
}
