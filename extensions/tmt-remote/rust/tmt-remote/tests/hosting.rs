//! Remote-authored static inventory fixtures; no Colab release or provider acceptance.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::Read;
use tmt_remote::hosting::{
    self, HostingBundle, HostingInventory, HostingLiveRelease, HostingManifest, HostingRefusal,
};
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn manifest() -> HostingManifest {
    HostingManifest::parse(&json!({"version":1,"files":[
        {"path":"/index.html","sha256":hash(b"hello"),"length":5,"contentType":"text/html"}
    ]}))
    .unwrap()
}
fn reply(manifest: &HostingManifest) -> Value {
    json!({"version":1,"manifestDigest":manifest.digest(),"files":[{"path":"/index.html","bytesBase64":STANDARD.encode(b"hello")}]})
}
fn bundle() -> HostingBundle {
    let m = manifest();
    HostingBundle::parse(&m, &serde_json::to_vec(&reply(&m)).unwrap()).unwrap()
}
#[test]
fn gzip_and_serving_config_are_frozen_and_public_shell_routes_are_bounded() {
    let first = hosting::compose(&[bundle()]).unwrap().unwrap();
    let second = hosting::compose(&[bundle()]).unwrap().unwrap();
    assert_eq!(first.view(), second.view());
    assert_eq!(first.gzip("/index.html"), second.gzip("/index.html"));
    let gzip = first.gzip("/index.html").unwrap();
    assert_eq!(&gzip[..10], &[31, 139, 8, 0, 0, 0, 0, 0, 0, 255]);
    // Python gzip.compress(b'hello', compresslevel=6, mtime=0), normalized OS=255.
    assert_eq!(
        gzip,
        &[
            31, 139, 8, 0, 0, 0, 0, 0, 0, 255, 203, 72, 205, 201, 201, 7, 0, 134, 166, 16, 54, 5,
            0, 0, 0
        ]
    );
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(gzip)
        .read_to_end(&mut raw)
        .unwrap();
    assert_eq!(raw, b"hello");
    assert_eq!(first.files()[0].raw_digest, hash(b"hello"));
    assert_eq!(first.files()[0].gzip_digest, hash(gzip));
    assert_eq!(
        first.config()["rewrites"],
        json!([
            {"regex":"^/colab/?$","path":"/index.html"},
            {"regex":"^/(p|read)/[A-Za-z0-9_-]{4,64}$","path":"/index.html"}
        ])
    );
    assert!(
        !serde_json::to_string(first.config())
            .unwrap()
            .contains("/__")
    );
    assert!(hosting::compose(&[]).unwrap().is_none());
}
#[test]
fn manifest_refuses_only_the_perturbed_guard_in_valid_controls() {
    let valid = serde_json::to_value(manifest()).unwrap();
    for path in [
        "index.html",
        "/../index.html",
        "/a//b",
        "/a%2fb",
        "/a?b",
        "/a#b",
        "/.env",
        "/a/./b",
        "/é",
    ] {
        let mut value = valid.clone();
        value["files"][0]["path"] = json!(path);
        assert_eq!(
            HostingManifest::parse(&value).unwrap_err(),
            HostingRefusal::Path,
            "{path}"
        );
    }
    for path in ["/__", "/__/firebase/init.json"] {
        let mut value = valid.clone();
        value["files"][0]["path"] = json!(path);
        assert_eq!(
            HostingManifest::parse(&value).unwrap_err(),
            HostingRefusal::Reserved
        );
    }
    for (key, value, reason) in [
        ("sha256", json!("a".repeat(63)), HostingRefusal::Digest),
        ("length", json!(4 * 1024 * 1024 + 1), HostingRefusal::Bounds),
        (
            "contentType",
            json!("TOKEN_CANARY"),
            HostingRefusal::ContentType,
        ),
    ] {
        let mut edited = valid.clone();
        edited["files"][0][key] = value;
        assert_eq!(HostingManifest::parse(&edited).unwrap_err(), reason);
    }
    let mut duplicate = valid.clone();
    duplicate["files"]
        .as_array_mut()
        .unwrap()
        .push(valid["files"][0].clone());
    assert_eq!(
        HostingManifest::parse(&duplicate).unwrap_err(),
        HostingRefusal::Collision
    );
    let mut extra = valid;
    extra["downloadUrl"] = json!("TOKEN_CANARY");
    assert_eq!(
        HostingManifest::parse(&extra).unwrap_err(),
        HostingRefusal::Shape
    );
    assert!(hosting::compose(&[bundle(), bundle()]).is_err());
}
#[test]
fn bundle_must_be_the_complete_exact_canonical_manifest_inventory() {
    let m = manifest();
    let valid = reply(&m);
    HostingBundle::parse(&m, &serde_json::to_vec(&valid).unwrap()).unwrap();
    for change in [0, 1, 2, 3, 4] {
        let mut value = valid.clone();
        match change {
            0 => value["manifestDigest"] = json!("0".repeat(64)),
            1 => value["files"][0]["path"] = json!("/other"),
            2 => value["files"][0]["bytesBase64"] = json!(STANDARD.encode(b"HELLO")),
            3 => value["files"] = json!([]),
            _ => value["files"][0]["extra"] = json!(true),
        }
        assert!(
            HostingBundle::parse(&m, &serde_json::to_vec(&value).unwrap()).is_err(),
            "{change}"
        );
    }
    assert!(HostingBundle::parse(&m, b"{\"version\":1,\"version\":1}").is_err());
}
const ID: &str = "11111111-1111-4111-8111-111111111111";
#[test]
fn site_creation_web_app_choice_and_foreign_fingerprint_are_named_in_the_plan() {
    let content = hosting::compose(&[bundle()]).unwrap().unwrap();
    let absent = HostingInventory {
        site_exists: false,
        web_apps: vec![],
        live: None,
    };
    let planned = hosting::deployment_view(&content, "demo-remote-1", ID, &absent, None).unwrap();
    assert!(planned.create_site && planned.create_web_app);
    assert_eq!(planned.public_url, "https://demo-remote-1.web.app");
    let mut inventory = HostingInventory {
        site_exists: true,
        web_apps: vec!["1:123:web:abc".into()],
        live: None,
    };
    assert_eq!(
        hosting::deployment_view(&content, "demo-remote-1", ID, &inventory, None)
            .unwrap()
            .web_app
            .as_deref(),
        Some("1:123:web:abc")
    );
    inventory.web_apps.push("1:123:web:def".into());
    assert!(matches!(
        hosting::deployment_view(&content, "demo-remote-1", ID, &inventory, None),
        Err(HostingRefusal::WebAppSelection)
    ));
    assert_eq!(
        hosting::deployment_view(
            &content,
            "demo-remote-1",
            ID,
            &inventory,
            Some("1:123:web:def")
        )
        .unwrap()
        .web_app
        .as_deref(),
        Some("1:123:web:def")
    );
    inventory.web_apps.truncate(1);
    inventory.live = Some(HostingLiveRelease {
        version: "version-1".into(),
        deployment_id: Some(ID.into()),
        plan_digest_prefix: Some("a".repeat(12)),
        content_digest: Some(content.digest().into()),
        files: content.files().to_vec(),
        config: content.config().clone(),
    });
    assert_eq!(
        hosting::deployment_view(&content, "demo-remote-1", ID, &inventory, None)
            .unwrap()
            .replaces,
        "none"
    );
    inventory.live.as_mut().unwrap().deployment_id = None;
    let foreign =
        hosting::deployment_view(&content, "demo-remote-1", ID, &inventory, None).unwrap();
    assert_eq!(foreign.replaces, "foreign");
    let fingerprint = foreign.replaced_fingerprint.unwrap();
    assert_eq!(fingerprint.len(), 64);
    inventory.live.as_mut().unwrap().config["cleanUrls"] = json!(true);
    assert_ne!(
        hosting::deployment_view(&content, "demo-remote-1", ID, &inventory, None)
            .unwrap()
            .replaced_fingerprint
            .unwrap(),
        fingerprint
    );
}

