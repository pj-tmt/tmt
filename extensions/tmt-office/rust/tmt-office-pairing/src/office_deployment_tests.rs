use super::*;
use crate::office_http::tests::with_http_response;
use serde_json::{Value, json};
use std::{
    io,
    net::TcpListener,
    time::{Duration, Instant},
};

fn cloud() -> WorldTarget {
    WorldTarget::parse(
        "https://office.example/worlds/abcdefghijklmnopqrst",
        DeploymentMode::Cloud,
    )
    .unwrap()
}

fn descriptor() -> Value {
    json!({"version":1,"mode":"cloud","projectId":"example-office", "apiKey":"public-web-api-key", "pairingUrl":"https://issuer.example/officePairing"})
}

#[test]
fn literal_browser_deployments_pin_origin_and_exact_claim_target() {
    let corpus: Value =
        serde_json::from_str(include_str!("../../../contracts/deployment-examples.json")).unwrap();
    let examples = corpus["valid"].as_array().unwrap();
    assert_eq!(examples.len(), 3);
    for example in examples {
        let mode = serde_json::from_value(example["descriptor"]["mode"].clone()).unwrap();
        let target = WorldTarget::parse(example["worldUrl"].as_str().unwrap(), mode).unwrap();
        let deployment = OfficeDeployment::decode(
            target.clone(),
            &serde_json::to_vec(&example["descriptor"]).unwrap(),
        )
        .unwrap();
        assert_eq!(deployment.target(), &target);
        assert_eq!(deployment.project_id(), example["descriptor"]["projectId"]);
        assert_eq!(deployment.api_key(), example["descriptor"]["apiKey"]);
        assert_eq!(
            deployment.claim_url(),
            format!(
                "{}/claim",
                example["descriptor"]["pairingUrl"].as_str().unwrap()
            )
        );
    }
    assert_eq!(
        cloud().discovery_url(),
        "https://office.example/.well-known/tmt-office.json"
    );
}

#[test]
fn world_selection_rejects_credentials_normalization_and_extra_routes() {
    for value in [
        "http://office.example/worlds/abcdefghijklmnopqrst",
        "https://user:secret@office.example/worlds/abcdefghijklmnopqrst",
        "https://office.example/worlds/abcdefghijklmnopqrst?token=secret",
        "https://office.example/worlds/abcdefghijklmnopqrst#secret",
        "https://office.example/a/../worlds/abcdefghijklmnopqrst",
        "https://OFFICE.example/worlds/abcdefghijklmnopqrst",
        "https://office.example/worlds/abcdefghijklmnopqrst/",
        "https://office.example/worlds/abcdefghijklmnopqrst/pair",
        "https://office.example/worlds/%61bcdefghijklmnopqrst",
        "https://office.example/worlds/abc",
        "https://office.example:443/worlds/abcdefghijklmnopqrst",
        "https://office.example\\worlds/abcdefghijklmnopqrst",
    ] {
        assert_eq!(
            WorldTarget::parse(value, DeploymentMode::Cloud),
            Err(InvalidDeployment),
            "{value}"
        );
    }
}

#[test]
fn malformed_unknown_duplicate_and_oversize_documents_fail_closed() {
    let bytes = serde_json::to_vec(&descriptor()).unwrap();
    let mut exact_bound = bytes.clone();
    exact_bound.resize(DEPLOYMENT_LIMIT, b' ');
    assert!(OfficeDeployment::decode(cloud(), &exact_bound).is_ok());
    exact_bound.push(b' ');
    assert_eq!(
        OfficeDeployment::decode(cloud(), &exact_bound),
        Err(InvalidDeployment)
    );
    for key in ["version", "mode", "projectId", "apiKey", "pairingUrl"] {
        let mut missing = descriptor();
        let removed = missing.as_object_mut().unwrap().remove(key).unwrap();
        assert!(OfficeDeployment::decode(cloud(), &serde_json::to_vec(&missing).unwrap()).is_err());
        let duplicate = format!(
            "{{\"{key}\":{removed},{}",
            String::from_utf8(bytes.clone())
                .unwrap()
                .strip_prefix('{')
                .unwrap()
        );
        assert!(
            OfficeDeployment::decode(cloud(), duplicate.as_bytes()).is_err(),
            "duplicate {key}"
        );
    }
    for input in [
        b"[]".as_slice(),
        b"{}",
        b"\xff",
        b"null",
        b"{\"apiKey\":\"\\ud800\"}",
    ] {
        assert!(OfficeDeployment::decode(cloud(), input).is_err());
    }
    let mut unknown = descriptor();
    unknown["authUrl"] = json!("https://evil.example/token");
    assert!(OfficeDeployment::decode(cloud(), &serde_json::to_vec(&unknown).unwrap()).is_err());
}

