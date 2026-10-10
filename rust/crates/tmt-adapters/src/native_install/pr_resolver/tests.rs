use super::*;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::{Cursor, Read, Write},
    time::Duration,
};

const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TOOLING: &str = "cccccccccccccccccccccccccccccccccccccccc";
const WORKFLOW: &[u8] = b"reviewed fixture workflow\n";
const POLICY: [wire::ApprovedProducer; 1] = [wire::ApprovedProducer {
    workflow_id: 701,
    workflow_sha256: "049c31e743140c5ad67e384b341336b1a35c8b8e520e09cfbe83f6b498bf8d58",
    tooling_sha: TOOLING,
}];
pub(in crate::native_install) const TARGET: &str = "aarch64-apple-darwin";
const NOW: u64 = 1_791_291_600_010;
const PR: &str = "234";

fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, contents) in files {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(contents).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// The existing native fixture producer supplies the inventory. Only its root
/// and manifest name are changed to the actual cargo-dist workflow spelling.
fn native(target: &str, schema: &Value) -> (Value, Vec<u8>, Vec<u8>, String) {
    let (mut release, old_manifest, archive, old_name) =
        release::product_fixture(Product::Cli, "5.0.0-alpha.92", target, 42);
    let name = format!("tmt-cli-{target}.tar.gz");
    let mut reader = tar::Archive::new(GzDecoder::new(Cursor::new(archive)));
    let mut writer = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
    for entry in reader.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().into_owned();
        let relative = path
            .strip_prefix(old_name.trim_end_matches(".tar.gz"))
            .unwrap();
        let mut header = entry.header().clone();
        header
            .set_path(std::path::Path::new(name.trim_end_matches(".tar.gz")).join(relative))
            .unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        header.set_cksum();
        writer.append(&header, bytes.as_slice()).unwrap();
    }
    let archive = writer.into_inner().unwrap().finish().unwrap();
    let mut manifest: Value = serde_json::from_slice(&old_manifest).unwrap();
    let mut metadata = manifest["artifacts"]
        .as_object_mut()
        .unwrap()
        .remove(&old_name)
        .unwrap();
    metadata["name"] = name.clone().into();
    metadata["checksums"]["sha256"] = artifact::digest(&archive).into();
    manifest["artifacts"][&name] = metadata;
    manifest["releases"][0]["artifacts"] = json!([name]);
    manifest["tmt_application_schema"] = schema.clone();
    let manifest = serde_json::to_vec(&manifest).unwrap();
    release["assets"][0]["size"] = manifest.len().into();
    release["assets"][0]["digest"] = format!("sha256:{}", artifact::digest(&manifest)).into();
    release["assets"][1]["name"] = name.clone().into();
    release["assets"][1]["size"] = archive.len().into();
    release["assets"][1]["digest"] = format!("sha256:{}", artifact::digest(&archive)).into();
    (release, manifest, archive, name)
}

