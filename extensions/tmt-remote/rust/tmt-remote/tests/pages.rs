//! Door-served browser assets on real sockets: the pairing page, the SDK
//! module and the mount lookup, each disjoint from the prefixed routes and mounts.
#[allow(dead_code)]
#[path = "support/door.rs"]
mod door;

use door::*;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};
use tmt_remote::pairing::Timing;

const FAST: Timing = Timing {
    lifetime: Duration::from_secs(30),
    submit_wait: Duration::from_secs(10),
};
struct Reply {
    status: u16,
    head: String,
    body: String,
}
impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().find_map(|line| {
            let (key, value) = line.split_once(": ")?;
            key.eq_ignore_ascii_case(name).then_some(value)
        })
    }
}
fn exchange(h: &Harness, request: &str) -> Reply {
    let mut client = TcpStream::connect(h.addr).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    client.write_all(request.as_bytes()).unwrap();
    let mut reply = String::new();
    client.read_to_string(&mut reply).unwrap();
    let (head, body) = reply.split_once("\r\n\r\n").unwrap();
    Reply {
        status: head[9..12].parse().unwrap(),
        head: head.to_owned(),
        body: body.to_owned(),
    }
}
fn get(h: &Harness, path: &str, headers: &str) -> Reply {
    exchange(
        h,
        &format!("GET {path} HTTP/1.1\r\nHost: {}\r\n{headers}\r\n", h.addr),
    )
}
fn mount(h: &Harness, body: &str, origin: Option<&str>) -> Reply {
    let origin = origin
        .map(|o| format!("Origin: {o}\r\n"))
        .unwrap_or_default();
    exchange(
        h,
        &format!(
            "POST /sdk/mount HTTP/1.1\r\nHost: {}\r\n{origin}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            h.addr,
            body.len()
        ),
    )
}