#[test]
fn cloud_descriptor_rejects_retargeting_and_invalid_config_fields() {
    for (key, value) in [
        ("version", json!(2)),
        ("version", json!(1.0)),
        ("mode", json!("emulator")),
        ("mode", json!("other")),
        ("projectId", json!("demo-tmt-office")),
        ("projectId", json!("EXAMPLE-office")),
        ("apiKey", json!("secret?query")),
        ("apiKey", json!("")),
        ("apiKey", json!("a".repeat(257))),
        ("pairingUrl", json!("http://issuer.example/officePairing")),
        ("pairingUrl", json!("https://issuer.example/officePairing/")),
        (
            "pairingUrl",
            json!("https://issuer.example/a/../officePairing"),
        ),
        (
            "pairingUrl",
            json!("https://issuer.example/officePairing?token=x"),
        ),
        (
            "pairingUrl",
            json!("https://issuer.example/officePairing#x"),
        ),
        (
            "pairingUrl",
            json!("https://secret@issuer.example/officePairing"),
        ),
    ] {
        let mut input = descriptor();
        input[key] = value;
        assert!(
            OfficeDeployment::decode(cloud(), &serde_json::to_vec(&input).unwrap()).is_err(),
            "{key}: {input}"
        );
    }
    let mut root_issuer = descriptor();
    root_issuer["pairingUrl"] = json!("https://issuer.example");
    assert_eq!(
        OfficeDeployment::decode(cloud(), &serde_json::to_vec(&root_issuer).unwrap())
            .unwrap()
            .claim_url(),
        "https://issuer.example/claim"
    );
}

#[test]
fn emulator_requires_explicit_mode_and_exact_demo_endpoints() {
    let world = "http://127.0.0.1:4174/worlds/abcdefghijklmnopqrst";
    assert!(WorldTarget::parse(world, DeploymentMode::Cloud).is_err());
    for host in [
        "localhost:4174",
        "127.0.0.2:4174",
        "[::1]:4174",
        "127.0.0.1:0",
        "127.0.0.1",
    ] {
        assert!(
            WorldTarget::parse(
                &format!("http://{host}/worlds/abcdefghijklmnopqrst"),
                DeploymentMode::Emulator
            )
            .is_err()
        );
    }
    let target = WorldTarget::parse(world, DeploymentMode::Emulator).unwrap();
    let mut input = json!({"version":1,"mode":"emulator","projectId":DEMO_PROJECT,"apiKey":DEMO_PROJECT,"pairingUrl":DEMO_ISSUER});
    assert!(OfficeDeployment::decode(target.clone(), &serde_json::to_vec(&input).unwrap()).is_ok());
    for (key, value) in [
        ("projectId", "real-project"),
        ("apiKey", "other-key"),
        (
            "pairingUrl",
            "http://127.0.0.1:5002/demo-tmt-office/us-central1/officePairing",
        ),
    ] {
        let original = input[key].clone();
        input[key] = json!(value);
        assert!(
            OfficeDeployment::decode(target.clone(), &serde_json::to_vec(&input).unwrap()).is_err()
        );
        input[key] = original;
    }
}

#[test]
fn real_discovery_uses_exact_unauthenticated_path_and_bounds_response() {
    let body = json!({"version":1,"mode":"emulator","projectId":DEMO_PROJECT,"apiKey":DEMO_PROJECT,"pairingUrl":DEMO_ISSUER}).to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let (result, request) = with_http_response(response.into_bytes(), |origin| {
        let target = WorldTarget::parse(
            &format!("{origin}/worlds/abcdefghijklmnopqrst"),
            DeploymentMode::Emulator,
        )
        .unwrap();
        OfficeDeployment::discover(target, Instant::now() + Duration::from_secs(2))
    });
    assert_eq!(result.unwrap().project_id(), DEMO_PROJECT);
    assert!(request.starts_with("GET /.well-known/tmt-office.json HTTP/1.1\r\n"));
    let lower = request.to_ascii_lowercase();
    assert!(!lower.contains("authorization:"));
    assert!(!lower.contains("cookie:"));
    assert!(lower.contains("accept: application/json\r\n"));
    for (response, kind) in [
        ("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/credential-target\r\nContent-Length: 0\r\n\r\n".to_owned(), io::ErrorKind::InvalidData),
        ("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\n\r\n{}".to_owned(), io::ErrorKind::InvalidData),
        (format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 4097\r\n\r\n{}", " ".repeat(4097)), io::ErrorKind::InvalidData),
        ("HTTP/1.1 503 Unavailable\r\nContent-Type: application/json\r\nContent-Length: 16\r\n\r\nprivate-material".to_owned(), io::ErrorKind::Other),
    ] {
        let (result, _) = with_http_response(response.into_bytes(), |origin| {
            let target = WorldTarget::parse(
                &format!("{origin}/worlds/abcdefghijklmnopqrst"),
                DeploymentMode::Emulator,
            )
            .unwrap();
            OfficeDeployment::discover(target, Instant::now() + Duration::from_secs(2))
        });
        let error = result.unwrap_err();
        assert_eq!(error.kind(), kind);
        assert!(!error.to_string().contains("private-material"));
        assert!(!error.to_string().contains("credential-target"));
    }
}

#[test]
fn exhausted_discovery_deadline_does_not_connect() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let target = WorldTarget::parse(
        &format!(
            "http://{}/worlds/abcdefghijklmnopqrst",
            listener.local_addr().unwrap()
        ),
        DeploymentMode::Emulator,
    )
    .unwrap();
    let error = OfficeDeployment::discover(target, Instant::now()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
}
