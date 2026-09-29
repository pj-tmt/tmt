use super::*;
use crate::office_deployment::WorldTarget;
use crate::office_http::tests::with_http_response;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn scope() -> (OfficeDeployment, Approval) {
    let target = WorldTarget::parse(
        "https://office.example/worlds/abcdefghijklmnopqrst",
        DeploymentMode::Cloud,
    )
    .unwrap();
    let approval = Approval::new(
        &target,
        "00000000-0000-4000-8000-000000000001",
        "00000000-0000-4000-8000-000000000002",
        "Alice",
        false,
        &Proof::generate().unwrap(),
    )
    .unwrap();
    let deployment = OfficeDeployment::decode(target, br#"{"version":1,"mode":"cloud","projectId":"example-office","apiKey":"public-key","pairingUrl":"https://issuer.example"}"#).unwrap();
    (deployment, approval)
}

const PRINCIPAL: &str = "office-agent:00000000-0000-4000-8000-000000000003";

#[test]
fn revocation_requires_an_exact_confirmed_receipt() {
    let (_, approval) = scope();
    let receipt = json!({"version":1,"pairingId":approval.pairing_id(),"revoked":true});
    assert_eq!(
        decode_revocation(receipt.to_string().as_bytes(), &approval),
        Ok(())
    );
    for (key, value) in [
        ("version", json!(2)),
        ("pairingId", json!("0".repeat(64))),
        ("revoked", json!(false)),
        ("revoked", json!("true")),
        ("token", json!("unexpected")),
    ] {
        let mut invalid = receipt.clone();
        invalid[key] = value;
        assert_eq!(
            decode_revocation(invalid.to_string().as_bytes(), &approval),
            Err(OfficeError::CredentialsInvalid)
        );
    }
    let duplicate = receipt.to_string().replacen('{', "{\"revoked\":true,", 1);
    assert!(decode_revocation(duplicate.as_bytes(), &approval).is_err());
    assert!(decode_revocation(b"{}", &approval).is_err());
}

fn claims() -> Value {
    json!({"aud":"example-office","iss":"https://securetoken.google.com/example-office","sub":PRINCIPAL,
        "tmtOfficeAgent":true,"tmtInstallationId":"00000000-0000-4000-8000-000000000001",
        "tmtIdentityId":"00000000-0000-4000-8000-000000000002"})
}

fn credential(payload: &Value) -> AgentCredential {
    // Unsigned fixture tokens prove consistency validation, never authentication.
    let id_token = format!("e30.{}.", URL_SAFE_NO_PAD.encode(payload.to_string()));
    AgentCredential::from_response(id_token, "private-refresh-token".into(), "3600", 1000).unwrap()
}

fn emulator_deployment() -> OfficeDeployment {
    let target = WorldTarget::parse(
        "http://127.0.0.1:4174/worlds/abcdefghijklmnopqrst",
        DeploymentMode::Emulator,
    )
    .unwrap();
    OfficeDeployment::decode(
        target,
        br#"{"version":1,"mode":"emulator","projectId":"demo-tmt-office","apiKey":"demo-tmt-office","pairingUrl":"http://127.0.0.1:5001/demo-tmt-office/us-central1/officePairing"}"#,
    )
    .unwrap()
}

fn http_response(
    status: u16,
    headers: &[(&str, &str)],
    body: &[u8],
    declared_length: Option<usize>,
) -> Vec<u8> {
    let mut response = format!("HTTP/1.1 {status} Test\r\n").into_bytes();
    if (300..=399).contains(&status) {
        response.extend_from_slice(b"Location: http://127.0.0.1:1/credential-target\r\n");
    }
    for (name, value) in headers {
        response.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    if let Some(length) = declared_length {
        response.extend_from_slice(format!("Content-Length: {length}\r\n").as_bytes());
    }
    response.extend_from_slice(b"Connection: close\r\n\r\n");
    response.extend_from_slice(body);
    response
}

fn post_fixture(
    deployment: &OfficeDeployment,
    response: Vec<u8>,
    claim: bool,
) -> (Result<Vec<u8>, OfficeError>, String) {
    with_http_response(response, |origin| {
        post(
            deployment,
            &format!("{origin}/claim"),
            "{\"version\":1}",
            "application/json",
            Instant::now() + Duration::from_secs(2),
            if claim {
                PostPurpose::Claim
            } else {
                PostPurpose::Auth
            },
        )
    })
}

#[test]
fn bounded_post_statuses_preserve_claim_auth_and_uncertain_meanings() {
    let deployment = emulator_deployment();
    let json = br#"{}"#;
    for (status, claim, expected) in [
        (404, true, OfficeError::PairingPending),
        (429, true, OfficeError::PairingPending),
        (400, false, OfficeError::RemoteDenied),
        (401, false, OfficeError::RemoteDenied),
        (403, false, OfficeError::RemoteDenied),
        (409, false, OfficeError::RemoteDenied),
        (503, false, OfficeError::RemoteUncertain),
        (302, false, OfficeError::CredentialsInvalid),
    ] {
        let response = http_response(
            status,
            &[("Content-Type", "application/json")],
            json,
            Some(json.len()),
        );
        let (result, request) = post_fixture(&deployment, response, claim);
        assert_eq!(result, Err(expected), "status {status}, claim {claim}");
        assert!(request.starts_with("POST /claim HTTP/1.1"));
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("content-type: application/json\r\n"));
        assert!(lower.contains("accept: application/json\r\n"));
    }
}

#[test]
fn bounded_post_body_failures_distinguish_exact_invalid_from_transport_uncertainty() {
    let deployment = emulator_deployment();
    let exact = br#"{"version":1}"#;
    let (result, _) = post_fixture(
        &deployment,
        http_response(
            200,
            &[("Content-Type", "application/json; charset=utf-8")],
            exact,
            Some(exact.len()),
        ),
        false,
    );
    assert_eq!(result, Ok(exact.to_vec()));

    let truncated = br#"{}"#;
    let (result, _) = post_fixture(
        &deployment,
        http_response(
            200,
            &[("Content-Type", "application/json")],
            truncated,
            Some(truncated.len() + 4),
        ),
        false,
    );
    assert_eq!(result, Err(OfficeError::RemoteUncertain));

    let oversized = vec![b'a'; RESPONSE_LIMIT + 1];
    let (result, _) = post_fixture(
        &deployment,
        http_response(
            200,
            &[("Content-Type", "application/json")],
            &oversized,
            Some(oversized.len()),
        ),
        false,
    );
    assert_eq!(result, Err(OfficeError::CredentialsInvalid));

    let non_json = br#"{}"#;
    let (result, _) = post_fixture(
        &deployment,
        http_response(
            200,
            &[("Content-Type", "text/plain")],
            non_json,
            Some(non_json.len()),
        ),
        false,
    );
    assert_eq!(result, Err(OfficeError::CredentialsInvalid));
}

#[test]
fn token_consistency_rejects_every_wrong_scope_and_malformed_payload() {
    let (deployment, approval) = scope();
    credential(&claims())
        .validate(&deployment, &approval, PRINCIPAL)
        .unwrap();
    for (field, value) in [
        ("aud", json!("other-project")),
        ("iss", json!("https://issuer.example")),
        ("sub", json!("human-owner")),
        ("tmtOfficeAgent", json!(false)),
        (
            "tmtInstallationId",
            json!("00000000-0000-4000-8000-000000000009"),
        ),
        (
            "tmtIdentityId",
            json!("00000000-0000-4000-8000-000000000009"),
        ),
    ] {
        let mut changed = claims();
        changed[field] = value;
        assert_eq!(
            credential(&changed).validate(&deployment, &approval, PRINCIPAL),
            Err(OfficeError::CredentialsInvalid),
            "{field}"
        );
        changed.as_object_mut().unwrap().remove(field);
        assert!(
            credential(&changed)
                .validate(&deployment, &approval, PRINCIPAL)
                .is_err()
        );
    }
    for token in [
        "",
        "private-token",
        "a.!!!!.b",
        "a.e30.b.extra",
        "a.e30.b\n",
    ] {
        let mut value = credential(&claims());
        value.id_token = token.into();
        assert_eq!(
            value.validate(&deployment, &approval, PRINCIPAL),
            Err(OfficeError::CredentialsInvalid)
        );
    }
    let duplicate = claims()
        .to_string()
        .replacen('{', "{\"aud\":\"other-project\",", 1);
    let mut value = credential(&claims());
    value.id_token = format!("e30.{}.", URL_SAFE_NO_PAD.encode(duplicate));
    assert!(value.validate(&deployment, &approval, PRINCIPAL).is_err());
}

#[test]
fn bounded_token_clock_and_secret_record_do_not_accept_missing_or_extra_fields() {
    let value = credential(&claims());
    assert_eq!(value.token_expires_at(), 3_601_000);
    for seconds in [
        "0",
        "3601",
        "-1",
        "01",
        "1.0",
        "1e3",
        "",
        "18446744073709551615",
    ] {
        assert!(
            AgentCredential::from_response("token".into(), "refresh".into(), seconds, 1000)
                .is_err()
        );
    }
    assert!(
        AgentCredential::from_response("token".into(), "refresh".into(), "3600", u64::MAX).is_err()
    );
    let serialized = serde_json::to_vec(&value).unwrap();
    let decoded: AgentCredential = serde_json::from_slice(&serialized).unwrap();
    assert_eq!(decoded.refresh_token, "private-refresh-token");
    for key in ["idToken", "refreshToken", "tokenExpiresAt"] {
        let mut input = serde_json::to_value(&value).unwrap();
        input.as_object_mut().unwrap().remove(key);
        assert!(serde_json::from_value::<AgentCredential>(input).is_err());
    }
    let extra = String::from_utf8(serialized).unwrap().replacen(
        '{',
        "{\"customToken\":\"not-durable\",",
        1,
    );
    assert!(serde_json::from_str::<AgentCredential>(&extra).is_err());
}

#[test]
fn fixed_auth_endpoints_do_not_come_from_issuer_or_browser_link() {
    let (deployment, _) = scope();
    assert_eq!(
        auth_url(&deployment, "accounts:signInWithCustomToken"),
        "https://identitytoolkit.googleapis.com/v1/accounts:signInWithCustomToken?key=public-key"
    );
    let target = WorldTarget::parse(
        "http://127.0.0.1:4174/worlds/abcdefghijklmnopqrst",
        DeploymentMode::Emulator,
    )
    .unwrap();
    let deployment = OfficeDeployment::decode(target, br#"{"version":1,"mode":"emulator","projectId":"demo-tmt-office","apiKey":"demo-tmt-office","pairingUrl":"http://127.0.0.1:5001/demo-tmt-office/us-central1/officePairing"}"#).unwrap();
    assert_eq!(
        auth_url(&deployment, "accounts:signInWithCustomToken"),
        "http://127.0.0.1:9099/identitytoolkit.googleapis.com/v1/accounts:signInWithCustomToken?key=demo-tmt-office"
    );
}
