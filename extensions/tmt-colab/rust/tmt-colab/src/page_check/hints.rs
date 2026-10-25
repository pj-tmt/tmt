//! Advice about a page, not findings about its markup: a hint never ignores a block, never makes
//! `--strict` refuse and never changes an exit code. It has a stable code and a sentence the command
//! words; the shape is `{code, message}`, so later codes fit the same array.

use super::tree::Tree;

/// A source that contains any of these, ignoring ASCII case, uses the design system. A plain text
/// test with no parsing: the renderer frame makes the same one to paint the page, the exports carry
/// the design system for it, and `vectors/design-marker-v1.json` pins the frame's, the browser
/// export's and this one.
const DESIGN_MARKERS: [&str; 3] = ["tmt-ui-", "<tmt-icon", "tmt-starter"];

/// The custom properties the earlier starter (before the design system) declared; pages made from it
/// still carry them.
const EARLIER_STARTER: [&str; 2] = ["--ink:", "--paper:"];

/// One piece of advice about a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    pub code: &'static str,
}

/// Whether `source` uses the design system: it contains `tmt-ui-`, `<tmt-icon` or `tmt-starter`,
/// ignoring ASCII case.
#[must_use]
pub fn uses_design(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    DESIGN_MARKERS.iter().any(|marker| lower.contains(marker))
}

/// The page carries the starter: the design system's markers, or the earlier starter's properties.
fn carries_starter(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    uses_design(source)
        || EARLIER_STARTER
            .iter()
            .all(|property| lower.contains(property))
}

/// The page has CSS of its own, a `<style>` element or a `style` attribute.
fn styles_itself(tree: &Tree) -> bool {
    tree.elements()
        .into_iter()
        .any(|id| tree.name(id) == "style" || tree.attr(id, "style").is_some())
}

/// The hints for `source`, in a stable order.
pub(super) fn of(source: &str, tree: &Tree) -> Vec<Hint> {
    let mut hints = Vec::new();
    // Styled by hand with nothing of the starter or the design system: the pages Ben's question was
    // about. A bare page has nothing to nudge toward, and a page that carries either is where it
    // should be.
    if styles_itself(tree) && !carries_starter(source) {
        hints.push(Hint {
            code: "own-style-only",
        });
    }
    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hints(source: &str) -> Vec<&'static str> {
        let Ok(tree) = Tree::parse(source) else {
            panic!("the source nests too deep");
        };
        of(source, &tree).iter().map(|hint| hint.code).collect()
    }

    #[test]
    fn a_page_with_its_own_css_and_no_starter_is_hinted() {
        assert_eq!(
            hints("<style>p{color:red}</style><p>Hi</p>"),
            ["own-style-only"]
        );
        assert_eq!(hints("<p style=\"color:red\">Hi</p>"), ["own-style-only"]);
    }

    #[test]
    fn a_bare_page_a_design_page_and_an_earlier_starter_page_are_not() {
        assert!(hints("<p>Hi</p>").is_empty());
        assert!(
            hints("<style>p{color:red}</style><section class=\"tmt-ui-card\">Hi</section>")
                .is_empty()
        );
        assert!(hints("<style>@layer tmt-starter{p{margin:0}}</style><p>Hi</p>").is_empty());
        assert!(hints("<style>:root{--ink:#000;--paper:#fff}</style><p>Hi</p>").is_empty());
        // One earlier-starter property alone is not the starter.
        assert_eq!(
            hints("<style>:root{--ink:#000}</style><p>Hi</p>"),
            ["own-style-only"]
        );
    }

    #[test]
    fn the_markers_are_read_as_text_in_any_case() {
        assert!(uses_design("<P CLASS=\"TMT-UI-CARD\">x</P>"));
        assert!(uses_design("<!-- tmt-starter -->"));
        assert!(!uses_design("<p class=\"tmt-uix\">x</p>"));
    }
}
