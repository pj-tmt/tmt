//! Compiled first-party template source admission. Creation may register these questions only
//! when every literal byte and repeated UUID placeholder matches the compiled template.
use crate::{
    Result,
    decoder::{LayoutKind, PageLayout},
};
use std::collections::BTreeMap;

/// One starting page: its name, what it is for, the layout it sets and its source.
pub struct Template {
    pub name: &'static str,
    pub summary: &'static str,
    pub kind: LayoutKind,
    pub source: &'static str,
}

pub const TEMPLATES: [Template; 5] = [
    Template {
        name: "report",
        summary: "Summary, findings table and next steps",
        kind: LayoutKind::Column,
        source: include_str!("../templates/report.html"),
    },
    Template {
        name: "review",
        summary: "Parts for the reader to confirm, and questions",
        kind: LayoutKind::Column,
        source: include_str!("../templates/review.html"),
    },
    Template {
        name: "compare",
        summary: "Now and after, side by side",
        kind: LayoutKind::Column,
        source: include_str!("../templates/compare.html"),
    },
    Template {
        name: "board",
        summary: "A full-bleed status board",
        kind: LayoutKind::Full,
        source: include_str!("../templates/board.html"),
    },
    Template {
        name: "gallery",
        summary: "The design-system components, to copy from",
        kind: LayoutKind::Column,
        source: include_str!("../templates/gallery.html"),
    },
];

impl Template {
    pub fn layout(&self) -> PageLayout {
        PageLayout {
            kind: self.kind,
            reason: None,
        }
    }

    /// The page source: `{{style}}` becomes the base look, and each `{{uuid:NAME}}` a fresh
    /// canonical UUID, the same one for the same name within this page (a block's ID is unique on
    /// its page, never reused).
    pub fn source(&self) -> Result<String> {
        let source = self.source.replace("{{style}}", base_style().trim_end());
        let mut names: Vec<(&str, String)> = Vec::new();
        let mut rendered = String::with_capacity(source.len() + 64);
        let mut rest = source.as_str();
        while let Some(start) = rest.find("{{uuid:") {
            rendered.push_str(&rest[..start]);
            let after = &rest[start + "{{uuid:".len()..];
            let end = after
                .find("}}")
                .expect("a template placeholder is closed (checked by the catalog test)");
            let name = &after[..end];
            let id = match names.iter().find(|(known, _)| *known == name) {
                Some((_, id)) => id.clone(),
                None => {
                    let id = crate::page::fresh_id()?;
                    names.push((name, id.clone()));
                    id
                }
            };
            rendered.push_str(&id);
            rest = &after[end + 2..];
        }
        rendered.push_str(rest);
        Ok(rendered)
    }
}

pub fn find(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|template| template.name == name)
}

pub fn creator_source(source: &str, name: &str) -> String {
    let escaped = name
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    source.replace(
        "data-agent=\"Your agent name\"",
        &format!("data-agent=\"{escaped}\""),
    )
}

/// No arbitrary initial-registry payload is admitted. IDs and titles come from this checked
/// first-party source; authored files must register their questions before a later write.
pub fn admits(name: &str, source: &str, creator: &str) -> bool {
    let Some(template) = find(name) else {
        return false;
    };
    let pattern = creator_source(
        &template
            .source
            .replace("{{style}}", base_style().trim_end()),
        creator,
    );
    let mut pattern = pattern.as_str();
    let mut source = source;
    let mut ids = BTreeMap::<&str, &str>::new();
    while let Some(start) = pattern.find("{{uuid:") {
        let Some(rest) = source.strip_prefix(&pattern[..start]) else {
            return false;
        };
        pattern = &pattern[start + "{{uuid:".len()..];
        let Some(end) = pattern.find("}}") else {
            return false;
        };
        let key = &pattern[..end];
        let Some(id) = rest.get(..36) else {
            return false;
        };
        if tmt_colab_model::values::generated_id(id).is_err()
            || ids.get(key).is_some_and(|previous| *previous != id)
        {
            return false;
        }
        ids.insert(key, id);
        source = &rest[36..];
        pattern = &pattern[end + 2..];
    }
    source == pattern
}

/// The look every template starts from: the generated starter of the page-style reference, which
/// owns the type scale and rhythm on top of the injected design system, without its `body` rule
/// (max-width, margin, padding). That rule frames `canvas` and `freeform` pages; on `column` and
/// `full` Colab sets the width and the gaps, and a padding above the first block would add one.
fn base_style() -> String {
    const FRAME: &str = "    max-width: 1280px;\n    margin: 0 auto;\n    padding: 16px;\n";
    let reference = include_str!("../../../skills/tmt-colab/references/page-style.md");
    let starter = reference
        .split("<!-- BEGIN generated page style -->")
        .nth(1)
        .and_then(|region| region.split("```html\n").nth(1))
        .and_then(|block| block.split("```").next())
        .expect("the page-style reference holds its generated starter");
    assert!(starter.contains(FRAME), "the starter's body rule changed");
    // The starter ends with an inert comment of copyable question and reading examples for a page
    // started from the bare starter; a template carries blocks of its own.
    let examples = starter
        .find("<!-- Examples to copy")
        .expect("the starter holds its copyable examples");
    starter[..examples].replacen(FRAME, "", 1)
}
