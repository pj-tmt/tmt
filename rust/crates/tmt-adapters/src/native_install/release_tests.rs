use super::{OFFICIAL_REPOSITORY, artifact, download};
use flate2::{Compression, write::GzEncoder};
use semver::Version;
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
    links: BTreeMap<String, String>,
}

impl HttpFixture {
    fn response(&mut self, url: String, bytes: Vec<u8>) {
        self.responses.insert(url, bytes);
    }

    fn releases(&mut self, releases: &[Value]) {
        for product in Product::ALL {
            let refs = releases
                .iter()
                .filter_map(|release| {
                    let tag = release["tag_name"].as_str()?;
                    tag.starts_with(product.tag_prefix())
                        .then(|| json!({"ref": format!("refs/tags/{tag}")}))
                })
                .collect::<Vec<_>>();
            self.response(refs_url(product), serde_json::to_vec(&refs).unwrap());
        }
        for release in releases {
            self.response(
                format!(
                    "{}/tags/{}",
                    endpoint(),
                    release["tag_name"].as_str().unwrap()
                ),
                serde_json::to_vec(release).unwrap(),
            );
        }
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
    ) -> io::Result<crate::release_http::Response> {
        self.calls.push(Call {
            url: url.into(),
            accept: accept.into(),
            maximum,
        });
        self.responses
            .get(url)
            .cloned()
            .map(|body| crate::release_http::Response {
                body,
                link: self.links.get(url).cloned(),
            })
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "fixture response missing"))
    }
}

fn endpoint() -> String {
    format!("https://api.github.com/repos/{OFFICIAL_REPOSITORY}/releases")
}

fn refs_url(product: Product) -> String {
    format!(
        "https://api.github.com/repos/{OFFICIAL_REPOSITORY}/git/matching-refs/tags/{}?per_page=100&page=1",
        product.tag_prefix()
    )
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
    fixture.releases(&[cli, office.clone()]);
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
        vec![
            refs_url(Product::Office),
            format!("{}/tags/tmt-office-v0.1.0-alpha.1", endpoint()),
            asset_url(4011),
            asset_url(4012)
        ]
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
    fixture.releases(&[cli]);
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
    fixture.releases(&[stable_old, stable_new.clone(), alpha.clone()]);
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
        vec![
            refs_url(Product::Cli),
            exact_url("1.10.0"),
            asset_url(1021),
            asset_url(1022)
        ]
    );
    assert!(
        fixture.calls[2..]
            .iter()
            .all(|call| call.accept == "application/octet-stream")
    );
    assert_eq!(fixture.calls[2].maximum, artifact::MANIFEST_LIMIT);

    fixture.calls.clear();
    let result = call_download(&mut fixture, Channel::Alpha, None, TARGET).unwrap();
    assert_eq!(result.version.to_string(), "1.11.0-alpha.1");
    assert_eq!(fixture.calls[2].url, asset_url(1031));
    assert_eq!(fixture.calls[3].url, asset_url(1032));
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
    fixture.releases(&[]);
    let error = call_download(&mut fixture, Channel::Stable, None, TARGET)
        .err()
        .expect("empty discovery should fail");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(fixture.calls.len(), 1);
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

fn metadata(product: Product, version: &str, draft: bool) -> Value {
    json!({"tag_name": format!("{}{version}", product.tag_prefix()),
        "draft": draft, "immutable": true, "prerelease": version.contains('-')})
}

fn discover(fixture: &mut HttpFixture, product: Product, channel: Channel) -> io::Result<Version> {
    super::discover_latest(product, channel, deadline(), &mut |u, a, l, d| {
        fixture.get(u, a, l, d)
    })
    .map(|(_, version)| version)
}

