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
    assert!(page.body.contains(
        r#"<button class="tmt-ui-action" type="submit" aria-describedby="status" disabled>"#
    ));
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
    for path in ["/colab", "/colab/", "/p/aB_0", "/read/Z9_-"] {
        let answer: Value =
            serde_json::from_str(&mount(&h, &json!({"path":path}).to_string(), origin).body)
                .unwrap();
        assert_eq!(
            answer,
            json!({"machineId":h.machine_id,"windowId":h.window_id,"address":format!("{}{}",h.origin,h.prefix),"extension":"colab","mount":colab})
        );
    }
    let other = format!("{}/x/other/", h.prefix);
    let bare = format!("{}/x/colab", h.prefix);
    for path in [
        "/pair/abc",
        "/x/colab/",
        &other,
        &bare,
        "/",
        "/colab/other",
        "/p/abc",
        "/read/abcd/",
        "/p/abcd?next=1",
        "/read/abcd#secret",
        "/r/abcd",
        "/p/.tmt",
    ] {
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
    assert!(
        landing
            .body
            .contains(r#"class="header-wordmark tmt-ui-wordmark">Remote</span>"#)
    );
    assert!(landing.body.contains("aria-hidden=\"true\">◌</span"));
    assert!(landing.body.contains("/sdk/landing.js"));
    for text in [
        "checked-time",
        "Check again",
        "tmt remote pair",
        "four words",
        "Confirm in the terminal",
    ] {
        assert!(landing.body.contains(text), "{text}");
    }
    for removed in ["command-pair", "command-devices", "command-status"] {
        assert!(!landing.body.contains(&format!("id=\"{removed}\"")));
    }
    for (state, commands) in [
        ("missing", vec!["pair"]),
        ("different", vec!["pair"]),
        ("refused", vec!["devices", "pair"]),
        ("unconfirmed", vec!["status"]),
        ("unreadable", vec!["pair"]),
    ] {
        let steps = landing
            .body
            .split(&format!("id=\"steps-{state}\""))
            .nth(1)
            .unwrap()
            .split("</ol>")
            .next()
            .unwrap();
        assert_eq!(
            steps
                .matches("<code class=\"tmt-ui-command-text\">")
                .count(),
            commands.len()
        );
        assert_eq!(
            steps
                .matches("class=\"entry-command tmt-ui-command\"")
                .count(),
            commands.len()
        );
        for closing in steps.split("</span").skip(1) {
            assert!(
                closing
                    .trim_start()
                    .strip_prefix('>')
                    .is_some_and(|after| { !after.trim_start().starts_with('.') })
            );
        }
        assert_eq!(
            steps.matches("class=\"tmt-ui-command-copy\"").count(),
            commands.len()
        );
        for command in commands {
            assert_eq!(
                steps
                    .matches(&format!(
                        "<code class=\"tmt-ui-command-text\">tmt remote {command}</code>"
                    ))
                    .count(),
                1
            );
            assert!(steps.contains(&format!("id=\"copy-{state}-{command}\"")));
            assert_eq!(
                steps
                    .matches(&format!("aria-label=\"Copy tmt remote {command}\""))
                    .count(),
                1
            );
        }
    }
    assert!(
        landing.body.find("id=\"command-location\"").unwrap()
            < landing.body.find("id=\"steps-missing\"").unwrap()
    );
    assert!(
        landing.body.find("id=\"steps-missing\"").unwrap()
            < landing.body.find("id=\"pairing-note\"").unwrap()
    );
    let entry = get(&h, "/sdk/landing.js", "");
    assert_eq!(entry.status, 200);
    assert_eq!(entry.body, include_str!("../assets/landing.js"));
    assert_eq!(entry.header("cache-control"), Some("no-store"));
    assert_eq!(
        entry.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(
        get(&h, "/sdk/landing.js", "Origin: https://example.com\r\n").status,
        403
    );
    assert_eq!(
        h.grants(),
        0,
        "static entry never enrolls or admits a browser"
    );
    assert_eq!(
        landing.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    let policy = landing.header("content-security-policy").unwrap();
    assert!(policy.contains("style-src 'self'"));
    let css = get(&h, "/sdk/pages.css", "");
    assert_eq!(css.status, 200);
    assert_eq!(
        css.body,
        concat!(
            include_str!("../../../../../design/browser-ui/generated/static.css"),
            "\n",
            include_str!("../assets/pages.css"),
        )
    );
    assert!(!css.body.contains("@import"));
    assert!(!css.body.contains("url("));
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
        assert!(reply.body.contains("aria-hidden=\"true\">✗</span"));
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
fn pages_consume_shared_presentation_without_a_second_projection() {
    let shared = include_str!("../../../../../design/browser-ui/generated/static.css");
    let host = include_str!("../assets/pages.css");
    for obsolete in ["--c-", "--f-", "--header-", "box-shadow:", "opacity:"] {
        assert!(
            !host.contains(obsolete),
            "duplicate presentation: {obsolete}"
        );
    }
    for page in [
        include_str!("../assets/landing.html"),
        include_str!("../assets/pair.html"),
        include_str!("../assets/error.html"),
    ] {
        let classes = page
            .split("class=\"")
            .skip(1)
            .filter_map(|attribute| attribute.split_once('"').map(|(value, _)| value))
            .flat_map(str::split_whitespace)
            .collect::<Vec<_>>();
        for class in [
            "tmt-ui-header",
            "tmt-ui-brand",
            "tmt-ui-mark",
            "tmt-ui-wordmark",
            "tmt-ui-title",
            "tmt-ui-notice",
            "tmt-ui-notice-mark",
            "tmt-ui-notice-heading",
            "tmt-ui-notice-body",
        ] {
            assert!(classes.contains(&class), "missing shared consumer {class}");
            assert!(
                shared.contains(&format!(".{class}")),
                "missing shared export {class}"
            );
        }
        if page != include_str!("../assets/landing.html") {
            assert!(classes.contains(&"tmt-ui-notice-eyebrow"));
        }
        let mark = page.find("header-mark tmt-ui-mark").unwrap();
        let product = page.find("header-wordmark tmt-ui-wordmark").unwrap();
        let title = page.find("header-title tmt-ui-title").unwrap();
        assert!(mark < product && product < title);
        assert_eq!(page.matches("<h1").count(), 1);
        assert!(page.contains("id=\"state-label\""));
        assert!(!page.contains("<style"));
    }
    let pair = include_str!("../assets/pair.html")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(pair.contains("class=\"tmt-ui-field-control\" aria-labelledby=\"name-label\""));
    assert!(pair.contains("id=\"name-label\" class=\"tmt-ui-field-label\" for=\"name\""));
    assert!(pair.contains("aria-describedby=\"status\" disabled"));
}

#[test]
fn embedded_shared_css_matches_browser_tokens_fonts_and_header_metrics() {
    let tokens: Value =
        serde_json::from_str(include_str!("../../../../../design/tokens/tokens.json")).unwrap();
    let css = include_str!("../../../../../design/browser-ui/generated/static.css");
    let (light, dark) = css
        .split_once("@media (prefers-color-scheme: dark)")
        .unwrap();
    for (theme, section) in [("light", light), ("dark", dark)] {
        for group in ["color", "surface"] {
            for (name, token) in tokens["browser"][group].as_object().unwrap() {
                assert!(
                    section.contains(&format!(
                        "--tmt-ui-{group}-{name}: {};",
                        token[theme].as_str().unwrap().to_ascii_lowercase()
                    )),
                    "{theme} {group} {name}"
                );
            }
        }
    }
    for (name, value) in tokens["header"].as_object().unwrap() {
        if name == "compact-max-width" {
            assert!(css.contains(&format!("@media (max-width: {})", value.as_str().unwrap())));
        } else {
            assert!(css.contains(&format!(
                "--tmt-ui-header-{name}: {};",
                value.as_str().unwrap()
            )));
        }
    }
    for name in ["display", "body", "mono"] {
        assert!(css.contains(&format!("--tmt-ui-font-{name}: {};",
            tokens["font"][name]["stack"].as_str().unwrap().replace('"', "'"))));
    }
}

#[test]
fn short_public_entry_without_colab_returns_a_generic_page_without_cookies() {
    let h = Harness::new(FAST);
    for id in ["aB_0".to_owned(), "74f92bf5".to_owned(), "Z9_-".repeat(16)] {
        for headers in [
            String::new(),
            format!("Origin: {}\r\n", h.origin),
            "Cookie: tmt_door=untrusted\r\n".to_owned(),
        ] {
            let reply = get(&h, &format!("/p/{id}"), &headers);
            assert_eq!(reply.status, 404, "{id}");
            assert_eq!(reply.header("location"), None);
            assert_eq!(reply.header("cache-control"), Some("no-store"));
            assert_eq!(reply.header("referrer-policy"), Some("no-referrer"));
            assert_eq!(reply.header("set-cookie"), None);
            assert_eq!(reply.body, include_str!("../assets/error.html"));
        }
    }
    assert_eq!(h.grants(), 0);
}

#[test]
fn malformed_short_page_aliases_return_the_generic_page_404() {
    let h = Harness::new(FAST);
    for path in [
        "/p".to_owned(),
        "/p/".to_owned(),
        "/p/abc".to_owned(),
        format!("/p/{}", "a".repeat(65)),
        "/p/abcd/".to_owned(),
        "/p/abcd/efgh".to_owned(),
        "/p/abcd.efgh".to_owned(),
        "/p/abcd:efgh".to_owned(),
        "/p/éabc".to_owned(),
    ] {
        let reply = get(&h, &path, "");
        assert_eq!(reply.status, 404, "{path}");
        assert_eq!(
            reply.header("content-type"),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(reply.body, include_str!("../assets/error.html"));
        assert_eq!(reply.header("location"), None);
        assert_eq!(reply.header("set-cookie"), None);
        assert!(!reply.body.contains(&h.prefix));
    }
    let upgrade = get(
        &h,
        "/p/abcd",
        "Connection: Upgrade\r\nUpgrade: websocket\r\n",
    );
    assert_eq!(upgrade.status, 404);
    assert_eq!(upgrade.body, include_str!("../assets/error.html"));
    assert_eq!(upgrade.header("location"), None);
    // Invalid HTTP targets still fail at the shared framing boundary.
    for path in [
        "/p/ab%63d",
        "/p/abcd?next=other",
        "/p/abcd#fragment",
        "/p/../abcd",
        "/p//abcd",
    ] {
        let reply = get(&h, path, "");
        assert_eq!(reply.status, 400, "{path}");
        assert_eq!(reply.body, "{}");
        assert_eq!(reply.header("location"), None);
    }
    for method in ["POST", "HEAD"] {
        let reply = exchange(
            &h,
            &format!("{method} /p/abcd HTTP/1.1\r\nHost: {}\r\n\r\n", h.addr),
        );
        assert_eq!(reply.status, 404);
        assert_eq!(reply.body, include_str!("../assets/error.html"));
        assert_eq!(reply.header("location"), None);
    }
}

#[test]
fn short_public_entry_refuses_cross_origin_requests_before_forwarding() {
    let h = Harness::new(FAST);
    for origin in ["http://127.0.0.1:1", "https://example.com", "null"] {
        let reply = get(
            &h,
            "/p/abcd",
            &format!("Origin: {origin}\r\nCookie: tmt_door=untrusted\r\n"),
        );
        assert_eq!(reply.status, 403, "{origin}");
        assert_eq!(reply.body, include_str!("../assets/error.html"));
        assert_eq!(reply.header("location"), None);
        assert_eq!(reply.header("access-control-allow-origin"), None);
        assert_eq!(reply.header("set-cookie"), None);
        assert!(!reply.body.contains(&h.prefix));
    }
}

#[test]
fn settings_page_and_product_script_reuse_the_same_door_policy_and_sdk() {
    let h = Harness::new(FAST);
    let page = get(&h, "/settings", "");
    assert_eq!(page.status, 200);
    assert_eq!(page.body, include_str!("../assets/settings.html"));
    assert!(
        page.body
            .contains("Recorded setup, not a live Firebase check.")
    );
    assert!(page.body.contains("id=\"firestore-content\""));
    assert!(page.body.contains(
        "<p>“This device” identifies a pairing. It does not grant settings authority.</p>"
    ));
    assert_eq!(
        page.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert!(
        page.header("content-security-policy")
            .unwrap()
            .contains("script-src 'self'")
    );
    assert_eq!(page.header("cache-control"), Some("no-store"));
    let script = get(&h, "/sdk/settings-v1.js", "");
    assert_eq!(script.body, include_str!("../assets/settings-v1.js"));
    assert!(script.body.contains("from \"/sdk/remote-v1.js\""));
    assert!(!script.body.contains("class DeviceKey"));
    assert_eq!(
        get(&h, "/settings", "Origin: http://127.0.0.1:1\r\n").status,
        403
    );
    assert_eq!(get(&h, "/settings/extra", "").status, 404);
    assert_eq!(
        get(&h, "/sdk/settings.css", "").body,
        include_str!("../assets/settings.css")
    );
}

#[test]
fn settings_consume_shared_classes_and_native_selects_without_a_palette() {
    let h = Harness::new(FAST);
    let html = get(&h, "/settings", "").body;
    for class in [
        "tmt-ui-header",
        "tmt-ui-brand",
        "tmt-ui-mark",
        "tmt-ui-wordmark",
        "tmt-ui-title",
        "tmt-ui-notice",
        "tmt-ui-notice-heading",
        "tmt-ui-field-label",
        "tmt-ui-field-control",
        "tmt-ui-action-label",
    ] {
        assert!(html.contains(class), "missing {class}");
    }
    assert_eq!(html.matches("<h1 ").count(), 1);
    assert!(html.contains("<select") && html.contains("id=\"opening\""));
    assert!(html.contains("aria-labelledby=\"limit-custom-label\""));
    let css = get(&h, "/sdk/settings.css", "").body;
    for legacy in [
        "--c-",
        "--f-",
        "--header-",
        "opacity:",
        "box-shadow:",
        "@import",
        "url(",
    ] {
        assert!(!css.contains(legacy), "second presentation owner: {legacy}");
    }
    assert!(
        !css.split_whitespace()
            .any(|word| word.starts_with('#') && word.ends_with(';')),
        "host CSS must not declare colors"
    );
}

#[test]
fn remote_static_headers_pin_the_approved_aperture_mark() {
    // Approved static-host projection of design/browser-ui/src/header.tsx (#2207).
    // This literal adds no native input dependency on the leaf's source module.
    const MARK: &str = r#"<svg class="header-mark tmt-ui-mark" viewBox="0 0 200 200" aria-hidden="true" fill="currentColor">
          <g transform="rotate(0 100 100)"><path d="M100 18A82 82 0 0 1 164 49C143 45 117 55 110 77L92 75C85 50 87 31 100 18Z" /></g>
          <g transform="rotate(60 100 100)"><path d="M100 18A82 82 0 0 1 164 49C143 45 117 55 110 77L92 75C85 50 87 31 100 18Z" /></g>
          <g transform="rotate(120 100 100)"><path d="M100 18A82 82 0 0 1 164 49C143 45 117 55 110 77L92 75C85 50 87 31 100 18Z" /></g>
          <g transform="rotate(180 100 100)"><path d="M100 18A82 82 0 0 1 164 49C143 45 117 55 110 77L92 75C85 50 87 31 100 18Z" /></g>
          <g transform="rotate(240 100 100)"><path d="M100 18A82 82 0 0 1 164 49C143 45 117 55 110 77L92 75C85 50 87 31 100 18Z" /></g>
          <g transform="rotate(300 100 100)"><path d="M100 18A82 82 0 0 1 164 49C143 45 117 55 110 77L92 75C85 50 87 31 100 18Z" /></g>
        </svg>"#;
    let normalize = |markup: &str| {
        markup
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace(" >", ">")
            .replace("> <", "><")
    };
    let expected = normalize(MARK);
    for page in [
        include_str!("../assets/landing.html"),
        include_str!("../assets/pair.html"),
        include_str!("../assets/error.html"),
        include_str!("../assets/settings.html"),
    ] {
        let normalized = normalize(page);
        assert!(normalized.contains(&expected));
        assert_eq!(page.matches("class=\"header-mark tmt-ui-mark\"").count(), 1);
        assert!(page.contains("class=\"header-wordmark tmt-ui-wordmark\">Remote</span>"));
    }
}
