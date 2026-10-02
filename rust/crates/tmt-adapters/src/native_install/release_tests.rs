use super::{OFFICIAL_REPOSITORY, artifact, download};
use flate2::{Compression, write::GzEncoder};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io,
    time::{Duration, Instant},
};
use tar::{Builder, EntryType, Header};
use tmt_core::content_digest::sha256;
use tmt_core::native_install::{Channel, Product};

const TARGET: &str = "aarch64-apple-darwin";
const MANIFEST_NAME: &str = "dist-manifest.json";

#[derive(Debug)]
struct Call {
    url: String,
    accept: String,
    maximum: usize,
}

#[derive(Default)]
struct HttpFixture {
    responses: BTreeMap<String, Vec<u8>>,
    calls: Vec<Call>,
}

impl HttpFixture {
    fn response(&mut self, url: String, bytes: Vec<u8>) {
        self.responses.insert(url, bytes);
    }

    fn page(&mut self, page: usize, releases: &[Value]) {
        self.response(page_url(page), serde_json::to_vec(releases).unwrap());
    }

    fn exact(&mut self, version: &str, release: &Value) {
        self.response(exact_url(version), serde_json::to_vec(release).unwrap());
    }

    fn assets(&mut self, release: &Value, manifest: &[u8], archive: &[u8]) {
        for asset in release["assets"].as_array().unwrap() {
            let id = asset["id"].as_u64().unwrap();
            let bytes = match asset["name"].as_str().unwrap() {
                MANIFEST_NAME => manifest,
                _ => archive,
            };
            self.response(asset_url(id), bytes.to_vec());
        }
    }

    fn get(
        &mut self,
        url: &str,
        accept: &str,
        maximum: usize,
        _deadline: Instant,
    ) -> io::Result<Vec<u8>> {
        self.calls.push(Call {
            url: url.into(),
            accept: accept.into(),
            maximum,
        });
        self.responses
            .get(url)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "fixture response missing"))
    }
}

fn endpoint() -> String {
    format!("https://api.github.com/repos/{OFFICIAL_REPOSITORY}/releases")
}

fn page_url(page: usize) -> String {
    format!("{}?per_page=100&page={page}", endpoint())
}

fn exact_url(version: &str) -> String {
    format!("{}/tags/v{version}", endpoint())
}

fn asset_url(id: u64) -> String {
    format!("{}/assets/{id}", endpoint())
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
}

fn call_download(
    fixture: &mut HttpFixture,
    channel: Channel,
    exact: Option<&str>,
    target: &str,
) -> io::Result<super::DownloadedRelease> {
    let exact = exact.map(|version| version.parse().unwrap());
    download(
        channel,
        exact.as_ref(),
        target,
        deadline(),
        |url, accept, maximum, deadline| fixture.get(url, accept, maximum, deadline),
    )
}

fn append_file(builder: &mut Builder<GzEncoder<Vec<u8>>>, path: &str, bytes: &[u8], mode: u32) {
    let mut header = Header::new_gnu();
    header.set_path(path).unwrap();
    header.set_entry_type(EntryType::Regular);
    header.set_mode(mode);
    header.set_size(bytes.len() as u64);
    header.set_cksum();
    builder.append(&header, bytes).unwrap();
}

fn archive(
    product: Product,
    version: &str,
    target: &str,
    companions: &[(&str, &[u8])],
) -> (String, Vec<u8>) {
    let name = format!("{}-{version}-{target}.tar.gz", product.package());
    let root = name.strip_suffix(".tar.gz").unwrap();
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = Builder::new(encoder);
    append_file(
        &mut builder,
        &format!("{root}/{}", product.executable()),
        b"native executable\n",
        0o755,
    );
    append_file(&mut builder, &format!("{root}/LICENSE"), b"MIT\n", 0o644);
    append_file(
        &mut builder,
        &format!("{root}/NATIVE-INSTALL.md"),
        b"Native install\n",
        0o644,
    );
    append_file(
        &mut builder,
        &format!("{root}/THIRD-PARTY-NOTICES.txt"),
        b"Third-party notices\n",
        0o644,
    );
    for (name, bytes) in companions {
        append_file(&mut builder, &format!("{root}/{name}"), bytes, 0o755);
    }
    builder.finish().unwrap();
    (name, builder.into_inner().unwrap().finish().unwrap())
}