#[test]
fn more_than_1000_product_refs_keep_the_common_metadata_path_at_two_requests() {
    for product in Product::ALL {
        for channel in Channel::ALL {
            let selected = if channel == Channel::Stable {
                "2.0.0"
            } else {
                "2.0.0-alpha.10"
            };
            let mut history = (0..1250)
                .map(|n| metadata(product, &format!("1.0.{n}"), false))
                .collect::<Vec<_>>();
            history.push(metadata(product, "2.0.0-alpha.9", false));
            history.push(metadata(product, "99.0.0-beta.1", false));
            history.push(metadata(product, "100.0.0-rc.1", false));
            // The winning ref is last in the response, past the old release cap.
            history.push(metadata(product, selected, false));
            let mut fixture = HttpFixture::default();
            fixture.releases(&history);
            assert_eq!(
                discover(&mut fixture, product, channel)
                    .unwrap()
                    .to_string(),
                selected
            );
            assert_eq!(
                fixture.calls.len(),
                2,
                "ordinary discovery must not spend extra API requests"
            );
            assert_eq!(fixture.calls[0].url, refs_url(product));
            assert_eq!(
                fixture.calls[1].url,
                format!("{}/tags/{}{selected}", endpoint(), product.tag_prefix())
            );
            assert!(
                fixture
                    .calls
                    .iter()
                    .all(|call| call.accept == "application/vnd.github+json")
            );
        }
    }
}

#[test]
fn interleaved_release_history_does_not_hide_a_product_or_channel_after_300_releases() {
    for product in Product::ALL {
        for near_top in [true, false] {
            let mut history = (0..600)
                .map(|n| {
                    let other = Product::ALL[(n % 3) as usize];
                    metadata(other, &format!("1.0.{n}-alpha.1"), false)
                })
                .collect::<Vec<_>>();
            let selected = metadata(product, "2.0.0", false);
            history.insert(if near_top { 0 } else { 450 }, selected);
            // No tag exists yet for the newer draft, so it is not a published candidate.
            let mut fixture = HttpFixture::default();
            fixture.releases(&history);
            // This draft appears only in release history, not in matching refs.
            history.push(metadata(product, "99.0.0", true));
            for (page, entries) in history.chunks(100).enumerate() {
                fixture.response(
                    format!("{}?per_page=100&page={}", endpoint(), page + 1),
                    serde_json::to_vec(entries).unwrap(),
                );
            }
            assert_eq!(
                discover(&mut fixture, product, Channel::Stable)
                    .unwrap()
                    .to_string(),
                "2.0.0"
            );
            assert_eq!(fixture.calls.len(), 2);
        }
    }
}

#[test]
fn only_drafts_and_confirmed_missing_releases_allow_an_extra_tag_lookup() {
    let mut fixture = HttpFixture::default();
    fixture.releases(&[
        metadata(Product::Cli, "2.0.0-alpha.10", false),
        metadata(Product::Cli, "2.0.0-alpha.11", true),
        metadata(Product::Cli, "2.0.0-alpha.12", false),
    ]);
    fixture.responses.remove(&exact_url("2.0.0-alpha.12"));
    assert_eq!(
        discover(&mut fixture, Product::Cli, Channel::Alpha)
            .unwrap()
            .to_string(),
        "2.0.0-alpha.10"
    );
    assert_eq!(fixture.calls.len(), 4);
    assert_eq!(fixture.calls[1].url, exact_url("2.0.0-alpha.12"));
    assert_eq!(fixture.calls[2].url, exact_url("2.0.0-alpha.11"));
    assert_eq!(fixture.calls[3].url, exact_url("2.0.0-alpha.10"));
}