#[test]
fn full_file_and_byte_caps_sorted_paths_and_nonadjacent_nested_collisions_are_checked() {
    let file = |path: &str, length: u64| json!({"path":path,"sha256":hash(b"hello"),"length":length,"contentType":"text/html"});
    let cap = 4 * 1024 * 1024;
    let value =
        json!({"version":1,"files":[file("/a",cap),file("/b",cap),file("/c",cap),file("/d",cap)]});
    HostingManifest::parse(&value).unwrap();
    let mut too_large = value.clone();
    too_large["files"]
        .as_array_mut()
        .unwrap()
        .push(file("/e", 1));
    assert_eq!(
        HostingManifest::parse(&too_large).unwrap_err(),
        HostingRefusal::Bounds
    );
    let listed = (0..257)
        .map(|i| file(&format!("/a{i:03}"), 0))
        .collect::<Vec<_>>();
    assert_eq!(
        HostingManifest::parse(&json!({"version":1,"files":listed})).unwrap_err(),
        HostingRefusal::Bounds
    );
    assert_eq!(
        HostingManifest::parse(&json!({"version":1,"files":[file("/b",0),file("/a",0)]}))
            .unwrap_err(),
        HostingRefusal::Collision
    );
    // /a-b sorts between /a and /a/b, so adjacency alone does not detect this.
    let paths = ["/a", "/a-b", "/a/b", "/index.html"];
    let m = HostingManifest::parse(
        &json!({"version":1,"files":paths.iter().map(|p|file(p,5)).collect::<Vec<_>>()}),
    )
    .unwrap();
    let bytes=serde_json::to_vec(&json!({"version":1,"manifestDigest":m.digest(),"files":paths.iter().map(|p|json!({"path":p,"bytesBase64":STANDARD.encode(b"hello")})).collect::<Vec<_>>()})).unwrap();
    let bundle = HostingBundle::parse(&m, &bytes).unwrap();
    assert!(matches!(
        hosting::compose(&[bundle]),
        Err(HostingRefusal::Collision)
    ));
}
#[test]
fn matching_labels_do_not_adopt_an_edited_release_and_foreign_order_is_canonical() {
    let content = hosting::compose(&[bundle()]).unwrap().unwrap();
    let mut files = content.files().to_vec();
    let mut extra = files[0].clone();
    extra.path = "/extra.html".into();
    files.push(extra);
    let mut inventory = HostingInventory {
        site_exists: true,
        web_apps: vec![],
        live: Some(HostingLiveRelease {
            version: "version-1".into(),
            deployment_id: Some(ID.into()),
            plan_digest_prefix: Some("a".repeat(12)),
            content_digest: Some(content.digest().into()),
            files,
            config: content.config().clone(),
        }),
    };
    let first = hosting::deployment_view(&content, "demo-remote-1", ID, &inventory, None).unwrap();
    assert_eq!(first.replaces, "foreign");
    inventory.live.as_mut().unwrap().files.reverse();
    assert_eq!(
        hosting::deployment_view(&content, "demo-remote-1", ID, &inventory, None)
            .unwrap()
            .replaced_fingerprint,
        first.replaced_fingerprint
    );
}
