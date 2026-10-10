use super::artifact;
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
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(5)
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
    _version: &str,
    target: &str,
    companions: &[(&str, &[u8])],
) -> (String, Vec<u8>) {
    let name = format!("{}-{target}.tar.gz", product.package());
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

/// Normal-release fake network: only the fixed raw pointer and exact download URLs exist.
pub(in crate::native_install) fn indexed_routes(
    release: &Value,
    manifest: &[u8],
    archive: &[u8],
) -> BTreeMap<String, Vec<u8>> {
    let tag = release["tag_name"].as_str().unwrap();
    let product = Product::ALL
        .into_iter()
        .find(|product| tag.starts_with(product.tag_prefix()))
        .unwrap();
    let version = tag.strip_prefix(product.tag_prefix()).unwrap();
    let channel = if version.contains('-') {
        "alpha"
    } else {
        "stable"
    };
    let base = format!("{}{tag}/", super::RELEASE_BASE);
    let descriptor = |name: &str, bytes: &[u8]| json!({"name":name,"url":format!("{base}{name}"),"size":bytes.len(),"sha256":sha256(bytes)});
    let mut archives = serde_json::Map::new();
    for target in super::TARGETS {
        let name = format!("{}-{target}.tar.gz", product.package());
        archives.insert(target.into(), descriptor(&name, archive));
    }
    let record = json!({"schemaVersion":1,"product":product.as_str(),"version":version,"tag":tag,
        "releaseId":release["id"],"sourceSha":"a".repeat(40),"manifest":descriptor(MANIFEST_NAME, manifest),"archives":archives});
    let record = serde_json::to_vec(&record).unwrap();
    let record_url = format!("{base}{}", super::RECORD_NAME);
    let pointer = json!({"schemaVersion":1,"product":product.as_str(),"version":version,"tag":tag,"channel":channel,
        "record":{"url":record_url,"size":record.len(),"sha256":sha256(&record)}});
    let mut routes = BTreeMap::from([
        (
            format!(
                "{}channels/{}/{channel}.json",
                super::INDEX_BASE,
                product.as_str()
            ),
            serde_json::to_vec(&pointer).unwrap(),
        ),
        (record_url, record),
        (format!("{base}{MANIFEST_NAME}"), manifest.to_vec()),
    ]);
    for asset in release["assets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|asset| asset["name"] != MANIFEST_NAME)
    {
        routes.insert(
            format!("{base}{}", asset["name"].as_str().unwrap()),
            archive.to_vec(),
        );
    }
    routes
}

pub(in crate::native_install) fn indexed_download(
    release: Value,
    manifest: Vec<u8>,
    archive: Vec<u8>,
) -> impl FnMut(&str, &str, usize, Instant) -> io::Result<crate::release_http::Response> {
    let routes = indexed_routes(&release, &manifest, &archive);
    move |url, _, maximum, deadline| {
        assert!(
            !url.contains("api.github.com"),
            "normal release must never reach the API: {url}"
        );
        assert!(deadline > Instant::now());
        let bytes = routes
            .get(url)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "fixture response missing"))?;
        assert!(bytes.len() <= maximum);
        Ok(bytes.into())
    }
}

struct Fixture {
    routes: BTreeMap<String, Vec<u8>>,
    calls: Vec<String>,
}
impl Fixture {
    fn new(product: Product, version: &str) -> Self {
        let (release, manifest, archive, _) = product_fixture(product, version, TARGET, 42);
        Self {
            routes: indexed_routes(&release, &manifest, &archive),
            calls: vec![],
        }
    }
    fn get(
        &mut self,
        url: &str,
        _: &str,
        _: usize,
        _: Instant,
    ) -> io::Result<crate::release_http::Response> {
        assert!(!url.contains("api.github.com"));
        self.calls.push(url.into());
        self.routes
            .get(url)
            .cloned()
            .map(Into::into)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "fixture response missing"))
    }
    fn change(&mut self, url: &str, change: impl FnOnce(&mut Value)) {
        let mut value = serde_json::from_slice(&self.routes[url]).unwrap();
        change(&mut value);
        self.routes
            .insert(url.into(), serde_json::to_vec(&value).unwrap());
    }
    fn run(
        &mut self,
        product: Product,
        channel: Channel,
        exact: Option<&str>,
    ) -> io::Result<super::DownloadedRelease> {
        let exact = exact.map(|text| text.parse().unwrap());
        super::download_product(
            product,
            channel,
            exact.as_ref(),
            TARGET,
            deadline(),
            |u, a, l, d| self.get(u, a, l, d),
        )
    }
    fn pointer(&self) -> String {
        self.routes
            .keys()
            .find(|url| url.contains("/channels/"))
            .unwrap()
            .clone()
    }
    fn record(&self) -> String {
        self.routes
            .keys()
            .find(|url| url.ends_with(super::RECORD_NAME))
            .unwrap()
            .clone()
    }
    fn rebind(&mut self) {
        let record = self.record();
        let bytes = self.routes[&record].clone();
        let pointer = self.pointer();
        self.change(&pointer, |value| {
            value["record"]["size"] = bytes.len().into();
            value["record"]["sha256"] = sha256(&bytes).into();
        });
    }
}

