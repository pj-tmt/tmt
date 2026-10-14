//! Build-owned hosted distribution admission, SDK copy, and immutable wire snapshots.
use crate::browser_policy;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

// Strictly below 50% of Remote's v1 caps (limits.rs): 256 files, 4 MiB/file, 16 MiB total.
pub const FILES: usize = 128;
pub const FILE_BYTES: usize = 2 * 1024 * 1024;
pub const TOTAL_BYTES: usize = 8 * 1024 * 1024;
const SDK_ROUTE: &str = "/sdk/remote-v1.js";
/// Remote's lazy Firebase client and its notices: hosted-only build output that Remote pins.
const FIREBASE_ROUTE: &str = "/sdk/remote-firebase-v1.js";
const NOTICES_ROUTE: &str = "/sdk/THIRD-PARTY-NOTICES-remote.txt";
/// The routes Remote's SDK owns; the hosted tree cannot supply them.
const SDK_ROUTES: [&str; 3] = [SDK_ROUTE, FIREBASE_ROUTE, NOTICES_ROUTE];
const PIN: &str = "firebase-build-pin.json";
/// Each pinned Remote output: its hosted route, its path in Remote's pin manifest and the name of
/// the copy the build keeps.
const PINNED: [(&str, &str, &str); 2] = [
    (
        FIREBASE_ROUTE,
        "dist/remote-firebase-v1.js",
        "colab-remote-firebase-v1.js",
    ),
    (
        NOTICES_ROUTE,
        "dist/THIRD-PARTY-NOTICES.txt",
        "colab-remote-notices.txt",
    ),
];