#[test]
fn complete_ref_pagination_precedes_semver_selection() {
    let product = Product::Cli;
    let mut fixture = HttpFixture::default();
    let history = (0..1250)
        .map(|n| metadata(product, &format!("1.0.{n}"), false))
        .collect::<Vec<_>>();
    fixture.releases(&history);
    let url = refs_url(product);
    let second = url.replace("&page=1", "&page=2");
    let refs = history
        .iter()
        .map(|r| json!({"ref": format!("refs/tags/{}", r["tag_name"].as_str().unwrap())}))
        .collect::<Vec<_>>();
    fixture.response(url.clone(), serde_json::to_vec(&refs[..625]).unwrap());
    fixture.links.insert(
        url.clone(),
        format!("<{second}>; rel=\"next\", <{second}>; rel=\"last\""),
    );
    fixture.response(second.clone(), serde_json::to_vec(&refs[625..]).unwrap());
    fixture
        .links
        .insert(second.clone(), format!("<{url}>; rel=\"prev\""));
    assert_eq!(
        discover(&mut fixture, product, Channel::Stable)
            .unwrap()
            .to_string(),
        "1.0.1249"
    );
    assert_eq!(
        fixture
            .calls
            .iter()
            .map(|c| c.url.clone())
            .collect::<Vec<_>>(),
        vec![url, second, exact_url("1.0.1249")]
    );
    assert!(
        fixture.calls[1].maximum < super::METADATA_LIMIT,
        "all ref pages share one byte budget"
    );
}

#[test]
fn incomplete_ref_pagination_never_selects_even_a_page_one_candidate() {
    let mut fixture = HttpFixture::default();
    for page in 1..=super::MAX_REF_PAGES {
        let url = refs_url(Product::Cli).replace("&page=1", &format!("&page={page}"));
        let next = refs_url(Product::Cli).replace("&page=1", &format!("&page={}", page + 1));
        fixture.response(
            url.clone(),
            serde_json::to_vec(&json!([{"ref": format!("refs/tags/v1.0.{page}")}])).unwrap(),
        );
        fixture.links.insert(url, format!("<{next}>; rel=\"next\""));
    }
    let error = discover(&mut fixture, Product::Cli, Channel::Stable).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        error.to_string(),
        "Release discovery exceeds its bound; select an exact version with --to."
    );
    assert_eq!(fixture.calls.len(), super::MAX_REF_PAGES);
    assert!(
        fixture
            .calls
            .iter()
            .all(|c| c.url.contains("matching-refs"))
    );
}

#[test]
fn exhausted_tag_lookup_scan_preserves_the_existing_error_and_request_cap() {
    let mut fixture = HttpFixture::default();
    let history = (0..=super::MAX_RELEASE_LOOKUPS)
        .map(|n| metadata(Product::Cli, &format!("1.0.{n}"), n != 0))
        .collect::<Vec<_>>();
    fixture.releases(&history);
    let error = discover(&mut fixture, Product::Cli, Channel::Stable).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        error.to_string(),
        "Release discovery exceeds its bound; select an exact version with --to."
    );
    assert_eq!(fixture.calls.len(), 1 + super::MAX_RELEASE_LOOKUPS);
    assert!(!fixture.calls.iter().any(|c| c.url.contains("/assets/")));
    // Removing one skipped draft makes the last permitted lookup a valid control.
    fixture.releases(&history[..super::MAX_RELEASE_LOOKUPS]);
    fixture.calls.clear();
    assert_eq!(
        discover(&mut fixture, Product::Cli, Channel::Stable)
            .unwrap()
            .to_string(),
        "1.0.0"
    );
    assert_eq!(fixture.calls.len(), 1 + super::MAX_RELEASE_LOOKUPS);
}

#[test]
fn ref_and_pagination_uncertainty_fails_before_release_or_asset_requests() {
    let url = refs_url(Product::Cli);
    for response in [
        b"{".to_vec(),
        b"{}".to_vec(),
        serde_json::to_vec(&json!([{}])).unwrap(),
        serde_json::to_vec(&json!([{"ref":"refs/tags/tmt-squad-v1.0.0"}])).unwrap(),
        serde_json::to_vec(&json!([{"ref":"refs/tags/v1.0.0"},{"ref":"refs/tags/v1.0.0"}]))
            .unwrap(),
        vec![b' '; super::METADATA_LIMIT + 1],
    ] {
        let mut fixture = HttpFixture::default();
        fixture.response(url.clone(), response);
        assert!(discover(&mut fixture, Product::Cli, Channel::Stable).is_err());
        assert_eq!(fixture.calls.len(), 1);
    }
    for link in [
        "broken".to_owned(),
        format!("<{}>; rel=\"next", url.replace("&page=1", "&page=2")),
        format!("<{url}>; rel=\"next\""),
        "<https://evil.example/refs?page=2>; rel=\"next\"".to_owned(),
        format!(
            "<{}>; rel=\"next\", <{}>; rel=\"next\"",
            url.replace("&page=1", "&page=2"),
            url.replace("&page=1", "&page=2")
        ),
    ] {
        let mut fixture = HttpFixture::default();
        fixture.releases(&[metadata(Product::Cli, "1.0.0", false)]);
        fixture.links.insert(url.clone(), link);
        assert!(discover(&mut fixture, Product::Cli, Channel::Stable).is_err());
        assert_eq!(fixture.calls.len(), 1);
    }
}