#[test]
fn the_pairing_page_and_sdk_are_served_with_exact_types_and_policy() {
    let h = Harness::new(FAST);
    let Offered { link, .. } = open(&h);
    let path = link
        .strip_prefix(&h.origin)
        .unwrap()
        .split('#')
        .next()
        .unwrap();
    let page = get(&h, path, "");
    assert_eq!(page.status, 200);
    assert_eq!(
        page.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(
        page.header("content-security-policy"),
        Some(
            "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
        )
    );
    assert_eq!(page.header("cache-control"), Some("no-store"));
    assert_eq!(page.body, include_str!("../assets/pair.html"));
    assert!(
        page.body
            .contains(r#"<link rel="stylesheet" href="/sdk/pages.css" />"#)
    );
    assert!(page.body.contains(r#"data-tmt-page="pair""#));
    assert!(page.body.contains(r#"<script src="/sdk/pair.js">"#));
    let sdk = get(
        &h,
        "/sdk/remote-v1.js",
        &format!("Origin: {}\r\n", h.origin),
    );
    assert_eq!(sdk.status, 200);
    assert_eq!(
        sdk.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(sdk.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(
        sdk.body,
        include_str!("../assets/remote-v1.js"),
        "the embedded module is served unchanged"
    );
    // Only the exact grammar is served; nothing here reaches operations or mounts.
    for (path, headers, status) in [
        ("/pair/", "", 404),
        ("/pair/a.b", "", 404),
        ("/pair/abc/def", "", 404),
        (&format!("/pair/{}", "a".repeat(4097)), "", 404),
        ("/sdk/remote-v2.js", "", 404),
        ("/sdk/remote-v1.js", "Origin: http://127.0.0.1:1\r\n", 403),
        ("/sdk/mount", "", 404),
    ] {
        let label = &path[..path.len().min(40)];
        assert_eq!(get(&h, path, headers).status, status, "{label}");
    }
}

#[test]
fn offer_lookup_is_public_same_origin_data_and_tracks_offer_lifetime() {
    let h = Harness::new(FAST);
    assert_eq!(get(&h, "/sdk/pair-offer", "").status, 404);
    let mut first = open(&h);
    let answer = get(&h, "/sdk/pair-offer", &format!("Origin: {}\r\n", h.origin));
    assert_eq!(answer.status, 200);
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    assert_eq!(
        serde_json::from_str::<Value>(&answer.body).unwrap(),
        first.descriptor
    );
    assert!(!answer.body.contains("code"));
    assert_eq!(
        get(&h, "/sdk/pair-offer", "Origin: http://127.0.0.1:1\r\n").status,
        403
    );
    let mut second = open(&h);
    assert_eq!(first.owner.next()["reason"], "replaced");
    let current: Value = serde_json::from_str(&get(&h, "/sdk/pair-offer", "").body).unwrap();
    assert_eq!(current, second.descriptor);
    assert_ne!(current, first.descriptor);
    second.owner.answer("refuse");
    assert_eq!(second.owner.next()["reason"], "refused");
    assert_eq!(get(&h, "/sdk/pair-offer", "").status, 404);
    let bootstrap = get(&h, "/sdk/pair.js", "");
    assert_eq!(bootstrap.status, 200);
    assert_eq!(bootstrap.body, include_str!("../assets/pair.js"));
    assert!(
        bootstrap.body.find("history.replaceState").unwrap()
            < bootstrap.body.find("import(").unwrap()
    );
    assert_eq!(get(&h, "/pair/old", "").status, 404);
}

#[test]
fn mount_lookup_answers_from_the_door_mapping_for_same_origin_pages() {
    let h = Harness::new(FAST);
    let origin = Some(h.origin.as_str());
    let colab = format!("{}/x/colab/", h.prefix);
    let body = json!({ "path": format!("{colab}space/1") }).to_string();
    let answer: Value = serde_json::from_str(&mount(&h, &body, origin).body).unwrap();
    assert_eq!(
        answer,
        json!({
            "machineId": h.machine_id,
            "windowId": h.window_id,
            "address": format!("{}{}", h.origin, h.prefix),
            "extension": "colab",
            "mount": colab,
        })
    );
    let other = format!("{}/x/other/", h.prefix);
    let bare = format!("{}/x/colab", h.prefix);
    for path in ["/pair/abc", "/x/colab/", &other, &bare, "/"] {
        let body = json!({ "path": path }).to_string();
        let answer: Value = serde_json::from_str(&mount(&h, &body, origin).body).unwrap();
        assert_eq!(answer["extension"], Value::Null, "{path}");
        assert_eq!(answer["mount"], Value::Null, "{path}");
    }
    assert_eq!(mount(&h, r#"{"path":"/x/colab/"}"#, None).status, 403);
    assert_eq!(
        mount(&h, r#"{"path":"/x/colab/"}"#, Some("http://127.0.0.1:1")).status,
        403
    );
    for body in [
        r#"{"path":"/x/colab/","extra":1}"#,
        r#"{"path":1}"#,
        "[]",
        "x",
    ] {
        assert_eq!(mount(&h, body, origin).status, 400, "{body}");
    }
}

#[test]
fn static_pages_and_styles_are_exact_and_refusals_remain_generic() {
    let h = Harness::new(FAST);
    let landing = get(&h, "/", "");
    assert_eq!(landing.status, 200);
    assert_eq!(landing.body, include_str!("../assets/landing.html"));
    assert!(landing.body.contains("Remote <span>tmt</span>"));
    assert!(landing.body.contains("aria-hidden=\"true\">○</span>"));
    assert_eq!(
        landing.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    let policy = landing.header("content-security-policy").unwrap();
    assert!(policy.contains("style-src 'self'"));
    let css = get(&h, "/sdk/pages.css", "");
    assert_eq!(css.status, 200);
    assert_eq!(css.body, include_str!("../assets/pages.css"));
    assert_eq!(css.header("content-type"), Some("text/css; charset=utf-8"));
    assert_eq!(css.header("cache-control"), Some("no-store"));
    assert_eq!(css.header("x-content-type-options"), Some("nosniff"));
    for (path, headers, status) in [
        ("/pair/", "", 404),
        ("/pair/a.b", "", 404),
        ("/", "Origin: http://127.0.0.1:1\r\n", 403),
        ("/pair", "Origin: http://127.0.0.1:1\r\n", 403),
    ] {
        let reply = get(&h, path, headers);
        assert_eq!(reply.status, status);
        assert_eq!(
            reply.header("content-type"),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(reply.header("content-security-policy"), Some(policy));
        assert_eq!(reply.body, include_str!("../assets/error.html"));
        assert!(reply.body.contains("aria-hidden=\"true\">✗</span>"));
        // No cause crosses the generic refusal boundary; never infer expiry or reuse.
        assert!(reply.body.contains("This page is unavailable"));
        assert!(!reply.body.contains("has expired"));
        assert!(!reply.body.contains("was already used"));
    }
    for path in ["/sdk/unknown", "/x/colab/", &format!("{}/append", h.prefix)] {
        let reply = get(&h, path, "");
        assert_eq!(reply.status, 404);
        assert_eq!(reply.header("content-type"), Some("application/json"));
        assert_eq!(reply.body, "{}");
    }
    let refused = get(&h, "/sdk/pages.css", "Origin: http://127.0.0.1:1\r\n");
    assert_eq!(refused.status, 403);
    assert_eq!(refused.body, "{}");
}

#[test]
fn page_palette_and_font_stacks_match_the_shared_token_owner() {
    let tokens: Value =
        serde_json::from_str(include_str!("../../../../../design/tokens/tokens.json")).unwrap();
    let css = include_str!("../assets/pages.css");
    let (light, dark) = css
        .split_once("@media (prefers-color-scheme: dark)")
        .unwrap();
    for (theme, section) in [("light", light), ("dark", dark)] {
        for group in ["color", "surface"] {
            for (name, token) in tokens[group].as_object().unwrap() {
                if css.contains(&format!("--c-{name}:")) {
                    assert!(
                        section.contains(&format!(
                            "--c-{name}: {};",
                            token[theme].as_str().unwrap().to_ascii_lowercase()
                        )),
                        "{theme} {name}"
                    );
                }
            }
        }
    }
    for name in ["display", "body", "mono"] {
        assert!(css.contains(&format!(
            "--f-{name}: {};",
            tokens["font"][name]["stack"].as_str().unwrap().replace('"', "'")
        )));
    }
}
