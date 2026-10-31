//! The standalone copy of a page that opts in to the bundled runtime (#1693) or uses the design
//! system (#5057).
//!
//! `page.html` is the exact admitted source and never changes. For a page whose source asks for the
//! runtime or uses the design system, the export adds `page.standalone.html`: the same document the
//! renderer frame builds, with the runtime's and the design system's scripts and styles written
//! into its head, so the page looks and works when it is opened from disk with no Colab and no
//! network. The browser build writes the same bytes (`typescript/app/src/standalone.ts`);
//! `vectors/export-v1.json` pins both.

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

/// Why a page that needs a copy got none.
pub const NO_RUNTIME: &str = "runtime-unavailable";
pub const TOO_DEEP: &str = "page-too-deep";
pub const UNUSABLE: &str = "runtime-unusable";
pub const NO_DESIGN: &str = "design-unavailable";
pub const DESIGN_UNUSABLE: &str = "design-unusable";

/// The two texts the page runtime adds. They are the bytes of `assets/colab-runtime.js` and
/// `assets/colab-runtime.css` of the same build.
#[derive(Clone, Copy)]
pub struct Runtime<'a> {
    pub script: &'a str,
    pub style: &'a str,
}

/// The two texts the design system adds (#5057): the bytes of `assets/colab-page-ui.js` and
/// `assets/colab-page-ui.css` of the same build, which are what the renderer frame carries.
#[derive(Clone, Copy)]
pub struct Design<'a> {
    pub script: &'a str,
    pub style: &'a str,
}

/// What the build has to offer a copy; each is absent when the build lacks its files.
#[derive(Clone, Copy, Default)]
pub struct Assets<'a> {
    pub runtime: Option<Runtime<'a>>,
    pub design: Option<Design<'a>>,
}

/// What became of the copy, reported next to the files; absent for a page that needs none.
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
        NO_DESIGN => "this build carries no page design system",
        DESIGN_UNUSABLE => "this build's page design system cannot be written into an HTML file",
        _ => "unknown reason",
    }
}

/// A registered HTML widget asks for the runtime automatically. Otherwise the first runtime
/// meta's known token opts in. Browser and native selection share `runtime-marker-v1.json`.
pub fn wants_runtime(source: &str) -> Result<bool, page_check::TooDeep> {
    if !source.contains(MARKER) && !source.to_ascii_lowercase().contains("<tmt-ui-") {
        return Ok(false);
    }
    Ok(
        page_check::runtime_requirement(source)?.is_some_and(|content| {
            content
                .split_ascii_whitespace()
                .any(|token| TOKENS.contains(&token))
        }),
    )
}

pub use crate::page_check::uses_design;

/// The renderer frame's policy without its `sandbox` directive, the one a `<meta>` cannot carry:
/// the copy gains no network when it is opened from disk, but it is not sandboxed.
#[must_use]
pub fn meta_policy() -> &'static str {
    RENDERER_POLICY
        .split_once("; sandbox")
        .map_or(RENDERER_POLICY, |(policy, _)| policy)
}