#[test]
fn each_product_and_channel_reads_one_pointer_without_api_fallback() {
    for product in Product::ALL {
        for version in ["1.2.4", "1.2.4-alpha.2"] {
            let channel = if version.contains('-') {
                Channel::Alpha
            } else {
                Channel::Stable
            };
            let mut fixture = Fixture::new(product, version);
            let result = fixture.run(product, channel, None).unwrap();
            assert_eq!(result.version.to_string(), version);
            assert_eq!(fixture.calls.len(), 4);
            assert_eq!(
                fixture
                    .calls
                    .iter()
                    .filter(|url| url.contains("/channels/"))
                    .count(),
                1
            );
            assert_eq!(
                result.archive_name,
                format!("{}-{TARGET}.tar.gz", product.package())
            );
            artifact::acquire_bytes(
                product,
                &result.manifest,
                &result.archive_name,
                &result.archive,
                TARGET,
            )
            .unwrap();
        }
    }
}

#[test]
fn exact_version_reads_its_record_without_a_pointer() {
    let mut fixture = Fixture::new(Product::Cli, "1.2.4");
    fixture
        .run(Product::Cli, Channel::Stable, Some("1.2.4"))
        .unwrap();
    assert_eq!(fixture.calls.len(), 3);
    assert!(fixture.calls.iter().all(|url| !url.contains("/channels/")));
    fixture.routes.remove(&fixture.record());
    let error = fixture
        .run(Product::Cli, Channel::Stable, Some("1.2.4"))
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "v1.2.4 cannot be installed directly. Omit --to to get the latest release. Nothing was changed."
    );
}

#[test]
fn missing_pointer_refuses_instead_of_reporting_current_or_trying_api() {
    let mut fixture = Fixture::new(Product::Cli, "1.2.4");
    fixture.routes.clear();
    let error = fixture
        .run(Product::Cli, Channel::Stable, None)
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "Could not check for cli updates on raw.githubusercontent.com: fixture response missing. Nothing was changed."
    );
    assert_eq!(fixture.calls.len(), 1);
}

#[test]
fn pointer_identity_and_locations_fail_closed() {
    for (field, value) in [
        ("schemaVersion", json!(2)),
        ("product", json!("colab")),
        ("channel", json!("alpha")),
        ("version", json!("1.2.4+build")),
        ("version", json!("1.2.4-beta.1")),
        ("tag", json!("v1.2.5")),
    ] {
        let mut fixture = Fixture::new(Product::Cli, "1.2.4");
        let url = fixture.pointer();
        fixture.change(&url, |v| v[field] = value);
        assert!(
            fixture.run(Product::Cli, Channel::Stable, None).is_err(),
            "{field}"
        );
        assert_eq!(fixture.calls.len(), 1);
    }
    for url in [
        "https://api.github.com/repos/pj-tmt/tmt/releases/42",
        "https://raw.githubusercontent.com/other/repo/main/record.json",
        "https://github.com/pj-tmt/tmt/releases/download/v1.2.5/tmt-release-record.json",
    ] {
        let mut fixture = Fixture::new(Product::Cli, "1.2.4");
        let pointer = fixture.pointer();
        fixture.change(&pointer, |v| v["record"]["url"] = url.into());
        assert!(fixture.run(Product::Cli, Channel::Stable, None).is_err());
        assert_eq!(fixture.calls.len(), 1);
    }
}

#[test]
fn pointer_and_record_bounds_duplicates_and_digests_are_checked_before_assets() {
    for defect in [
        "pointer oversize",
        "duplicate",
        "record oversize",
        "record digest",
        "record size",
    ] {
        let mut fixture = Fixture::new(Product::Cli, "1.2.4");
        let pointer = fixture.pointer();
        let record = fixture.record();
        match defect {
            "pointer oversize" => {
                fixture
                    .routes
                    .insert(pointer, vec![b' '; super::POINTER_LIMIT + 1]);
            }
            "duplicate" => {
                fixture.routes.insert(
                    pointer,
                    br#"{"schemaVersion":1,"schemaVersion":1}"#.to_vec(),
                );
            }
            "record oversize" => fixture.change(&pointer, |v| {
                v["record"]["size"] = (super::RECORD_LIMIT + 1).into()
            }),
            "record digest" => {
                fixture.routes.get_mut(&record).unwrap().push(b' ');
            }
            "record size" => fixture.change(&pointer, |v| v["record"]["size"] = 1.into()),
            _ => unreachable!(),
        }
        assert!(
            fixture.run(Product::Cli, Channel::Stable, None).is_err(),
            "{defect}"
        );
        assert!(fixture.calls.len() <= 2);
    }
}

