//! Offline captured replies; production discovery alone validates and composes them.
use serde_json::Value;
use std::{fs::File, io::Read, path::Path};
use tmt_remote::{
    deploy_discovery::{self, DeclarationSource, DiscoveryRefusal},
    hosting::HostingRefusal,
    limits,
};

#[derive(Debug)]
pub(crate) enum Refusal {
    Usage,
    InputUnavailable,
    OutputUnavailable,
    NoManifest,
    Discovery(DiscoveryRefusal),
}
impl Refusal {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Usage => "usage",
            Self::InputUnavailable => "input-unavailable",
            Self::OutputUnavailable => "output-unavailable",
            Self::NoManifest => "hosting-manifest-absent",
            Self::Discovery(reason) => match reason {
                DiscoveryRefusal::Unavailable => "discovery-unavailable",
                DiscoveryRefusal::Envelope => "discovery-envelope",
                DiscoveryRefusal::Declaration => "discovery-declaration",
                DiscoveryRefusal::Digest => "discovery-digest",
                DiscoveryRefusal::Plan(_) => "discovery-plan",
                DiscoveryRefusal::Rules(_) => "discovery-rules",
                DiscoveryRefusal::Hosting(reason) => match reason {
                    HostingRefusal::Shape => "hosting-shape",
                    HostingRefusal::Bounds => "hosting-bounds",
                    HostingRefusal::Path => "hosting-path",
                    HostingRefusal::Reserved => "hosting-reserved",
                    HostingRefusal::Digest => "hosting-digest",
                    HostingRefusal::ContentType => "hosting-content-type",
                    HostingRefusal::Csp => "hosting-csp",
                    HostingRefusal::FileSet => "hosting-file-set",
                    HostingRefusal::Collision => "hosting-collision",
                    HostingRefusal::Entry => "hosting-entry",
                    HostingRefusal::Inventory => "hosting-inventory",
                    HostingRefusal::WebAppSelection => "hosting-web-app-selection",
                    HostingRefusal::Compression => "hosting-compression",
                },
            },
        }
    }
}

pub(crate) fn read_reply(
    path: &Path,
    cap: usize,
    oversized: DiscoveryRefusal,
) -> Result<Vec<u8>, Refusal> {
    let file = File::open(path).map_err(|_| Refusal::InputUnavailable)?;
    let metadata = file.metadata().map_err(|_| Refusal::InputUnavailable)?;
    if !metadata.is_file() {
        return Err(Refusal::InputUnavailable);
    }
    if metadata.len() > cap as u64 {
        return Err(Refusal::Discovery(oversized));
    }
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Refusal::InputUnavailable)?;
    if bytes.len() > cap {
        return Err(Refusal::Discovery(oversized));
    }
    Ok(bytes)
}

struct CapturedReplies<'a> {
    declaration: &'a [u8],
    bundle: &'a [u8],
}
impl DeclarationSource for CapturedReplies<'_> {
    fn declaration(&mut self, _: &str) -> Result<Option<Vec<u8>>, DiscoveryRefusal> {
        Ok(Some(self.declaration.to_vec()))
    }
    fn hosting_bundle(&mut self, _: &str) -> Result<Vec<u8>, DiscoveryRefusal> {
        Ok(self.bundle.to_vec())
    }
}

pub(crate) fn compose(declaration: &[u8], bundle: &[u8]) -> Result<Value, Refusal> {
    if declaration.len() > limits::DEPLOY_DECLARATION_REPLY_BYTES {
        return Err(Refusal::Discovery(DiscoveryRefusal::Envelope));
    }
    if bundle.len() > limits::HOSTING_REPLY_BYTES {
        return Err(Refusal::Discovery(DiscoveryRefusal::Hosting(
            HostingRefusal::Bounds,
        )));
    }
    // Select only the extension, as deploy_vector does. discover re-parses strictly,
    // verifies exact digests and owns all declaration, Rules and Hosting admission.
    let value: Value = serde_json::from_slice(declaration)
        .map_err(|_| Refusal::Discovery(DiscoveryRefusal::Envelope))?;
    let extension = value["extension"]
        .as_str()
        .ok_or(Refusal::Discovery(DiscoveryRefusal::Envelope))?;
    let discovered = deploy_discovery::discover(
        &mut CapturedReplies {
            declaration,
            bundle,
        },
        &[extension],
    )
    .map_err(Refusal::Discovery)?;
    Ok(discovered.hosting.ok_or(Refusal::NoManifest)?.view())
}