struct Fixture {
    routes: BTreeMap<String, Vec<u8>>,
    fresh: BTreeMap<String, Vec<u8>>,
    calls: Vec<String>,
    links: BTreeMap<String, String>,
    local: u32,
    opt_in: bool,
}
impl Fixture {
    fn put(&mut self, route: &str, value: Value) {
        self.routes.insert(
            format!("{ROOT}/{route}"),
            serde_json::to_vec(&value).unwrap(),
        );
    }
    fn modify(&mut self, route: &str, fresh: bool, change: impl FnOnce(&mut Value)) {
        let url = format!("{ROOT}/{route}");
        let mut value = serde_json::from_slice(&self.routes[&url]).unwrap();
        change(&mut value);
        let destination = if fresh {
            &mut self.fresh
        } else {
            &mut self.routes
        };
        destination.insert(url, serde_json::to_vec(&value).unwrap());
    }
    fn new() -> Self {
        let mut fixture = Self {
            routes: BTreeMap::new(),
            fresh: BTreeMap::new(),
            calls: Vec::new(),
            links: BTreeMap::new(),
            local: 48,
            opt_in: false,
        };
        let mut catalog: Value =
            serde_json::from_slice(include_bytes!("../fixtures/pr-rc-catalog-v2.json")).unwrap();
        catalog["producer"]["workflow_sha256"] = artifact::digest(WORKFLOW).into();
        let schema = super::super::compiled_application_schema(HEAD).unwrap();
        let membership = json!({"id":9001,"repository_id":7001,"head_repository_id":7001,"head_branch":"main","head_sha":TOOLING});
        for candidate in catalog["candidates"].as_array_mut().unwrap() {
            let target = candidate["target"].as_str().unwrap().to_owned();
            candidate["application_schema"] = schema.clone();
            let (release, manifest, archive, name) = native(&target, &schema);
            let payload = zip(&[("dist-manifest.json", &manifest), (&name, &archive)]);
            candidate["dist_manifest"]["bytes"] = manifest.len().into();
            candidate["dist_manifest"]["sha256"] = artifact::digest(&manifest).into();
            candidate["archive"]["bytes"] = archive.len().into();
            candidate["archive"]["sha256"] = artifact::digest(&archive).into();
            candidate["payload_artifact"]["zip_bytes"] = payload.len().into();
            candidate["payload_artifact"]["zip_sha256"] = artifact::digest(&payload).into();
            let payload_id = candidate["payload_artifact"]["id"].as_u64().unwrap();
            fixture.put(&format!("actions/artifacts/{payload_id}"), json!({
                "id":payload_id,"name":candidate["payload_artifact"]["name"],"expired":false,
                "size_in_bytes":payload.len(),"digest":format!("sha256:{}",artifact::digest(&payload)),"workflow_run":membership,
            }));
            fixture.routes.insert(
                format!("{ROOT}/actions/artifacts/{payload_id}/zip"),
                payload,
            );
            if target == TARGET {
                fixture
                    .routes
                    .extend(release::indexed_routes(&release, &manifest, &archive));
            }
        }
        let catalog_zip = zip(&[("catalog.json", &serde_json::to_vec(&catalog).unwrap())]);
        let metadata = json!({"id":8201,"name":"tmt-pr-rc-catalog-v2-pr234","expired":false,
            "size_in_bytes":catalog_zip.len(),"digest":format!("sha256:{}",artifact::digest(&catalog_zip)),"workflow_run":membership});
        fixture.put("actions/artifacts/8201", metadata.clone());
        fixture.put(
            "actions/artifacts?name=tmt-pr-rc-catalog-v2-pr234&per_page=100&page=1",
            json!({"total_count":1,"artifacts":[metadata]}),
        );
        fixture
            .routes
            .insert(format!("{ROOT}/actions/artifacts/8201/zip"), catalog_zip);
        let api: Value =
            serde_json::from_slice(include_bytes!("../fixtures/pr-rc-api-v2.json")).unwrap();
        fixture.put("pulls/234", api["pull"].clone());
        fixture.put(
            "issues/234/timeline?per_page=100&page=1",
            api["eligibility_timeline"]["events"].clone(),
        );
        fixture.put("actions/runs/9001", api["producer_run"].clone());
        fixture.put(&format!("contents/.github/workflows/pr-rc.yml?ref={TOOLING}"), json!({"type":"file","encoding":"base64","content":base64::engine::general_purpose::STANDARD.encode(WORKFLOW)}));
        fixture
    }
    fn download(&mut self) -> io::Result<DownloadedRelease> {
        let opt_in = self.opt_in;
        let local = self.local;
        download(
            Request {
                product: Product::Cli,
                pr: PrNumber::parse(PR).unwrap(),
                exact: None,
                target: TARGET,
                opt_in,
                deadline: Instant::now() + Duration::from_secs(60),
            },
            Policy {
                repository_id: 7001,
                producers: &POLICY,
            },
            |url, _, maximum, _| {
                let fresh = self.calls.iter().any(|called| called == url);
                self.calls.push(url.into());
                let body = if fresh {
                    self.fresh.get(url).or_else(|| self.routes.get(url))
                } else {
                    self.routes.get(url)
                }
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing fixture response"))?
                .clone();
                assert!(body.len() <= maximum);
                Ok(Response {
                    body,
                    link: self.links.get(url).cloned(),
                })
            },
            || {
                Ok(vec![wire::DatabaseSchema {
                    domain: "tmt-core-db".into(),
                    version: local,
                }])
            },
            || Ok(NOW),
        )
    }
}