#[test]
fn record_identity_target_set_and_asset_descriptors_fail_before_assets() {
    for defect in [
        "release id",
        "source",
        "product",
        "tag",
        "version",
        "schema",
        "target missing",
        "target extra",
        "name",
        "url",
        "size",
        "digest",
    ] {
        let mut fixture = Fixture::new(Product::Cli, "1.2.4");
        let record = fixture.record();
        fixture.change(&record, |v| match defect {
            "release id" => v["releaseId"] = 0.into(),
            "source" => v["sourceSha"] = "A".repeat(40).into(),
            "product" => v["product"] = "ops".into(),
            "tag" => v["tag"] = "v1.2.5".into(),
            "version" => v["version"] = "1.2.5".into(),
            "schema" => v["schemaVersion"] = 2.into(),
            "target missing" => {
                v["archives"].as_object_mut().unwrap().remove(TARGET);
            }
            "target extra" => v["archives"]["unknown"] = json!({}),
            "name" => v["manifest"]["name"] = "other.json".into(),
            "url" => v["manifest"]["url"] = "https://api.github.com/asset".into(),
            "size" => v["manifest"]["size"] = 0.into(),
            "digest" => v["manifest"]["sha256"] = "f".repeat(63).into(),
            _ => unreachable!(),
        });
        fixture.rebind();
        assert!(
            fixture.run(Product::Cli, Channel::Stable, None).is_err(),
            "{defect}"
        );
        assert_eq!(fixture.calls.len(), 2);
    }
}

#[test]
fn manifest_and_archive_bytes_remain_bound() {
    for suffix in [MANIFEST_NAME, "tmt-cli-aarch64-apple-darwin.tar.gz"] {
        let mut fixture = Fixture::new(Product::Cli, "1.2.4");
        let url = fixture
            .routes
            .keys()
            .find(|url| url.ends_with(suffix))
            .unwrap()
            .clone();
        fixture.routes.get_mut(&url).unwrap()[0] ^= 1;
        assert!(fixture.run(Product::Cli, Channel::Stable, None).is_err());
    }
}

#[test]
fn bootstrap_record_and_additive_fields_are_supported() {
    let mut fixture = Fixture::new(Product::Cli, "1.2.4");
    let record = fixture.record();
    let url = format!("{}records/v1.2.4.json", super::INDEX_BASE);
    let bytes = fixture.routes.remove(&record).unwrap();
    fixture.routes.insert(url.clone(), bytes);
    let pointer = fixture.pointer();
    fixture.change(&pointer, |v| {
        v["record"]["url"] = url.into();
        v["future"] = true.into();
    });
    fixture.run(Product::Cli, Channel::Stable, None).unwrap();
}

#[test]
fn manifest_identity_and_checksum_remain_authoritative_after_index_binding() {
    for defect in ["version", "target", "checksum"] {
        let mut fixture = Fixture::new(Product::Cli, "1.2.4");
        let url = fixture
            .routes
            .keys()
            .find(|url| url.ends_with(MANIFEST_NAME))
            .unwrap()
            .clone();
        fixture.change(&url, |v| match defect {
            "version" => v["releases"][0]["app_version"] = "1.2.5".into(),
            "target" => {
                v["artifacts"][format!("tmt-cli-{TARGET}.tar.gz")]["target_triples"] =
                    json!(["unsupported"])
            }
            "checksum" => {
                v["artifacts"][format!("tmt-cli-{TARGET}.tar.gz")]["checksums"]["sha256"] =
                    "0".repeat(64).into()
            }
            _ => unreachable!(),
        });
        let manifest = fixture.routes[&url].clone();
        let record = fixture.record();
        fixture.change(&record, |v| {
            v["manifest"]["size"] = manifest.len().into();
            v["manifest"]["sha256"] = sha256(&manifest).into();
        });
        fixture.rebind();
        let result = fixture.run(Product::Cli, Channel::Stable, None);
        if defect == "checksum" {
            let result = result.unwrap();
            assert!(
                artifact::acquire_bytes(
                    Product::Cli,
                    &result.manifest,
                    &result.archive_name,
                    &result.archive,
                    TARGET
                )
                .is_err()
            );
        } else {
            assert!(result.is_err(), "{defect}");
        }
    }
}

#[test]
fn canonical_versions_reject_non_alpha_and_unsafe_or_noncanonical_numbers() {
    for version in [
        "01.2.3",
        "1.2.3-alpha.01",
        "1.2.3-alpha.1.2",
        "1.2.3-beta.1",
        "1.2.3+metadata",
        "9007199254740992.0.0",
        "1.2.3-alpha.9007199254740992",
    ] {
        assert!(super::canonical_version(version).is_err(), "{version}");
    }
    for version in ["0.0.0", "1.2.3-alpha.0", "9007199254740991.0.0"] {
        assert_eq!(
            super::canonical_version(version).unwrap().to_string(),
            version
        );
    }
}