/// Shared real archive fixture for native-install orchestration tests.
pub(in crate::native_install) fn valid_fixture(
    version: &str,
    target: &str,
    release_id: u64,
) -> (Value, Vec<u8>, Vec<u8>, String) {
    product_fixture(Product::Cli, version, target, release_id)
}

pub(in crate::native_install) fn product_fixture(
    product: Product,
    version: &str,
    target: &str,
    release_id: u64,
) -> (Value, Vec<u8>, Vec<u8>, String) {
    companion_fixture(product, version, target, release_id, &[])
}

/// A release that also carries `companions`, each archived and listed in
/// the manifest as cargo-dist lists a binary.
pub(in crate::native_install) fn companion_fixture(
    product: Product,
    version: &str,
    target: &str,
    release_id: u64,
    companions: &[(&str, &[u8])],
) -> (Value, Vec<u8>, Vec<u8>, String) {
    let (archive_name, archive) = archive(product, version, target, companions);
    let manifest = serde_json::to_vec(&json!({
        "artifacts": {
            archive_name.clone(): {
                "kind": "executable-zip",
                "name": archive_name.clone(),
                "target_triples": [target],
                "checksums": {"sha256": sha256(&archive)},
                "assets": product.files()
                    .iter()
                    .copied()
                    .chain(companions.iter().map(|(name, _)| *name))
                    .map(|path| json!({"path": path}))
                    .collect::<Vec<_>>(),
            }
        },
        "releases": [{
            "app_name": product.package(),
            "app_version": version,
            "artifacts": [archive_name.clone()]
        }]
    }))
    .unwrap();
    let release = json!({
        "id": release_id,
        "tag_name": format!("{}{version}", product.tag_prefix()),
        "draft": false,
        "immutable": true,
        "prerelease": version.contains('-'),
        "assets": [
            {
                "id": release_id * 10 + 1,
                "name": MANIFEST_NAME,
                "state": "uploaded",
                "size": manifest.len(),
                "digest": format!("sha256:{}", sha256(&manifest)),
            },
            {
                "id": release_id * 10 + 2,
                "name": archive_name,
                "state": "uploaded",
                "size": archive.len(),
                "digest": format!("sha256:{}", sha256(&archive)),
            }
        ]
    });
    (release, manifest, archive, archive_name)
}

fn register(fixture: &mut HttpFixture, release: &Value, manifest: &[u8], archive: &[u8]) {
    fixture.assets(release, manifest, archive);
}

fn update_manifest_asset(release: &mut Value, manifest: &[u8]) {
    let asset = release["assets"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|asset| asset["name"] == MANIFEST_NAME)
        .unwrap();
    asset["size"] = json!(manifest.len());
    asset["digest"] = json!(format!("sha256:{}", sha256(manifest)));
}

#[test]
fn office_discovery_ignores_cli_versions_and_uses_its_own_exact_tags() {
    let (cli, _, _, _) = valid_fixture("99.0.0", TARGET, 400);
    let (office, manifest, archive, _) =
        product_fixture(Product::Office, "0.1.0-alpha.1", TARGET, 401);
    let mut fixture = HttpFixture::default();
    fixture.page(1, &[cli, office.clone()]);
    register(&mut fixture, &office, &manifest, &archive);
    let result = super::download_product(
        Product::Office,
        Channel::Alpha,
        None,
        TARGET,
        deadline(),
        |u, a, l, d| fixture.get(u, a, l, d),
    )
    .unwrap();
    assert_eq!(result.version.to_string(), "0.1.0-alpha.1");
    assert_eq!(
        fixture
            .calls
            .iter()
            .map(|c| c.url.clone())
            .collect::<Vec<_>>(),
        vec![page_url(1), asset_url(4011), asset_url(4012)]
    );
    let artifact = artifact::acquire_bytes(
        Product::Office,
        &result.manifest,
        &result.archive_name,
        &result.archive,
        TARGET,
    )
    .unwrap();
    assert!(artifact.files.contains_key("tmt-office"));
    assert!(!artifact.files.contains_key("tmt"));
    fixture.calls.clear();
    let url = format!("{}/tags/tmt-office-v0.1.0-alpha.1", endpoint());
    fixture.response(url.clone(), serde_json::to_vec(&office).unwrap());
    super::download_product(
        Product::Office,
        Channel::Alpha,
        Some(&"0.1.0-alpha.1".parse().unwrap()),
        TARGET,
        deadline(),
        |u, a, l, d| fixture.get(u, a, l, d),
    )
    .unwrap();
    assert_eq!(fixture.calls[0].url, url);
}