#[test]
fn equal_precedence_published_tags_are_ambiguous_but_draft_tags_are_not() {
    let mut fixture = HttpFixture::default();
    fixture.releases(&[
        metadata(Product::Cli, "1.0.0+one", false),
        metadata(Product::Cli, "1.0.0+two", false),
    ]);
    let error = discover(&mut fixture, Product::Cli, Channel::Stable).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Build metadata alone cannot select a different installed release."
    );
    assert_eq!(fixture.calls.len(), 3);
    fixture.calls.clear();
    fixture.releases(&[
        metadata(Product::Cli, "1.0.0+one", false),
        metadata(Product::Cli, "1.0.0+two", true),
    ]);
    assert_eq!(
        discover(&mut fixture, Product::Cli, Channel::Stable)
            .unwrap()
            .to_string(),
        "1.0.0+one"
    );
}

#[test]
fn an_uncertain_tag_lookup_never_falls_back_to_an_older_release() {
    let mut fixture = HttpFixture::default();
    fixture.releases(&[
        metadata(Product::Cli, "1.0.0", false),
        metadata(Product::Cli, "2.0.0", false),
    ]);
    let mut calls = 0;
    let error = super::discover_latest(
        Product::Cli,
        Channel::Stable,
        deadline(),
        &mut |u, a, l, d| {
            calls += 1;
            if calls == 2 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "rate limited",
                ));
            }
            fixture.get(u, a, l, d)
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(calls, 2);
    for field in ["tag_name", "draft"] {
        fixture.calls.clear();
        let mut newer = metadata(Product::Cli, "2.0.0", false);
        newer[field] = json!(null);
        fixture.response(exact_url("2.0.0"), serde_json::to_vec(&newer).unwrap());
        assert!(discover(&mut fixture, Product::Cli, Channel::Stable).is_err());
        assert_eq!(fixture.calls.len(), 2);
    }
}

#[test]
fn ref_pages_share_byte_and_deadline_budgets_without_partial_selection() {
    let mut fixture = HttpFixture::default();
    let first = refs_url(Product::Cli);
    let second = first.replace("&page=1", "&page=2");
    let mut body = serde_json::to_vec(&json!([{"ref":"refs/tags/v9.0.0"}])).unwrap();
    body.resize(super::METADATA_LIMIT - 2, b' ');
    fixture.response(first.clone(), body);
    fixture
        .links
        .insert(first.clone(), format!("<{second}>; rel=\"next\""));
    fixture.response(second.clone(), b"[] ".to_vec());
    let error = discover(&mut fixture, Product::Cli, Channel::Stable).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(fixture.calls.len(), 2);
    assert_eq!(fixture.calls[1].maximum, 2);
    assert!(
        fixture
            .calls
            .iter()
            .all(|c| c.url.contains("matching-refs"))
    );

    fixture.calls.clear();
    fixture.response(
        first,
        serde_json::to_vec(&json!([{"ref":"refs/tags/v9.0.0"}])).unwrap(),
    );
    let expected_deadline = deadline();
    let error = super::discover_latest(
        Product::Cli,
        Channel::Stable,
        expected_deadline,
        &mut |u, a, l, d| {
            assert_eq!(d, expected_deadline);
            if u == second {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "shared deadline exhausted",
                ));
            }
            fixture.get(u, a, l, d)
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(fixture.calls.len(), 1);
}