/// The copy's document, or the reason it cannot be made: a text that could end its own element
/// or open a comment would let it escape the head block. The build already refuses both, so this
/// is a guard. The head holds, in order, the design style and the runtime's theme (the frame
/// carries them in that order), the theme switch, then the design script and the runtime script;
/// a page that needs only the runtime gets the same bytes as before the design system.
pub fn document(source: &str, assets: &Assets) -> Result<Vec<u8>, &'static str> {
    let unsafe_in = |text: &str, closing: &str| {
        text.contains("<!--") || text.to_ascii_lowercase().contains(closing)
    };
    if let Some(runtime) = &assets.runtime
        && (unsafe_in(runtime.script, "</script") || unsafe_in(runtime.style, "</style"))
    {
        return Err(UNUSABLE);
    }
    if let Some(design) = &assets.design
        && (unsafe_in(design.script, "</script") || unsafe_in(design.style, "</style"))
    {
        return Err(DESIGN_UNUSABLE);
    }
    let design_attribute = if assets.design.is_some() {
        " data-colab-design"
    } else {
        ""
    };
    let design_style = assets.design.map_or_else(String::new, |design| {
        format!("<style>{}</style>", design.style)
    });
    let runtime_style = assets.runtime.map_or_else(String::new, |runtime| {
        format!("<style type=\"text/tailwindcss\">{}</style>", runtime.style)
    });
    let design_script = assets.design.map_or_else(String::new, |design| {
        format!("<script>{}</script>", design.script)
    });
    let runtime_script = assets.runtime.map_or_else(String::new, |runtime| {
        format!("<script>{}</script>", runtime.script)
    });
    Ok(format!(
        "<!doctype html><html{design_attribute}><head><meta http-equiv=\"Content-Security-Policy\" content=\"{policy}\">\
         <meta charset=\"utf-8\">{design_style}{runtime_style}<script>{THEME_SWITCH}</script>\
         {design_script}{runtime_script}</head><body>{source}</body></html>",
        policy = meta_policy(),
    )
    .into_bytes())
}