#[test]
fn cli_only_releases_are_not_an_office_installation_candidate() {
    let (cli, _, _, _) = valid_fixture("99.0.0-alpha.1", TARGET, 410);
    let mut fixture = HttpFixture::default();
    fixture.page(1, &[cli]);
    let error = super::download_product(
        Product::Office,
        Channel::Alpha,
        None,
        TARGET,
        deadline(),
        |u, a, l, d| fixture.get(u, a, l, d),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(
        error.to_string(),
        "No release is available in the selected native channel."
    );
    assert_eq!(fixture.calls.len(), 1);
}

#[test]
fn selects_latest_stable_and_alpha_versions_and_uses_canonical_asset_urls() {
    let (stable_old, _, _, _) = valid_fixture("1.2.3", TARGET, 101);
    let (stable_new, stable_manifest, stable_archive, _) = valid_fixture("1.10.0", TARGET, 102);
    let (alpha, alpha_manifest, alpha_archive, _) = valid_fixture("1.11.0-alpha.1", TARGET, 103);
    let mut fixture = HttpFixture::default();
    fixture.page(1, &[stable_old, stable_new.clone(), alpha.clone()]);
    register(&mut fixture, &stable_new, &stable_manifest, &stable_archive);
    register(&mut fixture, &alpha, &alpha_manifest, &alpha_archive);

    let result = call_download(&mut fixture, Channel::Stable, None, TARGET).unwrap();
    assert_eq!(result.version.to_string(), "1.10.0");
    assert_eq!(
        fixture
            .calls
            .iter()
            .map(|call| call.url.as_str())
            .collect::<Vec<_>>(),
        vec![page_url(1), asset_url(1021), asset_url(1022)]
    );
    assert!(
        fixture.calls[1..]
            .iter()
            .all(|call| call.accept == "application/octet-stream")
    );
    assert_eq!(fixture.calls[1].maximum, artifact::MANIFEST_LIMIT);

    fixture.calls.clear();
    let result = call_download(&mut fixture, Channel::Alpha, None, TARGET).unwrap();
    assert_eq!(result.version.to_string(), "1.11.0-alpha.1");
    assert_eq!(fixture.calls[1].url, asset_url(1031));
    assert_eq!(fixture.calls[2].url, asset_url(1032));
}

#[test]
fn exact_selection_uses_tag_api_and_numeric_asset_endpoints() {
    let (release, manifest, archive, _) = valid_fixture("1.2.3", TARGET, 220);
    let mut fixture = HttpFixture::default();
    fixture.exact("1.2.3", &release);
    register(&mut fixture, &release, &manifest, &archive);
    call_download(&mut fixture, Channel::Stable, Some("1.2.3"), TARGET).unwrap();
    assert_eq!(fixture.calls[0].url, exact_url("1.2.3"));
    assert_eq!(fixture.calls[1].url, asset_url(2201));
    assert_eq!(fixture.calls[2].url, asset_url(2202));
    assert!(
        fixture
            .calls
            .iter()
            .all(|call| !call.url.contains("/releases/download/"))
    );
}

#[test]
fn rejects_draft_mutable_and_mismatched_prerelease_metadata() {
    let (base, _, _, _) = valid_fixture("1.2.3", TARGET, 230);
    for (field, value) in [
        ("draft", json!(true)),
        ("immutable", json!(false)),
        ("prerelease", json!(true)),
    ] {
        let mut release = base.clone();
        release[field] = value;
        let mut fixture = HttpFixture::default();
        fixture.exact("1.2.3", &release);
        assert!(call_download(&mut fixture, Channel::Stable, Some("1.2.3"), TARGET).is_err());
        assert_eq!(fixture.calls.len(), 1);
    }
    let (mut missing, _, _, _) = valid_fixture("1.2.3", TARGET, 232);
    missing.as_object_mut().unwrap().remove("prerelease");
    let mut fixture = HttpFixture::default();
    fixture.exact("1.2.3", &missing);
    assert!(call_download(&mut fixture, Channel::Stable, Some("1.2.3"), TARGET).is_err());
}

/// Downloads `version` of `product` by exact tag, with its release's
/// `prerelease` flag set to `flagged`.
fn download_flagged(product: Product, version: &str, flagged: bool) -> io::Result<()> {
    let (mut release, manifest, archive, _) = product_fixture(product, version, TARGET, 233);
    release["prerelease"] = json!(flagged);
    let mut fixture = HttpFixture::default();
    let url = format!("{}/tags/{}{version}", endpoint(), product.tag_prefix());
    fixture.response(url, serde_json::to_vec(&release).unwrap());
    register(&mut fixture, &release, &manifest, &archive);
    let channel = if version.contains('-') {
        Channel::Alpha
    } else {
        Channel::Stable
    };
    super::download_product(
        product,
        channel,
        Some(&version.parse().unwrap()),
        TARGET,
        deadline(),
        |u, a, l, d| fixture.get(u, a, l, d),
    )
    .map(|_| ())
}

#[test]
fn a_cli_alpha_may_be_a_normal_release_but_extensions_stay_flagged() {
    // The CLI is published as a normal release (the repository's latest);
    // alpha.2-6 were flagged prereleases. A stable CLI is never flagged.
    download_flagged(Product::Cli, "5.0.0-alpha.7", false).unwrap();
    download_flagged(Product::Cli, "5.0.0-alpha.6", true).unwrap();
    assert!(download_flagged(Product::Cli, "5.0.0", true).is_err());
    for extension in [Product::Office, Product::Squad] {
        download_flagged(extension, "0.1.0-alpha.4", true).unwrap();
        assert!(download_flagged(extension, "0.1.0-alpha.4", false).is_err());
    }
}

/// The GitHub prerelease flag each product is published with, pinned here
/// and in `typescript/test/tooling/native-release-policy.test.ts` against
/// `typescript/scripts/native-release-policy.mjs`. Change all three together:
/// this is a two-sided pin, not an automatic cross-check.
#[test]
fn the_publication_policy_publishes_flags_the_updater_accepts() {
    let alpha = "1.0.0-alpha.1".parse().unwrap();
    for (product, published) in [
        (Product::Cli, false),
        (Product::Office, true),
        (Product::Squad, true),
    ] {
        assert!(
            product.accepts_prerelease_flag(&alpha, published),
            "{} publishes prerelease={published}, which tmt upgrade refuses",
            product.as_str()
        );
    }
}

#[test]
fn rejects_channel_and_exact_version_mismatches() {
    let (alpha, _, _, _) = valid_fixture("1.3.0-alpha.1", TARGET, 240);
    let mut fixture = HttpFixture::default();
    fixture.exact("1.3.0-alpha.1", &alpha);
    assert!(call_download(&mut fixture, Channel::Stable, Some("1.3.0-alpha.1"), TARGET).is_err());

    let (release, _, _, _) = valid_fixture("1.2.3", TARGET, 241);
    fixture.responses.clear();
    fixture.calls.clear();
    fixture.exact("1.2.4", &release);
    assert!(call_download(&mut fixture, Channel::Stable, Some("1.2.4"), TARGET).is_err());
}

#[test]
fn missing_release_is_not_reported_as_current_or_successful() {
    let mut fixture = HttpFixture::default();
    fixture.page(1, &[]);
    let error = call_download(&mut fixture, Channel::Stable, None, TARGET)
        .err()
        .expect("empty discovery should fail");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(fixture.calls.len(), 1);
}

#[test]
fn release_discovery_stops_at_its_three_page_bound() {
    let mut fixture = HttpFixture::default();
    for page in 1..=3 {
        let releases = (0..100)
            .map(|offset| {
                json!({
                    "id": page * 1000 + offset,
                    "tag_name": format!("v1.{page}.{offset}"),
                    "draft": false,
                    "immutable": true,
                    "prerelease": false,
                })
            })
            .collect::<Vec<_>>();
        fixture.page(page, &releases);
    }
    assert!(call_download(&mut fixture, Channel::Stable, None, TARGET).is_err());
    assert_eq!(
        fixture
            .calls
            .iter()
            .map(|call| call.url.as_str())
            .collect::<Vec<_>>(),
        vec![page_url(1), page_url(2), page_url(3)]
    );
}

#[test]
fn malformed_release_metadata_is_rejected_before_asset_requests() {
    let mut fixture = HttpFixture::default();
    fixture.response(exact_url("1.2.3"), b"{".to_vec());
    assert!(call_download(&mut fixture, Channel::Stable, Some("1.2.3"), TARGET).is_err());
    assert_eq!(fixture.calls.len(), 1);
    fixture.calls.clear();
    fixture.response(exact_url("1.2.3"), b"[]".to_vec());
    assert!(call_download(&mut fixture, Channel::Stable, Some("1.2.3"), TARGET).is_err());
    assert_eq!(fixture.calls.len(), 1);
}

#[test]
fn duplicate_and_missing_assets_are_rejected() {
    let (mut release, _, _, _) = valid_fixture("1.2.3", TARGET, 250);
    let duplicate = release["assets"][0].clone();
    release["assets"].as_array_mut().unwrap().push(duplicate);
    let mut fixture = HttpFixture::default();
    fixture.exact("1.2.3", &release);
    assert!(call_download(&mut fixture, Channel::Stable, Some("1.2.3"), TARGET).is_err());
    assert_eq!(fixture.calls.len(), 1);

    let (mut release, manifest, archive, _) = valid_fixture("1.2.3", TARGET, 251);
    release["assets"] = json!([release["assets"][0].clone()]);
    fixture.responses.clear();
    fixture.calls.clear();
    fixture.exact("1.2.3", &release);
    fixture.response(asset_url(2511), manifest);
    fixture.response(asset_url(2512), archive);
    assert!(call_download(&mut fixture, Channel::Stable, Some("1.2.3"), TARGET).is_err());
    assert_eq!(fixture.calls.len(), 2);
}

#[test]
fn api_size_and_digest_mismatches_are_rejected() {
    let (base, manifest, archive, _) = valid_fixture("1.2.3", TARGET, 260);
    let mut release = base.clone();
    release["assets"][1]["size"] = json!(archive.len() + 1);
    let mut fixture = HttpFixture::default();
    fixture.exact("1.2.3", &release);
    register(&mut fixture, &release, &manifest, &archive);
    assert!(call_download(&mut fixture, Channel::Stable, Some("1.2.3"), TARGET).is_err());

    let mut release = base;
    release["assets"][1]["digest"] = json!(format!("sha256:{}", "0".repeat(64)));
    fixture.responses.clear();
    fixture.calls.clear();
    fixture.exact("1.2.3", &release);
    register(&mut fixture, &release, &manifest, &archive);
    assert!(call_download(&mut fixture, Channel::Stable, Some("1.2.3"), TARGET).is_err());
}

#[test]
fn manifest_target_and_version_mismatches_are_rejected() {
    let (mut release, mut manifest, _, archive_name) = valid_fixture("1.2.3", TARGET, 270);
    manifest = serde_json::to_vec(&{
        let mut value: Value = serde_json::from_slice(&manifest).unwrap();
        value["artifacts"][archive_name.clone()]["target_triples"] =
            json!(["x86_64-unknown-linux-gnu"]);
        value
    })
    .unwrap();
    update_manifest_asset(&mut release, &manifest);
    let mut fixture = HttpFixture::default();
    fixture.exact("1.2.3", &release);
    fixture.response(asset_url(2701), manifest.clone());
    assert!(call_download(&mut fixture, Channel::Stable, Some("1.2.3"), TARGET).is_err());
    assert_eq!(fixture.calls.len(), 2);

    let (mut release, mut manifest, _, _) = valid_fixture("1.2.3", TARGET, 271);
    let mut value: Value = serde_json::from_slice(&manifest).unwrap();
    value["releases"][0]["app_version"] = json!("1.2.4");
    manifest = serde_json::to_vec(&value).unwrap();
    update_manifest_asset(&mut release, &manifest);
    fixture.responses.clear();
    fixture.calls.clear();
    fixture.exact("1.2.3", &release);
    fixture.response(asset_url(2711), manifest);
    assert!(call_download(&mut fixture, Channel::Stable, Some("1.2.3"), TARGET).is_err());
    assert_eq!(fixture.calls.len(), 2);
}