#[derive(Serialize)]
struct Manifest {
    version: u8,
    files: Vec<File>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct File {
    path: String,
    sha256: String,
    length: usize,
    content_type: &'static str,
    csp: &'static str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bundle {
    version: u8,
    manifest_digest: String,
    files: Vec<Bytes>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bytes {
    path: String,
    bytes_base64: String,
}

fn invalid(message: &str) -> io::Error {
    io::Error::other(message)
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(crate) fn verify_sdk_copy(expected_digest: &str, copied: &[u8]) -> io::Result<()> {
    if digest(copied) != expected_digest {
        return Err(invalid("Remote SDK copy digest mismatch"));
    }
    Ok(())
}
/// Remote's pin of its hosted-only build: exactly the two `PINNED` files, each with the lowercase
/// SHA-256 and length of the bytes Remote built, in `PINNED` order. A pin that names more, fewer
/// or other files means Remote's output changed shape and this copy must be reviewed.
fn pins(client: &Path) -> io::Result<Vec<(String, usize)>> {
    let malformed = || invalid("Remote's Firebase build pin is malformed");
    let value: serde_json::Value = serde_json::from_slice(&read(&client.join(PIN))?)?;
    let files = value["files"]
        .as_array()
        .filter(|files| value["version"] == 1 && files.len() == PINNED.len())
        .ok_or_else(malformed)?;
    PINNED
        .iter()
        .map(|(_, path, _)| {
            let mut named = files.iter().filter(|file| file["path"] == *path);
            let (Some(entry), None) = (named.next(), named.next()) else {
                return Err(malformed());
            };
            let sha256 = entry["sha256"]
                .as_str()
                .filter(|sha| {
                    sha.len() == 64 && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                })
                .ok_or_else(malformed)?;
            let length = entry["length"].as_u64().ok_or_else(malformed)?;
            Ok((sha256.to_owned(), length as usize))
        })
        .collect()
}
fn read(path: &Path) -> io::Result<Vec<u8>> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.len() == 0 || meta.len() >= FILE_BYTES as u64 {
        return Err(invalid(
            "Hosted files must be nonempty regular files below 50% of Remote's file cap",
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(FILE_BYTES as u64)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() >= FILE_BYTES {
        return Err(invalid("Hosted file reached its byte budget"));
    }
    Ok(bytes)
}
fn content_type(route: &str) -> io::Result<&'static str> {
    Ok(match route.rsplit('.').next() {
        Some("html") => "text/html",
        Some("txt") => "text/plain",
        Some("css") => "text/css",
        Some("js") => "text/javascript",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => return Err(invalid("Unsupported hosted content type")),
    })
}
fn collect(
    directory: &Path,
    prefix: &str,
    files: &mut Vec<(String, Vec<u8>)>,
    inputs: &mut Vec<PathBuf>,
) -> io::Result<()> {
    if !fs::symlink_metadata(directory)?.is_dir() {
        return Err(invalid("Hosted directories must be real directories"));
    }
    inputs.push(directory.to_owned());
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| invalid("Hosted filename must be UTF-8"))?;
        if name.is_empty()
            || name.starts_with('.')
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        {
            return Err(invalid("Unsafe hosted filename"));
        }
        let route = format!("{prefix}/{name}");
        if route.len() > 1024
            || SDK_ROUTES.contains(&route.as_str())
            || route == "/__"
            || route.starts_with("/__/")
        {
            return Err(invalid("Hosted path is reserved or too long"));
        }
        let path = entry.path();
        if fs::symlink_metadata(&path)?.is_dir() {
            collect(&path, &route, files, inputs)?;
        } else {
            content_type(&route)?;
            files.push((route, read(&path)?));
            inputs.push(path);
            // Reserve the files of the upstream SDK copy.
            if files.len() + SDK_ROUTES.len() >= FILES {
                return Err(invalid("Hosted inventory reached 50% of Remote's file cap"));
            }
        }
    }
    Ok(())
}

/// `sdk` is Remote's committed `remote-v1.js`; `client` is Remote's client directory, whose pin
/// manifest vouches for the Firebase build in its `dist`. Remote's build must have run first.
pub fn generate(
    directory: Option<&Path>,
    sdk: &Path,
    client: &Path,
    output: &Path,
) -> io::Result<Vec<PathBuf>> {
    let Some(directory) = directory else {
        fs::write(
            output.join("colab_hosting.rs"),
            "const BUNDLE: Option<&[u8]> = None;\nconst MANIFEST: Option<&[u8]> = None;\n",
        )?;
        return Ok(Vec::new());
    };
    if !directory.is_absolute() || !sdk.is_absolute() || !client.is_absolute() {
        return Err(invalid("Hosted build paths must be absolute"));
    }
    let mut inputs = Vec::new();
    let mut files = Vec::new();
    collect(directory, "", &mut files, &mut inputs)?;
    for route in ["/index.html", "/THIRD-PARTY-NOTICES.txt"] {
        if !files.iter().any(|(path, _)| path == route) {
            return Err(invalid("Hosted entry or dependency notices are missing"));
        }
    }
    // Remote's built output is the only SDK source. The input distribution cannot override it.
    let sdk_bytes = read(sdk)?;
    let sdk_digest = digest(&sdk_bytes);
    let sdk_copy = output.join("colab-remote-v1.js");
    fs::copy(sdk, &sdk_copy)?;
    let copied = read(&sdk_copy)?;
    verify_sdk_copy(&sdk_digest, &copied)?;
    files.push((SDK_ROUTE.into(), copied));
    inputs.push(sdk.to_owned());
    // Remote's pinned build output is copied byte for byte; nothing else may stand in for it.
    for ((route, source, copy), (sha256, length)) in PINNED.iter().zip(pins(client)?) {
        let from = client.join(source);
        let built = read(&from)?;
        if built.len() != length || digest(&built) != sha256 {
            return Err(invalid(
                "Remote's Firebase build output does not match its pin",
            ));
        }
        let kept = output.join(copy);
        fs::copy(&from, &kept)?;
        let copied = read(&kept)?;
        verify_sdk_copy(&sha256, &copied)?;
        files.push(((*route).into(), copied));
        inputs.push(from);
    }
    inputs.push(client.join(PIN));
    files.sort_by(|a, b| a.0.cmp(&b.0));
    if files.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(invalid("Duplicate hosted path"));
    }
    let total: usize = files.iter().map(|(_, bytes)| bytes.len()).sum();
    if total >= TOTAL_BYTES {
        return Err(invalid(
            "Hosted inventory reached 50% of Remote's total byte cap",
        ));
    }
    let mut manifest = Manifest {
        version: 1,
        files: Vec::new(),
    };
    let mut bundle = Bundle {
        version: 1,
        manifest_digest: String::new(),
        files: Vec::new(),
    };
    for (route, bytes) in files {
        let hash = digest(&bytes);
        manifest.files.push(File {
            content_type: content_type(&route)?,
            csp: if route == "/renderer.html" {
                browser_policy::RENDERER_POLICY
            } else {
                browser_policy::POLICY
            },
            path: route.clone(),
            sha256: hash,
            length: bytes.len(),
        });
        bundle.files.push(Bytes {
            path: route,
            bytes_base64: STANDARD.encode(&bytes),
        });
    }
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    bundle.manifest_digest = digest(&manifest_bytes);
    fs::write(output.join("colab-hosting-manifest.json"), manifest_bytes)?;
    fs::write(
        output.join("colab-hosting-bundle.json"),
        serde_json::to_vec(&bundle)?,
    )?;
    fs::write(
        output.join("colab_hosting.rs"),
        format!(
            "const BUNDLE: Option<&[u8]> = Some(include_bytes!({:?}));\nconst MANIFEST: Option<&[u8]> = Some(include_bytes!({:?}));\n",
            output.join("colab-hosting-bundle.json"),
            output.join("colab-hosting-manifest.json")
        ),
    )?;
    Ok(inputs)
}