pub(in crate::native_install) fn downloaded() -> DownloadedRelease {
    Fixture::new().download().unwrap()
}

#[test]
fn current_source_acquires_exact_native_bytes_and_records_one_fresh_admission() {
    let mut fixture = Fixture::new();
    let downloaded = fixture.download().unwrap();
    let artifact = artifact::acquire_bytes(
        Product::Cli,
        &downloaded.manifest,
        &downloaded.archive_name,
        &downloaded.archive,
        TARGET,
    )
    .unwrap();
    assert_eq!(artifact.files["tmt"], b"native executable\n");
    let Provenance::Pr(proof) = &downloaded.provenance else {
        panic!("PR proof missing");
    };
    assert_eq!(proof.head_sha, HEAD);
    assert_eq!(proof.producer.tooling_sha, TOOLING);
    assert_eq!(proof.eligibility.enabled_event_id, 6201);
    assert_eq!(proof.admission.local_schemas[0].version, 48);
    for route in [
        "pulls/234",
        "issues/234/timeline?per_page=100&page=1",
        "actions/runs/9001",
        "actions/artifacts/8201",
        "actions/artifacts/8101",
    ] {
        assert_eq!(
            fixture
                .calls
                .iter()
                .filter(|url| **url == format!("{ROOT}/{route}"))
                .count(),
            2,
            "{route}"
        );
    }
    assert!(
        !fixture
            .calls
            .iter()
            .any(|url| url.contains("/dispatches") || url.contains("releases/latest"))
    );
    assert!(fixture.calls.len() < 32);
}

#[test]
fn close_head_change_disable_and_reenable_before_admission_refuse_without_fallback() {
    for (route, pointer, value) in [
        ("pulls/234", "/state", json!("closed")),
        ("pulls/234", "/head/sha", json!("d".repeat(40))),
        ("pulls/234", "/head/repo/id", json!(7002)),
        ("pulls/234", "/labels", json!([])),
        ("pulls/234", "/labels/0/id", json!(6102)),
        (
            "issues/234/timeline?per_page=100&page=1",
            "/0/event",
            json!("unlabeled"),
        ),
        (
            "issues/234/timeline?per_page=100&page=1",
            "/0/id",
            json!(6202),
        ),
        (
            "issues/234/timeline?per_page=100&page=1",
            "/0/created_at",
            json!("2026-10-06T15:00:01Z"),
        ),
        ("actions/runs/9001", "/run_attempt", json!(2)),
        ("actions/artifacts/8101", "/expired", json!(true)),
    ] {
        let mut fixture = Fixture::new();
        fixture.modify(route, true, |document| {
            *document.pointer_mut(pointer).unwrap() = value
        });
        assert!(fixture.download().is_err(), "{route}{pointer}");
        assert_eq!(
            fixture
                .calls
                .iter()
                .filter(|url| **url == format!("{ROOT}/{route}"))
                .count(),
            2
        );
    }
}

