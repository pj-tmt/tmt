//! The standalone copy of a page that opts in to the bundled runtime (#1693).
//!
//! `page.html` is the exact admitted source and never changes. For a page whose source asks for
//! the runtime, the export adds `page.standalone.html`: the same document the renderer frame
//! builds, with the runtime's script and theme written into its head, so the page works when it is
//! opened from disk with no Colab and no network. The browser build writes the same bytes
//! (`typescript/app/src/standalone.ts`); `vectors/export-v1.json` pins both.

use crate::{assets::RENDERER_POLICY, page_check};
use serde::Serialize;

/// The published name of the derived file.
pub const FILE: &str = "page.standalone.html";
/// The file it is derived from, as the manifest records it.
pub const DERIVED_FROM: &str = "page.html";
/// A marker whose content has any of these space-separated tokens asks for the runtime (the
/// runtime is one bundle, so either loads all of it); unknown tokens are ignored.
const TOKENS: [&str; 2] = ["react", "tailwind"];
const MARKER: &str = "tmt-colab-runtime";

/// The theme follows `data-theme` only, so the copy sets it from the system colour scheme and
/// follows changes, the way Colab's own pages follow the app theme. The browser build holds the
/// same text.
pub const THEME_SWITCH: &str = "(()=>{const q=matchMedia('(prefers-color-scheme: dark)'),r=document.documentElement,s=()=>{r.dataset.theme=q.matches?'dark':'light'};s();q.addEventListener('change',s)})()";

/// Why a page that asks for the runtime got no standalone copy.
pub const NO_RUNTIME: &str = "runtime-unavailable";
pub const TOO_DEEP: &str = "page-too-deep";
pub const UNUSABLE: &str = "runtime-unusable";

/// The two texts the copy carries. They are the bytes of `assets/colab-runtime.js` and
/// `assets/colab-runtime.css` of the same build.
pub struct Runtime<'a> {
    pub script: &'a str,
    pub style: &'a str,
}

/// What became of the copy, reported next to the files; absent for a page that does not opt in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Outcome {
    pub included: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}

/// The copy's bytes, when it could be made, with its outcome.
pub struct Derived {
    pub outcome: Outcome,
    pub file: Option<Vec<u8>>,
}

/// One sentence for the human output; the JSON carries only the code.
#[must_use]
pub fn explanation(reason: &str) -> &'static str {
    match reason {
        NO_RUNTIME => "this build carries no page runtime",
        TOO_DEEP => "the page nests too deeply to read its runtime marker",
        UNUSABLE => "this build's page runtime cannot be written into an HTML file",
        _ => "unknown reason",
    }
}

/// Whether `source` asks for the runtime: the first `<meta name="tmt-colab-runtime">` in document
/// order, found by parsing, has a `content` with a known token. The same cases run in the app
/// (`vectors/runtime-marker-v1.json`).
pub fn wants_runtime(source: &str) -> Result<bool, page_check::TooDeep> {
    if !source.contains(MARKER) {
        return Ok(false);
    }
    Ok(page_check::runtime_marker(source)?.is_some_and(|content| {
        content
            .split_ascii_whitespace()
            .any(|token| TOKENS.contains(&token))
    }))
}

/// The renderer frame's policy without its `sandbox` directive, the one a `<meta>` cannot carry:
/// the copy gains no network when it is opened from disk, but it is not sandboxed.
#[must_use]
pub fn meta_policy() -> &'static str {
    RENDERER_POLICY
        .split_once("; sandbox")
        .map_or(RENDERER_POLICY, |(policy, _)| policy)
}

/// The copy's document. `None` when a runtime text could end its own element or open a comment,
/// which would let it escape the head block: the build already refuses both, so this is a guard.
#[must_use]
pub fn document(source: &str, runtime: &Runtime) -> Option<Vec<u8>> {
    let unsafe_in = |text: &str, closing: &str| {
        text.contains("<!--") || text.to_ascii_lowercase().contains(closing)
    };
    if unsafe_in(runtime.script, "</script") || unsafe_in(runtime.style, "</style") {
        return None;
    }
    Some(
        format!(
            "<!doctype html><html><head><meta http-equiv=\"Content-Security-Policy\" content=\"{policy}\">\
             <meta charset=\"utf-8\"><style type=\"text/tailwindcss\">{style}</style>\
             <script>{THEME_SWITCH}</script><script>{script}</script></head><body>{source}</body></html>",
            policy = meta_policy(),
            style = runtime.style,
            script = runtime.script,
        )
        .into_bytes(),
    )
}

