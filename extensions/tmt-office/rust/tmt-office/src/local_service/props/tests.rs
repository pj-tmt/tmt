use super::super::{test_fixture::HttpFixture, tests::response_value};
use super::*;

fn request(path: &str, body: serde_json::Value) -> Request {
    Request {
        method: "POST".into(),
        path: path.into(),
        headers: vec![
            ("Authorization".into(), "Bearer browser".into()),
            ("Origin".into(), "http://127.0.0.1:1234".into()),
            ("Content-Type".into(), "application/json".into()),
        ],
        body: serde_json::to_vec(&body).unwrap(),
    }
}

fn document(label: &str) -> String {
    let mut value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../../contracts/office/prop-pack-v2-sample.tmtprop.json"
    ))
    .unwrap();
    value["label"] = json!(label);
    serde_json::to_string_pretty(&value).unwrap()
}

#[test]
fn browser_authoring_admission_precedes_storage_and_does_not_expand_other_routes() {
    let fixture = HttpFixture::new();
    let input = json!({"expectedRevision":0,"document":document("Private art")});
    for header in ["Authorization", "Origin", "Content-Type"] {
        let mut call = request(INSTALL, input.clone());
        call.headers.retain(|(name, _)| name != header);
        let expected = match header {
            "Authorization" => 401,
            "Origin" => 403,
            _ => 403,
        };
        assert!(
            fixture
                .call(call)
                .starts_with(&format!("HTTP/1.1 {expected}"))
        );
    }
    for body in [
        json!({"expectedRevision":0,"document":"{}"}),
        json!({"expectedRevision":0,"document":document("Private art"),"path":"/tmp/art"}),
        json!({"expectedRevision":-1,"document":document("Private art")}),
        json!({"expectedRevision":9007199254740992_u64,"document":document("Private art")}),
    ] {
        assert!(
            fixture
                .call(request(INSTALL, body))
                .starts_with("HTTP/1.1 400")
        );
    }
    let mut unsupported = request(INSTALL, input);
    unsupported.method = "GET".into();
    assert!(fixture.call(unsupported).starts_with("HTTP/1.1 405"));
    assert!(!fixture.paths.database.exists());
    assert_eq!(input_limit("POST", INSTALL), Some(INSTALL_ENVELOPE_LIMIT));
    assert_eq!(input_limit("POST", LIST), Some(LIST_INPUT_LIMIT));
    assert_eq!(input_limit("PUT", INSTALL), None);
    assert_eq!(
        input_limit("POST", "/api/v1/local/props/install/extra"),
        None
    );
}

#[test]
fn browser_install_shares_exact_bytes_revision_and_retry_with_the_existing_catalog() {
    let fixture = HttpFixture::new();
    let source = format!("{}\n", document("Private art"));
    let candidate = office_prop::validate_pack(source.as_bytes()).unwrap();
    let input = json!({"expectedRevision":0,"document":source});
    let installed = fixture.call(request(INSTALL, input.clone()));
    assert!(installed.starts_with("HTTP/1.1 200"));
    assert_eq!(
        response_value(&installed),
        json!({"revision":1,"digest":candidate.digest(),"changed":true})
    );
    let retried = fixture.call(request(INSTALL, input));
    assert!(retried.starts_with("HTTP/1.1 200"));
    assert_eq!(
        response_value(&retried),
        json!({"revision":1,"digest":candidate.digest(),"changed":false})
    );
    let conflict = fixture.call(request(
        INSTALL,
        json!({"expectedRevision":0,"document":document("Other art")}),
    ));
    assert!(conflict.starts_with("HTTP/1.1 409"));
    let mut storage = Storage::open(&fixture.paths.database).unwrap();
    let stored = storage.show_local_prop_pack(candidate.digest()).unwrap();
    assert_eq!(stored.pack.bytes(), source.as_bytes());
    assert_eq!(stored.catalog_revision, 1);
    assert_eq!(
        storage.list_local_prop_packs(20, None).unwrap().packs.len(),
        1
    );
    storage.close().unwrap();
    let list = fixture.call(request(LIST, json!({})));
    assert_eq!(
        response_value(&list),
        json!({"revision":1,"entries":[{"digest":candidate.digest(),"label":"Private art"}],"excluded":[],"nextCursor":null})
    );
}

#[test]
fn browser_catalog_pages_metadata_and_rejects_stale_cursor_without_altering_content() {
    let fixture = HttpFixture::new();
    let mut storage = Storage::open(&fixture.paths.database).unwrap();
    for revision in 0..21 {
        let candidate =
            office_prop::validate_pack(document(&format!("Art {revision}")).as_bytes()).unwrap();
        storage
            .install_local_prop_pack(revision, &candidate)
            .unwrap();
    }
    storage.close().unwrap();
    let first = response_value(&fixture.call(request(LIST, json!({}))));
    assert_eq!(first["entries"].as_array().unwrap().len(), 20);
    assert_eq!(first["revision"], 21);
    assert!(
        first["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry.as_object().unwrap().len() == 2)
    );
    let next = response_value(&fixture.call(request(LIST, json!({"cursor":first["nextCursor"]}))));
    assert_eq!(next["entries"].as_array().unwrap().len(), 1);
    assert!(next["nextCursor"].is_null());
    let added = fixture.call(request(
        INSTALL,
        json!({"expectedRevision":21,"document":document("Newest")}),
    ));
    assert!(added.starts_with("HTTP/1.1 200"));
    let stale = fixture.call(request(LIST, json!({"cursor":first["nextCursor"]})));
    assert!(stale.starts_with("HTTP/1.1 409"));
}

#[test]
fn prop_authoring_wire_budget_is_exact_and_does_not_leak_to_other_routes() {
    use super::super::tests::parse_wire;
    let wire = |method: &str, path: &str, length: usize| {
        let mut bytes = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:1\r\nContent-Length: {length}\r\n\r\n"
        )
        .into_bytes();
        bytes.resize(bytes.len() + length, b' ');
        bytes
    };
    assert!(parse_wire(&wire("POST", INSTALL, INSTALL_ENVELOPE_LIMIT)).is_ok());
    assert!(parse_wire(&wire("POST", LIST, LIST_INPUT_LIMIT)).is_ok());
    for (method, path, length) in [
        ("POST", INSTALL, INSTALL_ENVELOPE_LIMIT + 1),
        ("POST", LIST, LIST_INPUT_LIMIT + 1),
        ("PUT", INSTALL, INSTALL_ENVELOPE_LIMIT),
        (
            "POST",
            "/api/v1/local/props/install/extra",
            INSTALL_ENVELOPE_LIMIT,
        ),
        (
            "POST",
            "/api/v1/local/props/resolve",
            INSTALL_ENVELOPE_LIMIT,
        ),
    ] {
        assert!(parse_wire(&wire(method, path, length)).is_err());
    }
}
