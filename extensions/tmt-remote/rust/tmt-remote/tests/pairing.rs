//! Pairing acceptance on real sockets: a door with `/pair`, serve's control
//! socket, and a Rust test device that signs with a fixed test key.
#[allow(dead_code)]
#[path = "support/door.rs"]
mod door;

use door::*;
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use std::{sync::Arc, thread, time::Duration};
use tmt_remote::{
    canonical,
    control::{self, Control},
    crypto,
    pairing::Timing,
    store::{Grant, uuid_v4},
};

const FAST: Timing = Timing {
    lifetime: Duration::from_secs(30),
    submit_wait: Duration::from_secs(10),
};

fn automatic(h: &Harness, target: &str) -> Value {
    use std::io::{BufRead, BufReader, Write};
    let mut stream = control::connect(&h.root.join("remote")).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    writeln!(stream, "{}", json!({"op":"auto-pair","target":target})).unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn automatic_pairing_is_first_use_only_and_exactly_the_read_only_default() {
    let h = Harness::new(FAST);
    let before = tmt_remote::pairing::now_ms().unwrap();
    let offer = automatic(&h, "/settings");
    let link = offer["link"]
        .as_str()
        .expect("native open returns a launch URL");
    let code = canonical::pairing_code(link.split_once("#auto=").unwrap().1).unwrap();
    let descriptor = h
        .pairing
        .descriptor(&format!("{}{}", h.origin, h.prefix))
        .unwrap();
    assert_eq!(descriptor["target"], "/settings");
    assert_eq!(descriptor["automatic"], true);
    assert!((before + 59_000..=before + 61_000).contains(&offer["expiresAtMs"].as_u64().unwrap()));
    let device = Device::browser(&h, 71);
    let body = device.body(&descriptor, &code, &device.key).to_string();
    let (status, reply) = h.post(&body, Some(&h.origin));
    assert_eq!(
        status, 200,
        "no terminal confirmation for an owner-opened auto offer"
    );
    let receipt = canonical::base64url_decode(reply["receipt"].as_str().unwrap()).unwrap();
    let parsed: Value = serde_json::from_slice(&receipt).unwrap();
    assert_eq!(
        parsed["grant"]["scopes"],
        json!(["agents.read", "check.read", "results.read", "status.read"])
    );
    assert_eq!(parsed["grant"]["origin"], h.origin);
    assert!(parsed["grant"]["issuedAtMs"].as_u64().unwrap() >= before);
    assert_eq!(
        h.post(&body, Some(&h.origin)).0,
        404,
        "even exact auto replay is refused"
    );
    let other = Device::browser(&h, 72);
    assert_eq!(
        h.post(
            &other.body(&descriptor, &code, &other.key).to_string(),
            Some(&h.origin)
        )
        .0,
        404
    );
    assert_eq!(h.grants(), 1);
    assert!(h.pairing.descriptor(&h.origin).is_none());
}

#[test]
fn automatic_open_replaces_only_an_automatic_offer_and_refuses_unsafe_targets() {
    let h = Harness::new(FAST);
    let first = automatic(&h, "/");
    let first_link = first["link"].as_str().expect("auto offer");
    let first_code = canonical::pairing_code(first_link.split_once("#auto=").unwrap().1).unwrap();
    let first_descriptor = h.pairing.descriptor(&h.origin).unwrap();
    let next = automatic(&h, "/colab/");
    assert_ne!(first["offerId"], next["offerId"]);
    let device = Device::browser(&h, 73);
    assert_eq!(
        h.post(
            &device
                .body(&first_descriptor, &first_code, &device.key)
                .to_string(),
            Some(&h.origin)
        )
        .0,
        404
    );
    let mut manual = open(&h);
    assert_eq!(automatic(&h, "/")["error"]["code"], "REMOTE_PAIRING_BUSY");
    assert_eq!(
        h.pairing.descriptor(&h.origin).unwrap()["offerId"],
        manual.descriptor["offerId"]
    );
    for target in [
        "https://example.test/",
        "//evil/",
        "/p/a?code=secret",
        "/p/a#secret",
        "/../",
        "/r/secret",
        "/sdk/pair-offer",
    ] {
        assert_eq!(
            automatic(&h, target)["error"]["code"],
            "REMOTE_INPUT_INVALID",
            "{target}"
        );
    }
    manual.owner.answer("refuse");
    assert_eq!(manual.owner.next()["reason"], "refused");
}

#[test]
fn a_live_paired_key_cancels_an_auto_offer_only_with_a_distinct_possession_proof() {
    use ed25519_dalek::Signer;
    let h = Harness::new(FAST);
    let device = Device::browser(&h, 74);
    // First auto enrollment establishes the existing read-only device.
    let first = automatic(&h, "/");
    let code = canonical::pairing_code(
        first["link"]
            .as_str()
            .unwrap()
            .split_once("#auto=")
            .unwrap()
            .1,
    )
    .unwrap();
    let descriptor = h.pairing.descriptor(&h.origin).unwrap();
    assert_eq!(
        h.post(
            &device.body(&descriptor, &code, &device.key).to_string(),
            Some(&h.origin)
        )
        .0,
        200
    );
    let unchanged = h.store.lock().unwrap().grants().unwrap();
    let second = automatic(&h, "/settings");
    let code = canonical::pairing_code(
        second["link"]
            .as_str()
            .unwrap()
            .split_once("#auto=")
            .unwrap()
            .1,
    )
    .unwrap();
    let descriptor = h.pairing.descriptor(&h.origin).unwrap();
    let mut body = device.body(&descriptor, &code, &device.key);
    body["cancel"] = json!(true);
    assert_eq!(
        h.post(&body.to_string(), Some(&h.origin)).0,
        404,
        "enrollment signature cannot be used as cancellation"
    );
    let mac: [u8; 32] = canonical::base64url_bytes(body["mac"].as_str().unwrap(), 32)
        .unwrap()
        .try_into()
        .unwrap();
    let bytes = canonical::cancellation(&device.enrollment(&descriptor), &mac).unwrap();
    body["signature"] = json!(canonical::base64url(&device.key.sign(&bytes).to_bytes()));
    assert_eq!(
        h.post(&body.to_string(), Some("https://foreign.example")).0,
        404
    );
    let unknown = Device::browser(&h, 76);
    let mut unknown_body = unknown.body(&descriptor, &code, &unknown.key);
    unknown_body["cancel"] = json!(true);
    let unknown_mac: [u8; 32] =
        canonical::base64url_bytes(unknown_body["mac"].as_str().unwrap(), 32)
            .unwrap()
            .try_into()
            .unwrap();
    unknown_body["signature"] = json!(canonical::base64url(
        &unknown
            .key
            .sign(&canonical::cancellation(&unknown.enrollment(&descriptor), &unknown_mac).unwrap())
            .to_bytes()
    ));
    assert_eq!(
        h.post(&unknown_body.to_string(), Some(&h.origin)).0,
        404,
        "a correct code and cancellation signature still require a live stored key"
    );
    assert_eq!(
        h.pairing.descriptor(&h.origin).unwrap()["offerId"],
        descriptor["offerId"]
    );
    assert_eq!(h.post(&body.to_string(), Some(&h.origin)).0, 204);
    assert!(h.pairing.descriptor(&h.origin).is_none());
    assert_eq!(h.post(&body.to_string(), Some(&h.origin)).0, 404);
    let fresh = Device::browser(&h, 75);
    assert_eq!(
        h.post(
            &fresh.body(&descriptor, &code, &fresh.key).to_string(),
            Some(&h.origin)
        )
        .0,
        404
    );
    assert_eq!(h.grants(), 1, "cancellation never creates a second grant");
    assert_eq!(h.store.lock().unwrap().grants().unwrap(), unchanged);
}

#[test]
fn suppressed_missing_and_failed_openers_do_not_leave_an_automatic_offer_or_leak_its_code() {
    use tmt_invoke::open::{Env, Flag, Outcome};
    use tmt_remote::open::open_page_with;
    let h = Harness::new(FAST);
    let env = Env {
        terminal: true,
        ..Env::default()
    };
    let directory = h.root.join("remote");
    let forbidden = |_: &str| -> Outcome { panic!("suppressed opener must never run") };
    for (flag, json_output, opener) in [
        (
            Flag::NoOpen,
            false,
            Some(&forbidden as &dyn Fn(&str) -> Outcome),
        ),
        (
            Flag::Open,
            true,
            Some(&forbidden as &dyn Fn(&str) -> Outcome),
        ),
        (Flag::Unset, false, None),
    ] {
        let result = open_page_with(
            &directory,
            "/settings",
            flag,
            true,
            json_output,
            &env,
            opener,
        )
        .unwrap();
        assert!(!result.opened);
        assert_eq!(result.link, format!("{}/settings", h.origin));
        assert!(h.pairing.descriptor(&h.origin).is_none());
    }
    let captured = std::cell::RefCell::new(String::new());
    let failed = |url: &str| {
        captured.replace(url.to_owned());
        Outcome::Failed(url.to_owned())
    };
    let result = open_page_with(
        &directory,
        "/settings",
        Flag::Open,
        true,
        false,
        &env,
        Some(&failed),
    )
    .unwrap();
    assert_eq!(result.state, "failed");
    assert!(captured.borrow().contains("#auto="));
    let shown = serde_json::to_string(&result).unwrap();
    assert!(!shown.contains('#') && !shown.contains(captured.borrow().as_str()));
    assert!(h.pairing.descriptor(&h.origin).is_none());
}

#[test]
fn confirmed_pairing_issues_one_grant_and_a_verifiable_receipt() {
    let h = Harness::new(FAST);
    let Offered {
        mut owner,
        code,
        descriptor,
        link,
    } = open(&h);
    // The short link carries only the 128-bit code, in its fragment.
    let (path, fragment) = link.split_once('#').unwrap();
    assert_eq!(path, format!("{}/pair", h.origin));
    assert_eq!(fragment.len(), 26);
    assert_eq!(canonical::pairing_code(fragment).unwrap(), code);
    assert_eq!(descriptor["address"], format!("{}{}", h.origin, h.prefix));
    assert_eq!(descriptor["serverChallenge"].as_str().unwrap().len(), 32);
    let device = Device::browser(&h, 7);
    let body = device.body(&descriptor, &code, &device.key).to_string();
    let submit = {
        let (addr, prefix, origin) = (h.addr, h.prefix.clone(), h.origin.clone());
        let body = body.clone();
        thread::spawn(move || post_to(addr, &prefix, &body, Some(&origin)))
    };
    let candidate = owner.next();
    assert_eq!(candidate["event"], "candidate");
    assert_eq!(candidate["kind"], "browser");
    assert_eq!(candidate["origin"], h.origin.as_str());
    assert_eq!(candidate["name"], "Laptop é");
    assert_eq!(
        candidate["words"],
        json!(canonical::fingerprint_words(&device.public()).unwrap())
    );
    assert_eq!(h.grants(), 0, "no grant before confirmation");
    owner.answer("confirm");
    let ended = owner.next();
    assert_eq!(ended["reason"], "paired");
    let (status, reply) = submit.join().unwrap();
    assert_eq!(status, 200);
    let receipt = canonical::base64url_decode(reply["receipt"].as_str().unwrap()).unwrap();
    let proof = canonical::base64url_bytes(reply["serverProof"].as_str().unwrap(), 32).unwrap();
    // The device derives K_response from its code and checks the exact receipt bytes.
    let enrollment = device.enrollment(&descriptor);
    let key = crypto::response_key(&code, &enrollment).unwrap();
    assert!(crypto::verify_server_proof(&key, &receipt, &proof).is_ok());
    assert!(
        crypto::verify_server_proof(
            &crypto::response_key(&[0; 16], &enrollment).unwrap(),
            &receipt,
            &proof
        )
        .is_err()
    );
    let parsed: Value = serde_json::from_slice(&receipt).unwrap();
    let grant = &parsed["grant"];
    assert_eq!(grant["clientId"], ended["clientId"]);
    assert_eq!(grant["machineId"], descriptor["machineId"]);
    assert_eq!(grant["profile"], "local-v1");
    assert_eq!(grant["publicKey"], canonical::base64url(&device.public()));
    assert_eq!(grant["kind"], "browser");
    assert_eq!(grant["origin"], h.origin.as_str());
    assert_eq!(grant["agents"], "all");
    assert_eq!(grant["mode"], "direct");
    assert_eq!(grant["expiresAtMs"], Value::Null);
    assert_eq!(grant["revision"], 1);
    assert_eq!(grant["disabled"], false);
    assert_eq!(
        grant["scopes"],
        json!(["agents.read", "check.read", "results.read", "status.read"])
    );
    assert_eq!(
        parsed["machinePublicKey"],
        canonical::base64url(&h.machine_public)
    );
    let stored: Grant = h
        .store
        .lock()
        .unwrap()
        .grant(grant["clientId"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(stored.public_key, device.public());
    assert_eq!(h.grants(), 1);
    // An exact retry recovers the same receipt; nothing else is accepted.
    assert_eq!(h.post(&body, Some(&h.origin)), (200, reply.clone()));
    assert_eq!(h.grants(), 1, "retry creates no second grant");
    let other = Device::browser(&h, 8);
    assert_eq!(
        h.post(
            &other.body(&descriptor, &code, &other.key).to_string(),
            Some(&h.origin)
        )
        .0,
        404
    );
}

#[test]
fn three_failed_code_proofs_cancel_the_offer() {
    let h = Harness::new(FAST);
    let Offered {
        mut owner,
        code,
        descriptor,
        ..
    } = open(&h);
    let device = Device::browser(&h, 7);
    let mut wrong = code;
    wrong[0] ^= 1;
    let origin = Some(h.origin.as_str());
    for _ in 0..2 {
        let bad = device.body(&descriptor, &wrong, &device.key).to_string();
        assert_eq!(h.post(&bad, origin).0, 404);
    }
    // Two failures leave the offer open: a correct candidate still pins.
    let pending = submit(
        &h,
        device.body(&descriptor, &code, &device.key),
        Some(h.origin.clone()),
    );
    assert_eq!(owner.next()["event"], "candidate");
    owner.answer("refuse");
    assert_eq!(owner.next()["reason"], "refused");
    assert_eq!(pending.join().unwrap().0, 404);
    // A fresh offer cancels on the third failure; the right code then refuses.
    let Offered {
        mut owner,
        code,
        descriptor,
        ..
    } = open(&h);
    for _ in 0..3 {
        let bad = device.body(&descriptor, &wrong, &device.key).to_string();
        assert_eq!(h.post(&bad, origin).0, 404);
    }
    assert_eq!(owner.next()["reason"], "too-many-failures");
    let good = device.body(&descriptor, &code, &device.key).to_string();
    assert_eq!(h.post(&good, origin).0, 404);
    assert_eq!(h.grants(), 0);
}

#[test]
fn wrong_possession_origin_and_fields_refuse_without_pinning() {
    let h = Harness::new(Timing {
        lifetime: Duration::from_secs(30),
        submit_wait: Duration::from_millis(300),
    });
    let Offered {
        mut owner,
        code,
        descriptor,
        ..
    } = open(&h);
    let device = Device::browser(&h, 7);
    let origin = Some(h.origin.as_str());
    // Signed by another key: possession fails and nothing is pinned.
    let impostor = SigningKey::from_bytes(&[3; 32]);
    let forged = device.body(&descriptor, &code, &impostor).to_string();
    assert_eq!(h.post(&forged, origin).0, 404);
    let good = device.body(&descriptor, &code, &device.key);
    // A page on another live loopback port that proposes and sends its own
    // origin is still not this door's origin.
    let elsewhere = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let foreign = Device {
        origin: format!(
            "http://127.0.0.1:{}",
            elsewhere.local_addr().unwrap().port()
        ),
        ..Device::browser(&h, 7)
    };
    let mut cases = vec![
        (
            foreign.body(&descriptor, &code, &foreign.key).to_string(),
            Some(foreign.origin.clone()),
            "another loopback port",
        ),
        (good.to_string(), None, "missing Origin"),
        (
            good.to_string(),
            Some("http://127.0.0.1:1".to_owned()),
            "other Origin",
        ),
    ];
    for (field, value) in [
        ("machineId", json!(uuid_v4().unwrap())),
        ("windowId", json!(uuid_v4().unwrap())),
        ("offerId", json!(uuid_v4().unwrap())),
        ("serverChallenge", json!("00".repeat(16))),
        ("profile", json!("cloud-v1")),
        (
            "publicKey",
            json!(format!("{}=", good["publicKey"].as_str().unwrap())),
        ),
        ("mac", json!(1)),
        ("extra", json!("field")),
    ] {
        let mut changed = good.clone();
        changed[field] = value;
        cases.push((changed.to_string(), Some(h.origin.clone()), field));
    }
    cases.push(("not json".into(), Some(h.origin.clone()), "framing"));
    for (body, header, label) in cases {
        assert_eq!(h.post(&body, header.as_deref()).0, 404, "{label}");
    }
    // None of those pinned or ended the offer: the valid candidate pins, then
    // reports pending when the owner has not answered within the wait.
    let (status, body) = h.post(&good.to_string(), origin);
    assert_eq!((status, body), (202, json!({"state": "pending"})));
    assert_eq!(owner.next()["event"], "candidate");
    // A competing candidate cannot replace the pinned one.
    let other = Device::browser(&h, 8);
    assert_eq!(
        h.post(
            &other.body(&descriptor, &code, &other.key).to_string(),
            origin
        )
        .0,
        404
    );
    // The identical candidate coalesces and receives the receipt once confirmed.
    let retry = submit(&h, good.clone(), Some(h.origin.clone()));
    thread::sleep(Duration::from_millis(50));
    owner.answer("confirm");
    assert_eq!(owner.next()["reason"], "paired");
    assert_eq!(retry.join().unwrap().0, 200);
    assert_eq!(h.grants(), 1);
}

#[test]
fn refusal_disconnect_expiry_and_stop_end_the_offer_without_a_grant() {
    let h = Harness::new(Timing {
        lifetime: Duration::from_millis(800),
        submit_wait: Duration::from_secs(5),
    });
    let device = Device::browser(&h, 7);
    // Owner refuses: the waiting device is refused.
    let Offered {
        mut owner,
        code,
        descriptor,
        ..
    } = open(&h);
    let waiting = submit(
        &h,
        device.body(&descriptor, &code, &device.key),
        Some(h.origin.clone()),
    );
    assert_eq!(owner.next()["event"], "candidate");
    owner.answer("refuse");
    assert_eq!(owner.next()["reason"], "refused");
    assert_eq!(waiting.join().unwrap().0, 404);
    // The pairing command exits: the offer is cancelled.
    let Offered {
        owner,
        code,
        descriptor,
        ..
    } = open(&h);
    drop(owner);
    thread::sleep(Duration::from_millis(200));
    let body = device.body(&descriptor, &code, &device.key).to_string();
    assert_eq!(h.post(&body, Some(&h.origin)).0, 404);
    // No confirmation before the deadline: expiry ends it.
    let Offered {
        mut owner,
        code,
        descriptor,
        ..
    } = open(&h);
    let waiting = submit(
        &h,
        device.body(&descriptor, &code, &device.key),
        Some(h.origin.clone()),
    );
    assert_eq!(owner.next()["event"], "candidate");
    assert_eq!(owner.next()["reason"], "expired");
    assert_eq!(waiting.join().unwrap().0, 404);
    // A new offer replaces the previous one explicitly.
    let Offered {
        owner: mut first, ..
    } = open(&h);
    let _second = open(&h);
    assert_eq!(first.next()["reason"], "replaced");
    assert_eq!(h.grants(), 0);
}

#[test]
fn enrollment_is_atomic_when_the_grant_cannot_be_written() {
    let h = Harness::new(FAST);
    let device = Device::browser(&h, 7);
    // A live grant for the same device key already exists.
    h.store
        .lock()
        .unwrap()
        .insert_grant(&Grant {
            client_id: uuid_v4().unwrap(),
            public_key: device.public(),
            kind: "cli".into(),
            origin: "cli".into(),
            name: "Earlier".into(),
            agents: "all".into(),
            scopes: vec!["talk".into()],
            mode: "direct".into(),
            issued_at_ms: 1,
            expires_at_ms: None,
            revision: 1,
            disabled: false,
        })
        .unwrap();
    let Offered {
        mut owner,
        code,
        descriptor,
        ..
    } = open(&h);
    let waiting = submit(
        &h,
        device.body(&descriptor, &code, &device.key),
        Some(h.origin.clone()),
    );
    assert_eq!(owner.next()["event"], "candidate");
    owner.answer("confirm");
    let ended = owner.next();
    assert_eq!(ended["reason"], "failed");
    assert!(ended["message"].as_str().unwrap().contains("live grant"));
    assert_eq!(waiting.join().unwrap().0, 404);
    assert_eq!(h.grants(), 1, "no partial second grant");
}

#[test]
fn stopping_serve_cancels_a_pending_offer() {
    let mut h = Harness::new(FAST);
    let Offered { mut owner, .. } = open(&h);
    let control = h.control.take().unwrap();
    control.stop();
    assert_eq!(owner.next()["reason"], "cancelled");
    assert!(!h.root.join("remote/control.sock").exists());
    h.control = Some(
        Control::start(
            &h._serving,
            Arc::clone(&h.pairing),
            Arc::clone(&h.devices),
            control::Door {
                origin: h.origin.clone(),
                prefix: h.prefix.clone(),
            },
            None,
            Arc::clone(&h.stop),
        )
        .unwrap(),
    );
}

#[test]
fn owner_talk_policy_narrows_agents_and_hold_in_the_signed_receipt_and_atomic_audit() {
    use std::io::Write;
    let h = Harness::new(FAST);
    let Offered {
        mut owner,
        code,
        descriptor,
        ..
    } = open(&h);
    let device = Device::browser(&h, 7);
    let pending = submit(
        &h,
        device.body(&descriptor, &code, &device.key),
        Some(device.origin.clone()),
    );
    assert_eq!(owner.next()["event"], "candidate");
    let agent = uuid_v4().unwrap();
    writeln!(
        owner.stream,
        "{}",
        json!({"op":"confirm","policy":{"talk":true,"agents":[agent],"hold":true}})
    )
    .unwrap();
    let ended = owner.next();
    assert_eq!(ended["reason"], "paired");
    let (status, reply) = pending.join().unwrap();
    assert_eq!(status, 200);
    let receipt = canonical::base64url_decode(reply["receipt"].as_str().unwrap()).unwrap();
    let enrollment = device.enrollment(&descriptor);
    let key = crypto::response_key(&code, &enrollment).unwrap();
    crypto::verify_server_proof(
        &key,
        &receipt,
        &canonical::base64url_bytes(reply["serverProof"].as_str().unwrap(), 32).unwrap(),
    )
    .unwrap();
    let result: Value = serde_json::from_slice(&receipt).unwrap();
    assert_eq!(result["grant"]["agents"], json!([agent]));
    assert_eq!(result["grant"]["mode"], "hold");
    let stored = h
        .store
        .lock()
        .unwrap()
        .grant(ended["clientId"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert!(stored.permits_scope("talk"));
    let oracle = rusqlite::Connection::open(h.root.join("remote/remote.db")).unwrap();
    let count: i64 = oracle
        .query_row(
            "SELECT COUNT(*) FROM audit WHERE operation='remote.pair' AND decision='committed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}