/// The standalone copy of `source`, or why there is none; `None` for a page that needs none.
#[must_use]
pub fn derive(source: &str, assets: &Assets) -> Option<Derived> {
    let skipped = |reason| {
        Some(Derived {
            outcome: Outcome {
                included: false,
                reason: Some(reason),
            },
            file: None,
        })
    };
    let wants_runtime = match wants_runtime(source) {
        Ok(wants) => wants,
        Err(page_check::TooDeep) => return skipped(TOO_DEEP),
    };
    let wants_design = uses_design(source);
    if !wants_runtime && !wants_design {
        return None;
    }
    if wants_runtime && assets.runtime.is_none() {
        return skipped(NO_RUNTIME);
    }
    if wants_design && assets.design.is_none() {
        return skipped(NO_DESIGN);
    }
    let needed = Assets {
        runtime: assets.runtime.filter(|_| wants_runtime),
        design: assets.design.filter(|_| wants_design),
    };
    match document(source, &needed) {
        Ok(file) => Some(Derived {
            outcome: Outcome {
                included: true,
                reason: None,
            },
            file: Some(file),
        }),
        Err(reason) => skipped(reason),
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
    const DESIGN: Design = Design {
        script: "window.ui={};",
        style: "@layer tmt-ui{}",
    };
    const DESIGN_PAGE: &str = r#"<section class="tmt-ui-card">λ</section>"#;
    const RUNTIME_ONLY: Assets = Assets {
        runtime: Some(RUNTIME),
        design: None,
    };
    const DESIGN_ONLY: Assets = Assets {
        runtime: None,
        design: Some(DESIGN),
    };
    const BOTH: Assets = Assets {
        runtime: Some(RUNTIME),
        design: Some(DESIGN),
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
        let derived = derive(&marked, &BOTH).unwrap();
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
        let file = String::from_utf8(document(OPTED_IN, &RUNTIME_ONLY).unwrap()).unwrap();
        assert_eq!(
            file,
            format!(
                "<!doctype html><html><head><meta http-equiv=\"Content-Security-Policy\" content=\"{}\"><meta charset=\"utf-8\"><style type=\"text/tailwindcss\">@theme inline{{}}</style><script>{THEME_SWITCH}</script><script>window.tmt={{}};</script></head><body>{OPTED_IN}</body></html>",
                meta_policy()
            )
        );
    }

    #[test]
    fn a_text_that_could_leave_its_element_is_refused_with_the_reason_of_its_part() {
        for (script, style) in [
            ("a</script><b>", ""),
            ("a</SCRIPT >", ""),
            ("<!-- a", ""),
            ("", "a</style>"),
            ("", "a</STYLE >"),
            ("", "<!-- a"),
        ] {
            let runtime = Assets {
                runtime: Some(Runtime { script, style }),
                design: None,
            };
            assert_eq!(document(OPTED_IN, &runtime), Err(UNUSABLE));
            let design = Assets {
                runtime: None,
                design: Some(Design { script, style }),
            };
            assert_eq!(document(DESIGN_PAGE, &design), Err(DESIGN_UNUSABLE));
        }
    }

    #[test]
    fn a_page_that_needs_no_copy_has_none_and_one_that_does_gets_a_reason_or_the_file() {
        assert!(derive("<p>plain</p>", &BOTH).is_none());
        let none = derive(OPTED_IN, &Assets::default()).unwrap();
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
            &Assets {
                runtime: Some(Runtime {
                    script: "</script>",
                    style: "",
                }),
                design: None,
            },
        )
        .unwrap();
        assert_eq!(bad.outcome.reason, Some(UNUSABLE));
        let made = derive(OPTED_IN, &RUNTIME_ONLY).unwrap();
        assert_eq!(
            made.outcome,
            Outcome {
                included: true,
                reason: None
            }
        );
        assert_eq!(made.file, document(OPTED_IN, &RUNTIME_ONLY).ok());
    }

    #[test]
    fn the_shared_design_marker_vectors_decide_the_use() {
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../contracts/vectors/design-marker-v1.json"
        ))
        .unwrap();
        let cases = vectors["cases"].as_array().unwrap();
        assert!(cases.len() >= 20);
        for case in cases {
            assert_eq!(
                uses_design(case["source"].as_str().unwrap()),
                case["usesDesign"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn the_design_system_is_written_before_the_runtime_and_the_attribute_marks_the_root() {
        let both = String::from_utf8(document(OPTED_IN, &BOTH).unwrap()).unwrap();
        let at = |needle: &str| both.find(needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(both.starts_with("<!doctype html><html data-colab-design><head>"));
        // The frame carries the design style, then the runtime's theme; the design script comes
        // before the runtime's.
        assert!(at("<style>@layer tmt-ui{}</style>") < at("<style type=\"text/tailwindcss\">"));
        assert!(at("<script>window.ui={};</script>") < at("<script>window.tmt={};</script>"));
        assert!(at(THEME_SWITCH) < at("<script>window.ui={};</script>"));
        let alone = String::from_utf8(document(DESIGN_PAGE, &DESIGN_ONLY).unwrap()).unwrap();
        assert!(!alone.contains("tailwindcss") && !alone.contains("window.tmt"));
        let runtime = String::from_utf8(document(OPTED_IN, &RUNTIME_ONLY).unwrap()).unwrap();
        assert!(runtime.starts_with("<!doctype html><html><head>"));
        assert!(!runtime.contains("data-colab-design") && !runtime.contains("window.ui"));
    }

    #[test]
    fn only_what_a_page_asks_for_is_written_and_a_missing_part_is_named() {
        // The runtime page is not given the design system, and the design page is not given the runtime.
        let runtime = derive(OPTED_IN, &BOTH).unwrap();
        assert_eq!(runtime.file, document(OPTED_IN, &RUNTIME_ONLY).ok());
        let design = derive(DESIGN_PAGE, &BOTH).unwrap();
        assert_eq!(design.file, document(DESIGN_PAGE, &DESIGN_ONLY).ok());
        // A page that asks for both gets both, or the reason of the first part the build lacks.
        let source = format!("{OPTED_IN}{DESIGN_PAGE}");
        assert_eq!(
            derive(&source, &BOTH).unwrap().file,
            document(&source, &BOTH).ok()
        );
        assert_eq!(
            derive(&source, &DESIGN_ONLY).unwrap().outcome.reason,
            Some(NO_RUNTIME)
        );
        assert_eq!(
            derive(&source, &RUNTIME_ONLY).unwrap().outcome.reason,
            Some(NO_DESIGN)
        );
        assert_eq!(
            derive(DESIGN_PAGE, &RUNTIME_ONLY).unwrap().outcome.reason,
            Some(NO_DESIGN)
        );
        for reason in [NO_RUNTIME, NO_DESIGN, UNUSABLE, DESIGN_UNUSABLE, TOO_DEEP] {
            assert_ne!(explanation(reason), "unknown reason", "{reason}");
        }
    }
}
