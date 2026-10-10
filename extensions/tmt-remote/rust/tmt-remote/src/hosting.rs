//! Pure release-embedded static bundle validation and deterministic Hosting composition.
//! No file, process, provider or credential access; public bytes grant no authority.
use crate::{deploy_plan::sha256_hex, limits, wire};
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compression, GzBuilder};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Write};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostingManifest {
    pub version: u8,
    pub files: Vec<HostingFile>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostingFile {
    pub path: String,
    pub sha256: String,
    pub length: u64,
    pub content_type: String,
    #[serde(
        default,
        deserialize_with = "optional_csp",
        skip_serializing_if = "Option::is_none"
    )]
    pub csp: Option<String>,
}
fn optional_csp<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostingRefusal {
    Shape,
    Bounds,
    Path,
    Reserved,
    Digest,
    ContentType,
    Csp,
    FileSet,
    Collision,
    Entry,
    Inventory,
    WebAppSelection,
    Compression,
}
impl HostingManifest {
    pub fn parse(value: &Value) -> Result<Self, HostingRefusal> {
        let manifest: Self =
            serde_json::from_value(value.clone()).map_err(|_| HostingRefusal::Shape)?;
        if manifest.version != 1 || manifest.files.is_empty() {
            return Err(HostingRefusal::Shape);
        }
        if manifest.files.len() > limits::HOSTING_FILES {
            return Err(HostingRefusal::Bounds);
        }
        let mut total = 0u64;
        let mut previous: Option<&str> = None;
        for file in &manifest.files {
            if !path_ok(&file.path) {
                return Err(HostingRefusal::Path);
            }
            if file.path == "/__" || file.path.starts_with("/__/") {
                return Err(HostingRefusal::Reserved);
            }
            if previous.is_some_and(|path| path >= file.path.as_str()) {
                return Err(HostingRefusal::Collision);
            }
            previous = Some(&file.path);
            if !digest_ok(&file.sha256) {
                return Err(HostingRefusal::Digest);
            }
            if !content_type_ok(&file.content_type) {
                return Err(HostingRefusal::ContentType);
            }
            if file.csp.as_ref().is_some_and(|csp| {
                csp.is_empty()
                    || csp.len() > limits::HOSTING_CSP_BYTES
                    || !csp.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
            }) {
                return Err(HostingRefusal::Csp);
            }
            total = total
                .checked_add(file.length)
                .ok_or(HostingRefusal::Bounds)?;
            if file.length > limits::HOSTING_FILE_BYTES as u64
                || total > limits::HOSTING_TOTAL_BYTES as u64
            {
                return Err(HostingRefusal::Bounds);
            }
        }
        Ok(manifest)
    }
    /// Canonical manifest identity, independent of declaration whitespace.
    pub fn digest(&self) -> String {
        sha256_hex(&serde_json::to_vec(self).expect("manifest serializes"))
    }
}
fn path_ok(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= limits::HOSTING_PATH_BYTES
        && path[1..].split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        })
}
fn digest_ok(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
fn content_type_ok(value: &str) -> bool {
    matches!(
        value,
        "text/html"
            | "text/plain"
            | "text/css"
            | "text/javascript"
            | "application/javascript"
            | "application/json"
            | "image/svg+xml"
            | "image/png"
            | "image/jpeg"
            | "image/webp"
            | "image/x-icon"
            | "font/woff"
            | "font/woff2"
    )
}

fn response_headers(file: &HostingFile) -> Value {
    let content_type = if matches!(
        file.content_type.as_str(),
        "text/html"
            | "text/css"
            | "text/javascript"
            | "application/javascript"
            | "application/json"
            | "text/plain"
    ) {
        format!("{}; charset=utf-8", file.content_type)
    } else {
        file.content_type.clone()
    };
    let mut headers = json!({"Content-Type":content_type,"X-Content-Type-Options":"nosniff",
        "Cache-Control":"no-cache","Referrer-Policy":"no-referrer"});
    if let Some(csp) = &file.csp {
        headers["Content-Security-Policy"] = json!(csp);
    }
    headers
}
const SHORT_ROUTES: [&str; 2] = ["^/colab/?$", "^/(p|read)/[A-Za-z0-9_-]{4,64}$"];

/// One captured extension reply, checked against the declaration before provider setup.
pub struct HostingBundle {
    manifest: HostingManifest,
    files: BTreeMap<String, Vec<u8>>,
}
impl HostingBundle {
    pub fn parse(manifest: &HostingManifest, bytes: &[u8]) -> Result<Self, HostingRefusal> {
        HostingManifest::parse(
            &serde_json::to_value(manifest).map_err(|_| HostingRefusal::Shape)?,
        )?;
        if bytes.len() > limits::HOSTING_REPLY_BYTES {
            return Err(HostingRefusal::Bounds);
        }
        let value = wire::strict_json(bytes).ok_or(HostingRefusal::Shape)?;
        let object = value.as_object().ok_or(HostingRefusal::Shape)?;
        if object.len() != 3
            || value["version"] != 1
            || value["manifestDigest"] != manifest.digest()
        {
            return Err(HostingRefusal::Digest);
        }
        let listed = value["files"].as_array().ok_or(HostingRefusal::Shape)?;
        if listed.len() != manifest.files.len() {
            return Err(HostingRefusal::FileSet);
        }
        let mut files = BTreeMap::new();
        for (item, expected) in listed.iter().zip(&manifest.files) {
            let item = item.as_object().ok_or(HostingRefusal::Shape)?;
            if item.len() != 2
                || item.get("path").and_then(Value::as_str) != Some(expected.path.as_str())
            {
                return Err(HostingRefusal::FileSet);
            }
            let encoded = item
                .get("bytesBase64")
                .and_then(Value::as_str)
                .ok_or(HostingRefusal::Shape)?;
            let raw = STANDARD
                .decode(encoded)
                .map_err(|_| HostingRefusal::Shape)?;
            if STANDARD.encode(&raw) != encoded {
                return Err(HostingRefusal::Shape);
            }
            if raw.len() as u64 != expected.length || sha256_hex(&raw) != expected.sha256 {
                return Err(HostingRefusal::Digest);
            }
            files.insert(expected.path.clone(), raw);
        }
        Ok(Self {
            manifest: manifest.clone(),
            files,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostingTransportFile {
    pub path: String,
    pub raw_digest: String,
    pub raw_length: u64,
    pub gzip_digest: String,
    pub gzip_length: usize,
    pub content_type: String,
}
/// Frozen content, transport identities and REST ServingConfig, all covered by its digest.
pub struct HostingComposition {
    files: Vec<HostingTransportFile>,
    config: Value,
    compressed: BTreeMap<String, Vec<u8>>,
    digest: String,
}
impl HostingComposition {
    pub fn files(&self) -> &[HostingTransportFile] {
        &self.files
    }
    pub fn config(&self) -> &Value {
        &self.config
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn gzip(&self, path: &str) -> Option<&[u8]> {
        self.compressed.get(path).map(Vec::as_slice)
    }
    pub fn view(&self) -> Value {
        json!({"version":1,"files":self.files,"config":self.config,"digest":self.digest})
    }
}
pub fn compose(bundles: &[HostingBundle]) -> Result<Option<HostingComposition>, HostingRefusal> {
    if bundles.is_empty() {
        return Ok(None);
    }
    let mut files = BTreeMap::new();
    let mut total = 0u64;
    for bundle in bundles {
        for file in &bundle.manifest.files {
            total = total
                .checked_add(file.length)
                .ok_or(HostingRefusal::Bounds)?;
            if files
                .insert(file.path.clone(), (file, &bundle.files[&file.path]))
                .is_some()
            {
                return Err(HostingRefusal::Collision);
            }
        }
    }
    if files.len() > limits::HOSTING_FILES || total > limits::HOSTING_TOTAL_BYTES as u64 {
        return Err(HostingRefusal::Bounds);
    }
    for path in files.keys() {
        let mut ancestor = path.as_str();
        while let Some((parent, _)) = ancestor.rsplit_once('/') {
            if files.contains_key(parent) {
                return Err(HostingRefusal::Collision);
            }
            ancestor = parent;
        }
    }
    if files
        .get("/index.html")
        .is_none_or(|(file, _)| file.content_type != "text/html")
    {
        return Err(HostingRefusal::Entry);
    }
    let shell_headers = response_headers(files["/index.html"].0);
    let mut transport = Vec::new();
    let mut compressed = BTreeMap::new();
    let mut headers = Vec::new();
    for (path, (file, raw)) in files {
        let mut writer = GzBuilder::new()
            .mtime(0)
            .operating_system(255)
            .write(Vec::new(), Compression::new(6));
        writer
            .write_all(raw)
            .map_err(|_| HostingRefusal::Compression)?;
        let gzip = writer.finish().map_err(|_| HostingRefusal::Compression)?;
        transport.push(HostingTransportFile {
            path: path.clone(),
            raw_digest: file.sha256.clone(),
            raw_length: file.length,
            gzip_digest: sha256_hex(&gzip),
            gzip_length: gzip.len(),
            content_type: file.content_type.clone(),
        });
        headers.push(json!({"glob":path,"headers":response_headers(file)}));
        compressed.insert(path.clone(), gzip);
    }
    // REST headers match the original request path, not the rewrite destination.
    // The short entries therefore need the shell's exact headers on their own patterns.
    for regex in SHORT_ROUTES {
        headers.push(json!({"regex":regex,"headers":shell_headers}));
    }
    let rewrites: Vec<_> = SHORT_ROUTES
        .iter()
        .map(|regex| json!({"regex":regex,"path":"/index.html"}))
        .collect();
    // Only public shell routes; reserved Firebase configuration is never rewritten.
    let config = json!({"headers":headers,"rewrites":rewrites});
    let digest = sha256_hex(
        &serde_json::to_vec(&json!({"files":transport,"config":config}))
            .expect("composition serializes"),
    );
    Ok(Some(HostingComposition {
        files: transport,
        config,
        compressed,
        digest,
    }))
}

/// Read-only site inventory supplied by the provider edge; never a guessed empty site.
pub struct HostingInventory {
    pub site_exists: bool,
    pub web_apps: Vec<String>,
    pub live: Option<HostingLiveRelease>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostingLiveRelease {
    pub version: String,
    pub deployment_id: Option<String>,
    pub plan_digest_prefix: Option<String>,
    pub content_digest: Option<String>,
    pub files: Vec<HostingTransportFile>,
    pub config: Value,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostingDeploymentView {
    pub site: String,
    pub public_url: String,
    pub create_site: bool,
    pub web_app: Option<String>,
    pub create_web_app: bool,
    pub content: Value,
    pub replaces: &'static str,
    pub replaced_fingerprint: Option<String>,
}
/// Ownership labels plus exact content identify an own release. Labels alone never do.
/// Foreign fingerprints include the captured release identity and complete file/config set.
pub fn deployment_view(
    composition: &HostingComposition,
    project: &str,
    deployment_id: &str,
    inventory: &HostingInventory,
) -> Result<HostingDeploymentView, HostingRefusal> {
    if !crate::deploy_run::project_ok(project)
        || crate::canonical::uuid(deployment_id).is_err()
        || inventory.web_apps.len() > limits::HOSTING_WEB_APPS
    {
        return Err(HostingRefusal::Inventory);
    }
    let valid_app = |value: &str| {
        !value.is_empty()
            && value.len() <= 256
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'-'))
    };
    let mut apps = inventory.web_apps.clone();
    apps.sort();
    if apps.iter().any(|app| !valid_app(app)) || apps.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(HostingRefusal::Inventory);
    }
    let web_app = match apps.as_slice() {
        [] => None,
        [only] => Some(only.clone()),
        _ => return Err(HostingRefusal::WebAppSelection),
    };
    let mut replaces = "none";
    let mut replaced_fingerprint = None;
    if let Some(live) = &inventory.live {
        if !inventory.site_exists
            || live.version.is_empty()
            || live.version.len() > 256
            || live.version.chars().any(char::is_control)
            || live.files.len() > limits::HOSTING_FILES
            || serde_json::to_vec(&live.config)
                .map_err(|_| HostingRefusal::Inventory)?
                .len()
                > limits::HOSTING_CONFIG_BYTES
        {
            return Err(HostingRefusal::Inventory);
        }
        let mut files = live.files.clone();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        if files.windows(2).any(|p| p[0].path == p[1].path)
            || files.iter().any(|f| {
                !path_ok(&f.path)
                    || !digest_ok(&f.raw_digest)
                    || !digest_ok(&f.gzip_digest)
                    || !content_type_ok(&f.content_type)
                    || f.raw_length > limits::HOSTING_FILE_BYTES as u64
                    || f.gzip_length > limits::HOSTING_FILE_BYTES + 65536
            })
        {
            return Err(HostingRefusal::Inventory);
        }
        let content = json!({"files":&files,"config":live.config});
        let digest = sha256_hex(&serde_json::to_vec(&content).expect("content serializes"));
        let own = live.deployment_id.as_deref() == Some(deployment_id)
            && live.content_digest.as_deref() == Some(&digest)
            && live.plan_digest_prefix.as_deref().is_some_and(|p| {
                p.len() == 12 && p.bytes().all(|b| matches!(b,b'0'..=b'9'|b'a'..=b'f'))
            });
        if own {
            replaces = if digest == composition.digest() {
                "none"
            } else {
                "own"
            };
        } else {
            replaces = "foreign";
            let mut canonical = live.clone();
            canonical.files = files;
            replaced_fingerprint = Some(sha256_hex(
                &serde_json::to_vec(&canonical).expect("release serializes"),
            ));
        }
    }
    Ok(HostingDeploymentView {
        site: project.into(),
        public_url: format!("https://{project}.web.app"),
        create_site: !inventory.site_exists,
        create_web_app: web_app.is_none(),
        web_app,
        content: composition.view(),
        replaces,
        replaced_fingerprint,
    })
}