#[test]
fn timeline_refuses_absent_duplicate_tied_noncanonical_and_incomplete_evidence() {
    for change in [0, 1, 2, 3, 4, 5] {
        let mut fixture = Fixture::new();
        fixture.modify(
            "issues/234/timeline?per_page=100&page=1",
            false,
            |timeline| {
                let events = timeline.as_array_mut().unwrap();
                match change {
                    0 => events.clear(),
                    1 => events.push(events[0].clone()),
                    2 => {
                        let mut tied = events[0].clone();
                        tied["id"] = 6202.into();
                        tied["event"] = "unlabeled".into();
                        events.push(tied);
                    }
                    3 => events[0]["created_at"] = "2026-10-06T15:00:00+00:00".into(),
                    4 => events[0]["created_at"] = "2026-02-30T00:00:00Z".into(),
                    _ => events.resize(101, json!({"event":"commented"})),
                }
            },
        );
        assert!(fixture.download().is_err(), "control {change}");
        assert_eq!(fixture.calls.len(), 2);
    }
    let mut fixture = Fixture::new();
    fixture.links.insert(
        format!("{ROOT}/issues/234/timeline?per_page=100&page=1"),
        "next page".into(),
    );
    assert!(fixture.download().is_err());
}

#[test]
fn unknown_publisher_and_changed_workflow_never_search_old_success() {
    for (route, pointer, value) in [
        ("actions/runs/9001", "/head_sha", json!("d".repeat(40))),
        ("actions/runs/9001", "/workflow_id", json!(702)),
        ("actions/runs/9001", "/conclusion", json!("failure")),
        ("actions/runs/9001", "/head_repository/id", json!(7002)),
        ("actions/artifacts/8201", "/expired", json!(true)),
    ] {
        let mut fixture = Fixture::new();
        fixture.modify(route, false, |v| *v.pointer_mut(pointer).unwrap() = value);
        assert!(fixture.download().is_err(), "{route}{pointer}");
        assert!(
            !fixture
                .calls
                .iter()
                .any(|url| url.contains("actions/runs/9000"))
        );
    }
    let mut fixture = Fixture::new();
    fixture.modify(
        &format!("contents/.github/workflows/pr-rc.yml?ref={TOOLING}"),
        false,
        |v| {
            v["content"] = base64::engine::general_purpose::STANDARD
                .encode(b"unreviewed")
                .into()
        },
    );
    assert!(fixture.download().is_err());
    assert!(APPROVED.is_empty());
}

#[test]
fn local_newer_than_candidate_and_unknown_latest_alpha_refuse() {
    let mut fixture = Fixture::new();
    fixture.local = crate::storage::Storage::compiled_schema().version + 1;
    let error = fixture.download().err().unwrap();
    assert!(
        error
            .get_ref()
            .unwrap()
            .is::<tmt_core::native_install::SchemaError>()
    );
    let mut fixture = Fixture::new();
    let manifest_url =
        "https://github.com/pj-tmt/tmt/releases/download/v5.0.0-alpha.92/dist-manifest.json";
    let record_url =
        "https://github.com/pj-tmt/tmt/releases/download/v5.0.0-alpha.92/tmt-release-record.json";
    let pointer_url =
        "https://raw.githubusercontent.com/pj-tmt/tmt/release-index/channels/cli/alpha.json";
    let mut alpha: Value = serde_json::from_slice(&fixture.routes[manifest_url]).unwrap();
    alpha
        .as_object_mut()
        .unwrap()
        .remove("tmt_application_schema");
    let alpha = serde_json::to_vec(&alpha).unwrap();
    let mut record: Value = serde_json::from_slice(&fixture.routes[record_url]).unwrap();
    record["manifest"]["size"] = alpha.len().into();
    record["manifest"]["sha256"] = artifact::digest(&alpha).into();
    let record = serde_json::to_vec(&record).unwrap();
    let mut pointer: Value = serde_json::from_slice(&fixture.routes[pointer_url]).unwrap();
    pointer["record"]["size"] = record.len().into();
    pointer["record"]["sha256"] = artifact::digest(&record).into();
    fixture.routes.insert(manifest_url.into(), alpha);
    fixture.routes.insert(record_url.into(), record);
    fixture
        .routes
        .insert(pointer_url.into(), serde_json::to_vec(&pointer).unwrap());
    assert!(fixture.download().is_err());
}

