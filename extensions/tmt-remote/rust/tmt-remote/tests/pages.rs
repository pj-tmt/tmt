//! Door-served browser assets on real sockets: the pairing page, the SDK
//! module and the mount lookup, each disjoint from the prefixed routes and mounts.
#[allow(dead_code)]
#[path = "support/door.rs"]
mod door;

use base64::{Engine, engine::general_purpose::STANDARD};
use door::*;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};
use tmt_remote::pairing::Timing;

const PAGE_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src data:; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
fn pairing_policy(body: &str) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let inline = body
        .split_once("<script>")
        .unwrap()
        .1
        .split_once("</script>")
        .unwrap()
        .0;
    let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(inline.as_bytes()));
    PAGE_CSP.replace(
        "script-src 'self';",
        &format!("script-src 'self' 'sha256-{hash}';"),
    )
}

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
/// The one TMT web app document every app route serves, and its policy.
const SHELL_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; worker-src 'self'; frame-src 'self'; base-uri 'none'; form-action 'none'; object-src 'none'; frame-ancestors 'none'";
/// An app route: the door answers the shell document; the route's markup is the page asset the
/// app adopts, so the markup checks below read that asset.
fn app_page(h: &Harness, path: &str, page: &str) -> Reply {
    let mut reply = get(h, path, "");
    assert_eq!(reply.status, 200, "{path}");
    assert_eq!(reply.body, include_str!("../assets/shell.html"), "{path}");
    assert_eq!(
        reply.header("content-security-policy"),
        Some(SHELL_CSP),
        "{path}"
    );
    reply.body = page.to_owned();
    reply
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
    let policy = pairing_policy(&page.body);
    assert_eq!(
        page.header("content-security-policy"),
        Some(policy.as_str())
    );
    assert_eq!(page.header("cache-control"), Some("no-store"));
    assert_eq!(
        page.body,
        include_str!("../assets/pair.html")
            .replace("__TMT_PAIR_BOOTSTRAP__", include_str!("../assets/pair.js"))
    );
    assert!(
        page.body
            .contains(r#"<link rel="stylesheet" href="/sdk/pages.css" />"#)
    );
    assert!(page.body.contains(r#"data-tmt-page="pair""#));
    assert!(page.body.contains(r#"id="comparison" role="region""#));
    assert!(page.body.contains(r#"aria-labelledby="words-label""#));
    assert!(
        page.body
            .contains(r#"id="status" role="status" tabindex="-1""#)
    );
    assert!(
        page.body.find("history.replaceState").unwrap()
            < page.body.find("rel=\"stylesheet\"").unwrap()
    );
    assert!(page.body.find("history.replaceState").unwrap() < page.body.find("import(").unwrap());
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
    for copy in [
        "This pairing was removed or has expired. To use Remote here, run tmt remote pair on the machine.",
        "This page's connection ended. Try again. Your pairing is unchanged.",
    ] {
        assert!(sdk.body.contains(copy), "{copy}");
    }
    assert!(sdk.body.contains("tmt-ui-code"));
    assert!(
        !sdk.body
            .contains("The pairing may have been removed on the machine.")
    );
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
    // The reader entry and the Colab routes of the one web app both belong to Colab's mount.
    for path in ["/read/Z9_-", "/colab", "/colab/", "/colab/pages/x?read=y"] {
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
        "/colab/a.b",
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
    let landing = app_page(&h, "/remote/", include_str!("../assets/landing.html"));
    assert!(landing.body.contains(
        r#"<header class="header tmt-ui-header" aria-label="Remote · Remote connection">"#
    ));
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
    assert!(SHELL_CSP.contains("style-src 'self'"));
    let policy = PAGE_CSP;
    let css = get(&h, "/sdk/pages.css", "");
    assert_eq!(css.status, 200);
    assert_eq!(
        css.body,
        concat!(
            include_str!("../../../../../design/browser-ui/generated/static.css"),
            "\n",
            include_str!("../assets/pages.css"),
            "\n",
            include_str!("../assets/tmt-settings.css"),
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
        assert!(reply.body.contains(
            "This page is unavailable. Open it again from the machine. Your pairing is unchanged."
        ));
        assert!(!reply.body.contains("tmt remote pair"));
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
        let standalone = page == include_str!("../assets/error.html");
        for class in [
            "tmt-ui-header",
            "tmt-ui-title",
            "tmt-ui-notice",
            "tmt-ui-notice-mark",
            "tmt-ui-notice-heading",
            "tmt-ui-notice-body",
        ]
        .into_iter()
        .chain(
            standalone
                .then_some(["tmt-ui-brand", "tmt-ui-mark", "tmt-ui-wordmark"])
                .into_iter()
                .flatten(),
        ) {
            assert!(classes.contains(&class), "missing shared consumer {class}");
            assert!(
                shared.contains(&format!(".{class}")),
                "missing shared export {class}"
            );
        }
        if page != include_str!("../assets/landing.html") {
            assert!(classes.contains(&"tmt-ui-notice-eyebrow"));
        }
        let title = page.find("header-title tmt-ui-title").unwrap();
        if standalone {
            // Outside the web shell the header carries the brand: mark, product, then the title.
            let mark = page.find("header-mark tmt-brand-mark tmt-ui-mark").unwrap();
            let product = page.find("header-wordmark tmt-ui-wordmark").unwrap();
            assert!(mark < product && product < title);
        } else {
            // In the web shell the rail names the module: the header carries the title alone.
            assert!(!page.contains("header-mark") && !page.contains("header-wordmark"));
        }
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
                // Theme-invariant roles inherit from :root; themed roles override them.
                let section = if token["light"] == token["dark"] {
                    light
                } else {
                    section
                };
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
        assert!(css.contains(&format!(
            "--tmt-ui-font-{name}: {};",
            tokens["font"][name]["stack"]
                .as_str()
                .unwrap()
                .replace('"', "'")
        )));
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
            // A short page link redirects into the app without reaching Colab.
            let reply = get(&h, &format!("/p/{id}"), &headers);
            assert_eq!(reply.status, 308, "{id}");
            assert_eq!(
                reply.header("location"),
                Some(format!("/colab/p/{id}").as_str())
            );
            assert_eq!(reply.header("cache-control"), Some("no-store"));
            assert_eq!(reply.header("referrer-policy"), Some("no-referrer"));
            assert_eq!(reply.header("set-cookie"), None);
            assert_eq!(reply.body, "");
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
            Some("text/html; charset=utf-8"),
            "{path}"
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
    for path in ["/p/ab%63d", "/p/abcd#fragment", "/p/../abcd", "/p//abcd"] {
        let reply = get(&h, path, "");
        assert_eq!(reply.status, 400, "{path}");
        assert_eq!(reply.body, "{}");
        assert_eq!(reply.header("location"), None);
    }
    // HEAD redirects like GET; other methods refuse.
    let head = exchange(
        &h,
        &format!("HEAD /p/abcd HTTP/1.1\r\nHost: {}\r\n\r\n", h.addr),
    );
    assert_eq!(head.status, 308);
    for method in ["POST", "PUT"] {
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
    let page = app_page(
        &h,
        "/remote/settings",
        include_str!("../assets/settings.html"),
    );
    assert!(
        page.body
            .contains("Recorded setup, not a live Firebase check.")
    );
    assert!(page.body.contains("id=\"firestore-content\""));
    assert!(page.body.contains("id=\"access-announcement\""));
    assert!(page.body.contains("id=\"firestore-announcement\""));
    assert!(
        !page
            .body
            .contains("id=\"firestore-content\" role=\"status\"")
    );
    assert!(!page.body.contains("id=\"access\" role=\"status\""));
    for origin in ["shared", "open", "sessions-per-device"] {
        assert!(page.body.contains(&format!("data-feedback=\"{origin}\"")));
    }
    assert_eq!(
        page.body
            .matches(
                "data-outcome-slot role=\"status\" aria-live=\"polite\" aria-atomic=\"true\"></p>"
            )
            .count(),
        3
    );
    assert!(page.body.contains("No changes to save."));
    assert!(page.body.contains("Check the saved value"));

    assert!(page.body.contains(
        "<p>“This device” marks the pairing this browser uses. It does not give access to settings.</p>"
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
    assert!(
        script
            .body
            .contains("Another change is still unconfirmed. Check the saved value first.")
    );
    assert!(
        !script
            .body
            .contains("Check the original result above before making another change.")
    );
    assert!(script.body.contains(
        "Browser change limit reached. Use the local CLI; do not retry or reset storage."
    ));
    assert!(
        !script
            .body
            .contains("The original outcome is unknown. Read it before another change.")
    );
    assert!(
        !script
            .body
            .contains("Browser management operation limit reached.")
    );
    assert_eq!(
        get(&h, "/remote/settings", "Origin: http://127.0.0.1:1\r\n").status,
        403
    );
    assert_eq!(get(&h, "/settings/extra", "").status, 404);
    assert_eq!(
        get(&h, "/sdk/settings.css", "").body,
        include_str!("../assets/settings.css")
    );
}

#[test]
fn settings_consume_shared_classes_and_static_listboxes_without_a_palette() {
    let h = Harness::new(FAST);
    let html = app_page(
        &h,
        "/remote/settings",
        include_str!("../assets/settings.html"),
    )
    .body;
    for class in [
        "tmt-ui-header",
        "tmt-ui-title",
        "tmt-ui-notice",
        "tmt-ui-section-heading",
        "tmt-ui-field-label",
        "tmt-ui-field-control",
        "tmt-ui-action-label",
    ] {
        assert!(html.contains(class), "missing {class}");
    }
    assert_eq!(html.matches("<h1 ").count(), 1);
    assert!(!html.contains("<select"));
    assert_eq!(html.matches("aria-haspopup=\"listbox\"").count(), 2);
    assert_eq!(html.matches("popover=\"auto\"").count(), 2);
    assert!(html.contains("id=\"opening\""));
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
fn remote_headers_carry_the_static_mark_only_outside_the_web_shell() {
    // The browser leaf's static-host copy of its header mark, with the header-mark class.
    let shared = include_str!("../../../../../design/browser-ui/generated/brand-mark-static.svg");
    let mark = shared.trim().replacen(
        r#"class="tmt-brand-mark tmt-ui-mark""#,
        r#"class="header-mark tmt-brand-mark tmt-ui-mark""#,
        1,
    );
    let normalize = |markup: &str| {
        markup
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .replace(" >", ">")
            .replace("> <", "><")
    };
    let expected = normalize(&mark);
    // The refusal page is the one Remote page outside the web shell: it keeps the brand.
    let error = include_str!("../assets/error.html");
    assert!(normalize(error).contains(&expected));
    assert_eq!(error.matches("class=\"header-mark ").count(), 1);
    assert!(error.contains("class=\"header-wordmark tmt-ui-wordmark\">Remote</span>"));
    // Every other page renders inside the web shell, where the rail's head carries the brand and
    // the rail names the module: a header carries the page title alone. A Remote page whose title
    // does not say Remote names the module in the header's accessible name.
    for (page, name) in [
        (
            include_str!("../assets/landing.html"),
            Some("Remote · Remote connection"),
        ),
        (
            include_str!("../assets/pair.html"),
            Some("Remote · Pair a browser"),
        ),
        (
            include_str!("../assets/settings.html"),
            Some("Remote · Remote settings"),
        ),
        (include_str!("../assets/ops.html"), None),
        (include_str!("../assets/home.html"), None),
        (include_str!("../assets/tmt-settings.html"), None),
        (include_str!("../assets/digest.html"), None),
    ] {
        assert!(!page.contains("header-mark"));
        assert!(!page.contains("header-wordmark"));
        assert!(!page.contains("header-divider"));
        assert!(page.contains("class=\"header-title tmt-ui-title\">"));
        match name {
            Some(name) => assert!(page.contains(&format!(
                "<header class=\"header tmt-ui-header\" aria-label=\"{name}\">"
            ))),
            None => assert!(page.contains("<header class=\"header tmt-ui-header\">")),
        }
    }
}

#[test]
fn every_door_html_page_serves_the_shared_favicon_with_the_exact_data_only_policy() {
    let h = Harness::new(FAST);
    for (path, status) in [
        ("/", 200),
        ("/remote/pair", 200),
        ("/remote/", 200),
        ("/settings", 200),
        ("/settings/", 200),
        ("/remote/devices", 200),
        ("/remote/connect", 200),
        ("/remote/machine", 200),
        ("/remote/settings", 200),
        ("/pair/", 404),
    ] {
        let reply = get(&h, path, "");
        assert_eq!(reply.status, status, "{path}");
        assert!(
            reply.body.find("<meta charset").is_some_and(|at| at < 1024),
            "{path}: charset declaration must start within the first 1024 bytes"
        );
        let links: Vec<_> = reply
            .body
            .split("<link")
            .skip(1)
            .map(|tail| tail.split('>').next().unwrap())
            .filter(|tag| tag.contains("rel=\"icon\""))
            .collect();
        assert_eq!(links.len(), 1, "{path}");
        let policy = if path == "/remote/pair" {
            pairing_policy(&reply.body)
        } else if status == 200 {
            SHELL_CSP.to_owned()
        } else {
            PAGE_CSP.to_owned()
        };
        assert_eq!(
            reply.header("content-security-policy"),
            Some(policy.as_str()),
            "{path}"
        );
        let link = links[0];
        let href = link
            .split("href=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let bytes = STANDARD
            .decode(href.strip_prefix("data:image/svg+xml;base64,").unwrap())
            .unwrap();
        assert_eq!(
            bytes,
            include_bytes!("../../../../../design/browser-ui/generated/favicon.svg"),
            "{path}"
        );
    }
}

#[test]
fn door_routes_legacy_pair_redirect_is_fixed_and_admitted_before_location() {
    let h = Harness::new(FAST);
    for method in ["GET", "HEAD"] {
        for path in [
            "/pair",
            "/pair?next=https://example.com/",
            "/pair?next=%2F%2Fevil.test",
        ] {
            let reply = exchange(
                &h,
                &format!("{method} {path} HTTP/1.1\r\nHost: {}\r\n\r\n", h.addr),
            );
            assert_eq!(reply.status, 308, "{method} {path}");
            assert_eq!(reply.header("location"), Some("/remote/pair"));
            assert_eq!(reply.header("cache-control"), Some("no-store"));
            assert_eq!(reply.header("referrer-policy"), Some("no-referrer"));
            assert_eq!(reply.header("content-security-policy"), Some(PAGE_CSP));
            assert!(reply.body.is_empty());
        }
    }
    for headers in [
        "Origin: https://example.com\r\n",
        "Sec-Fetch-Site: cross-site\r\n",
    ] {
        let reply = get(&h, "/pair?next=https://example.com/", headers);
        assert_eq!(reply.status, 403);
        assert_eq!(reply.header("location"), None);
    }
    let reply = exchange(&h, "GET /pair HTTP/1.1\r\nHost: example.com\r\n\r\n");
    assert_eq!(reply.status, 400);
    assert_eq!(reply.header("location"), None);
    let reply = exchange(
        &h,
        &format!("POST /pair HTTP/1.1\r\nHost: {}\r\n\r\n", h.addr),
    );
    assert_eq!(reply.status, 404);
    assert_eq!(reply.header("location"), None);
    for path in [
        "/pair/",
        "/remote/pair/",
        "/settings?next=other",
        "/remote/pair?next=other",
    ] {
        assert_ne!(get(&h, path, "").status, 308, "{path}");
    }
}

#[test]
fn door_routes_app_routes_keep_the_app_plain_query() {
    let h = Harness::new(FAST);
    // A short link's query follows its redirect to an app route, which serves the shell with it.
    let short = get(&h, "/p/abcd?open=1", "");
    assert_eq!(short.status, 308);
    let location = short.header("location").unwrap().to_owned();
    assert_eq!(location, "/colab/p/abcd?open=1");
    app_page(&h, &location, "");
    // The app writes its own keys onto its routes; a reload asks the door for the same address.
    for path in [
        "/colab/pages/abcd?read=x",
        "/colab/pages/abcd?anchor=a-1&read=r_2",
        "/settings?tab=one",
    ] {
        app_page(&h, path, "");
    }
    // The same plain byte set as a short link: no escapes, session key or other separators.
    for path in [
        "/colab/pages/abcd?read=%2F",
        "/colab/pages/abcd?tmt-session=x",
        "/colab/pages/abcd?read=x&tmt-session=y",
        "/colab/pages/abcd?read=a/b",
        "/colab/pages/abcd?",
        "/home?read=x",
        "/colab/a.b?read=x",
    ] {
        assert_ne!(get(&h, path, "").status, 200, "{path}");
    }
    // A key named like the session hand-off inside a value is plain text, not the hand-off.
    app_page(&h, "/colab/pages/abcd?read=tmt-session", "");
    // Mount paths keep their strict grammar: an app key on a module mount is a malformed target.
    for path in [
        format!("{}/x/colab/?read=a", h.prefix),
        format!("{}/x/colab/pages/abcd?open=a", h.prefix),
        "/x/colab/p/abcd?open=a".to_owned(),
    ] {
        assert_eq!(get(&h, &path, "").status, 400, "{path}");
    }
}

#[test]
fn door_routes_home_and_tmt_settings_load_only_the_shell_and_remote_pages_move() {
    let h = Harness::new(FAST);
    // Every app route is the same shell document; the rail and Home's rows come from its script.
    for path in [
        "/",
        "/settings",
        "/settings/",
        "/remote",
        "/remote/",
        "/ops",
        "/digest/",
        "/colab/pages/x",
    ] {
        let shell = app_page(&h, path, include_str!("../assets/shell.html"));
        assert_eq!(shell.header("set-cookie"), None, "{path}");
        assert_eq!(shell.body.matches("<script").count(), 1);
        assert!(
            shell
                .body
                .contains("<script type=\"module\" src=\"/sdk/shell.js\"></script>")
        );
        assert!(shell.body.contains("class=\"tmt-ui-shell\""));
        assert!(!shell.body.contains(&h.prefix));
    }
    // The explicit list never becomes a catch-all.
    for path in ["/home", "/ops/x", "/colab/a.b", "/remote/other"] {
        assert_eq!(get(&h, path, "").status, 404, "{path}");
    }
    let home = app_page(&h, "/", include_str!("../assets/home.html"));
    assert!(home.body.contains("id=\"home-needs\""));
    let settings = app_page(&h, "/settings", include_str!("../assets/tmt-settings.html"));
    assert!(settings.body.contains(">TMT settings</h1>"));
    let entry = get(&h, "/sdk/shell.js", "");
    assert_eq!(entry.status, 200);
    assert_eq!(entry.body, include_str!("../assets/shell.js"));
    assert_eq!(entry.header("cache-control"), Some("no-store"));
    assert_eq!(
        entry.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(
        get(&h, "/sdk/shell.js", "Origin: https://example.com\r\n").status,
        403
    );
    let pair = get(&h, "/remote/pair", "");
    assert_eq!(pair.status, 200);
    assert!(!pair.body.contains("__TMT_PAIR_BOOTSTRAP__"));
    assert!(pair.body.contains("history.replaceState"));
    assert!(include_str!("../assets/landing.html").contains("Remote connection"));
    assert!(include_str!("../assets/settings.html").contains("Browser opening"));
}