/// The standalone copy of `source`, or why there is none; `None` for a page that does not opt in.
#[must_use]
pub fn derive(source: &str, runtime: Option<&Runtime>) -> Option<Derived> {
    let skipped = |reason| {
        Some(Derived {
            outcome: Outcome {
                included: false,
                reason: Some(reason),
            },
            file: None,
        })
    };
    match wants_runtime(source) {
        Ok(false) => None,
        Err(page_check::TooDeep) => skipped(TOO_DEEP),
        Ok(true) => match runtime {
            None => skipped(NO_RUNTIME),
            Some(runtime) => match document(source, runtime) {
                Some(file) => Some(Derived {
                    outcome: Outcome {
                        included: true,
                        reason: None,
                    },
                    file: Some(file),
                }),
                None => skipped(UNUSABLE),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPTED_IN: &str = r#"<meta name="tmt-colab-runtime" content="react"><p>λ</p>"#;
    const RUNTIME: Runtime = Runtime {
        script: "window.tmt={};",
        style: "@theme inline{}",
    };

    #[test]
    fn the_shared_marker_vectors_decide_the_opt_in() {
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../contracts/vectors/runtime-marker-v1.json"
        ))
        .unwrap();
        let cases = vectors["cases"].as_array().unwrap();
        assert!(cases.len() >= 29);
        for case in cases {
            assert_eq!(
                wants_runtime(case["source"].as_str().unwrap()),
                Ok(case["optsIn"].as_bool().unwrap()),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn a_marker_in_markup_that_a_document_parse_hides_still_follows_the_browser() {
        // <noscript> content is markup with scripting off, as in DOMParser; a <template>'s content
        // is a separate fragment that querySelector does not enter.
        let yes = |source: &str| wants_runtime(source) == Ok(true);
        assert!(yes(
            r#"<noscript><meta name="tmt-colab-runtime" content="react"></noscript>"#
        ));
        assert!(!yes(
            r#"<template><meta name="tmt-colab-runtime" content="react"></template>"#
        ));
        // A table foster-parents the meta out before the table; it is still the first element.
        assert!(yes(
            r#"<table><meta name="tmt-colab-runtime" content="tailwind"></table>"#
        ));
    }

    #[test]
    fn a_source_nested_past_the_parse_depth_cannot_be_judged_but_costs_nothing_without_the_marker()
    {
        let deep = "<div>".repeat(crate::limits::PAGE_DEPTH + 50);
        assert_eq!(wants_runtime(&deep), Ok(false));
        let marked = format!("{OPTED_IN}{deep}");
        assert_eq!(wants_runtime(&marked), Err(page_check::TooDeep));
        let derived = derive(&marked, Some(&RUNTIME)).unwrap();
        assert_eq!(derived.outcome.reason, Some(TOO_DEEP));
        assert!(derived.file.is_none());
    }

    #[test]
    fn the_meta_policy_is_the_frames_policy_without_what_a_meta_cannot_carry() {
        let policy = meta_policy();
        assert!(!policy.contains("sandbox"));
        assert_eq!(format!("{policy}; sandbox allow-scripts"), RENDERER_POLICY);
        for directive in [
            "default-src 'none'",
            "connect-src 'none'",
            "frame-src 'none'",
        ] {
            assert!(policy.contains(directive), "{directive}");
        }
    }

    #[test]
    fn the_document_is_the_frames_document_with_the_runtime_in_its_head() {
        let file = String::from_utf8(document(OPTED_IN, &RUNTIME).unwrap()).unwrap();
        assert_eq!(
            file,
            format!(
                "<!doctype html><html><head><meta http-equiv=\"Content-Security-Policy\" content=\"{}\"><meta charset=\"utf-8\"><style type=\"text/tailwindcss\">@theme inline{{}}</style><script>{THEME_SWITCH}</script><script>window.tmt={{}};</script></head><body>{OPTED_IN}</body></html>",
                meta_policy()
            )
        );
    }

    #[test]
    fn a_runtime_text_that_could_leave_its_element_is_refused() {
        for (script, style) in [
            ("a</script><b>", ""),
            ("a</SCRIPT >", ""),
            ("<!-- a", ""),
            ("", "a</style>"),
            ("", "a</STYLE >"),
            ("", "<!-- a"),
        ] {
            assert!(document(OPTED_IN, &Runtime { script, style }).is_none());
        }
    }

    #[test]
    fn a_page_that_does_not_ask_has_no_copy_and_one_that_does_gets_a_reason_or_the_file() {
        assert!(derive("<p>plain</p>", Some(&RUNTIME)).is_none());
        let none = derive(OPTED_IN, None).unwrap();
        assert_eq!(
            none.outcome,
            Outcome {
                included: false,
                reason: Some(NO_RUNTIME)
            }
        );
        assert!(none.file.is_none());
        let bad = derive(
            OPTED_IN,
            Some(&Runtime {
                script: "</script>",
                style: "",
            }),
        )
        .unwrap();
        assert_eq!(bad.outcome.reason, Some(UNUSABLE));
        let made = derive(OPTED_IN, Some(&RUNTIME)).unwrap();
        assert_eq!(
            made.outcome,
            Outcome {
                included: true,
                reason: None
            }
        );
        assert!(made.file.unwrap().starts_with(b"<!doctype html>"));
        for reason in [NO_RUNTIME, TOO_DEEP, UNUSABLE] {
            assert_ne!(explanation(reason), "unknown reason");
        }
    }
}