#[test]
fn metadata_budget_refuses_before_the_next_request_and_bounds_total_bytes() {
    let mut calls = 0;
    {
        let mut api = Api {
            get: |_: &str, _: &str, _: usize, _: Instant| {
                calls += 1;
                Ok(b"{}".to_vec().into())
            },
            deadline: Instant::now() + Duration::from_secs(60),
            requests: 0,
            remaining: METADATA_LIMIT,
        };
        for _ in 0..32 {
            api.metadata("pulls/234").unwrap();
        }
        assert!(
            api.metadata("pulls/234")
                .unwrap_err()
                .to_string()
                .contains("bound")
        );
    }
    assert_eq!(calls, 32);
    let mut calls = 0;
    {
        let mut api = Api {
            get: |_: &str, _: &str, limit: usize, _: Instant| {
                calls += 1;
                assert_eq!(
                    limit,
                    if calls == 1 {
                        METADATA_LIMIT
                    } else {
                        METADATA_LIMIT / 2
                    }
                );
                let mut bytes = b"{}".to_vec();
                bytes.resize(METADATA_LIMIT / 2, b' ');
                Ok(bytes.into())
            },
            deadline: Instant::now() + Duration::from_secs(60),
            requests: 0,
            remaining: METADATA_LIMIT,
        };
        api.metadata("pulls/234").unwrap();
        api.metadata("pulls/234").unwrap();
        assert!(api.metadata("pulls/234").is_err());
    }
    assert_eq!(calls, 2);
}

#[test]
fn failed_or_expired_newest_trusted_generation_never_downloads_an_older_catalog() {
    let route = "actions/artifacts?name=tmt-pr-rc-catalog-v2-pr234&per_page=100&page=1";
    for failure in ["run", "expired", "head"] {
        let mut fixture = Fixture::new();
        fixture.modify(route, false, |inventory| {
            let mut older = inventory["artifacts"][0].clone();
            older["id"] = 8200.into();
            older["workflow_run"]["id"] = 9000.into();
            inventory["artifacts"].as_array_mut().unwrap().push(older);
            inventory["total_count"] = 2.into();
        });
        match failure {
            "run" => fixture.modify("actions/runs/9001", false, |run| {
                run["conclusion"] = "failure".into()
            }),
            "expired" => fixture.modify("actions/artifacts/8201", false, |entry| {
                entry["expired"] = true.into()
            }),
            _ => fixture.modify("pulls/234", false, |pull| {
                pull["head"]["sha"] = "b".repeat(40).into()
            }),
        }
        assert!(fixture.download().is_err(), "{failure}");
        assert!(
            !fixture
                .calls
                .iter()
                .any(|url| url.ends_with("/actions/runs/9000")
                    || url.contains("/actions/artifacts/8200"))
        );
    }
}

#[test]
fn excessive_generation_discovery_refuses_before_any_producer_or_payload_fetch() {
    let mut fixture = Fixture::new();
    fixture.modify(
        "actions/artifacts?name=tmt-pr-rc-catalog-v2-pr234&per_page=100&page=1",
        false,
        |inventory| {
            let prototype = inventory["artifacts"][0].clone();
            inventory["artifacts"] = (0..9)
                .map(|index| {
                    let mut entry = prototype.clone();
                    entry["id"] = (8201 + index).into();
                    entry["workflow_run"]["id"] = (9001 + index).into();
                    entry
                })
                .collect::<Vec<_>>()
                .into();
            inventory["total_count"] = 9.into();
        },
    );
    assert!(
        fixture
            .download()
            .err()
            .unwrap()
            .to_string()
            .contains("run inventory exceeds its bound")
    );
    assert_eq!(fixture.calls.len(), 3);
}
