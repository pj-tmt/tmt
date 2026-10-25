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

pub const TEMPLATES: [Template; 6] = [
    Template {
        name: "starter",
        summary: "Title, summary, sections and questions",
        kind: LayoutKind::Column,
        source: include_str!("../templates/starter.html"),
    },
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

/// The optional creator of a compiled first-party template; anonymous questions stay on-page.
pub fn source_for_creator(source: &str, creator: Option<&str>) -> String {
    match creator {
        Some(name) => creator_source(source, name),
        None => source.replace(" data-agent=\"Your agent name\"", ""),
    }
}

/// The blocks that name the agent that asked them.
const AGENT_BLOCKS: [&str; 3] = ["tmt-question", "tmt-review", "tmt-read"];

/// A user template's source for a new page: the template page's HTML with the agent of every
/// question, review and reading block set to the creator. Block IDs stay: an ID is scoped to its
/// page, so the copy reuses them. Only the start tags of those blocks are read, and the result is
/// parsed again with the page checker: `None` unless every block it admits names the creator and
/// the question IDs are the source's own, in order, so a block this scan cannot locate refuses the
/// copy instead of registering something unverified.
pub fn rewrite_question_agents(source: &str, creator: &str) -> Option<String> {
    let escaped = escape_attribute(creator);
    let before = crate::page_check::check(source).ok()?;
    let mut rewritten = String::with_capacity(source.len() + 64);
    let mut rest = source;
    let mut tags = 0;
    while let Some(start) = rest.find('<') {
        rewritten.push_str(&rest[..start]);
        rest = &rest[start..];
        if let Some(after) = rest.strip_prefix("<!--") {
            let end = after.find("-->").map_or(rest.len(), |end| 4 + end + 3);
            rewritten.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        let name_len = rest[1..]
            .find(|c: char| c.is_ascii_whitespace() || matches!(c, '/' | '>'))
            .unwrap_or(rest.len() - 1);
        let name = &rest[1..1 + name_len];
        if !AGENT_BLOCKS
            .iter()
            .any(|block| name.eq_ignore_ascii_case(block))
        {
            rewritten.push('<');
            rest = &rest[1..];
            continue;
        }
        let end = tag_end(rest)?;
        rewritten.push_str(&rewrite_tag(&rest[..end], 1 + name_len, &escaped)?);
        rest = &rest[end..];
        tags += 1;
    }
    rewritten.push_str(rest);
    let after = crate::page_check::check(&rewritten).ok()?;
    let ids = |report: &crate::page_check::Report| -> Vec<String> {
        report
            .question_sources
            .iter()
            .map(|q| q.id.clone())
            .collect()
    };
    let named = after
        .question_sources
        .iter()
        .map(|q| &q.agent)
        .chain(after.reviews.iter().map(|r| &r.agent))
        .chain(after.readings.iter().map(|r| &r.agent));
    let admitted = after.question_sources.len() + after.reviews.len() + after.readings.len();
    (ids(&before) == ids(&after)
        && admitted <= tags
        && named.into_iter().all(|agent| agent == creator))
    .then_some(rewritten)
}

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The byte length of the start tag at the beginning of `tag`, quotes respected; `None` when it
/// never closes.
fn tag_end(tag: &str) -> Option<usize> {
    let mut quote = None;
    for (index, c) in tag.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(open), c) if c == open => quote = None,
            (None, '>') => return Some(index + 1),
            _ => {}
        }
    }
    None
}

/// `tag` with its first `data-agent` attribute set to `value` (already escaped), or added after the
/// tag name when it has none. `name_end` is the byte length of `<` and the name.
fn rewrite_tag(tag: &str, name_end: usize, value: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let blank = |at: usize| bytes.get(at).is_some_and(u8::is_ascii_whitespace);
    let mut at = name_end;
    loop {
        while blank(at) || bytes.get(at) == Some(&b'/') {
            at += 1;
        }
        if at >= bytes.len() || bytes[at] == b'>' {
            return Some(format!(
                "{} data-agent=\"{value}\"{}",
                &tag[..name_end],
                &tag[name_end..]
            ));
        }
        let name_start = at;
        while at < bytes.len() && !blank(at) && !matches!(bytes[at], b'=' | b'/' | b'>') {
            at += 1;
        }
        let name = &tag[name_start..at];
        let after_name = at;
        while blank(at) {
            at += 1;
        }
        // The attribute's value, if it has one, as a byte range of `tag`.
        let mut range = None;
        if bytes.get(at) == Some(&b'=') {
            at += 1;
            while blank(at) {
                at += 1;
            }
            let start = at;
            match bytes.get(at) {
                Some(&quote @ (b'"' | b'\'')) => {
                    at = tag[at + 1..].find(quote as char)? + at + 2;
                }
                _ => {
                    while at < bytes.len() && !blank(at) && bytes[at] != b'>' {
                        at += 1;
                    }
                }
            }
            range = Some((start, at));
        }
        if name.eq_ignore_ascii_case("data-agent") {
            return Some(match range {
                Some((start, end)) => {
                    format!("{}\"{value}\"{}", &tag[..start], &tag[end..])
                }
                None => format!("{}=\"{value}\"{}", &tag[..after_name], &tag[after_name..]),
            });
        }
    }
}

/// Whether `source` is exactly what a user template's page `template_source` gives a new page
/// created by `creator`: the same bytes with only the agents of its blocks rewritten.
pub fn admits_user(template_source: &str, source: &str, creator: &str) -> bool {
    rewrite_question_agents(template_source, creator).as_deref() == Some(source)
}

/// No arbitrary initial-registry payload is admitted. IDs and titles come from this checked
/// first-party source; authored files must register their questions before a later write.
pub fn admits(name: &str, source: &str, creator: Option<&str>) -> bool {
    let Some(template) = find(name) else {
        return false;
    };
    let pattern = template
        .source
        .replace("{{style}}", base_style().trim_end());
    let mut pattern = pattern.as_str();
    let mut source = source;
    let mut ids = BTreeMap::<&str, &str>::new();
    while let Some(start) = pattern.find("{{uuid:") {
        // Only compiled UUID placeholders are slots. A creator label remains literal even
        // when its text looks like a placeholder, and cannot introduce another slot.
        let literal = source_for_creator(&pattern[..start], creator);
        let Some(rest) = source.strip_prefix(literal.as_str()) else {
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
    source == source_for_creator(pattern, creator)
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

#[cfg(test)]
mod tests {
    use super::*;

    const QUESTION: &str = r#"<tmt-questions data-id="20000000-0000-4000-8000-000000000001">
<p data-title>Decision</p>
<tmt-question data-id="10000000-0000-4000-8000-000000000001" data-kind="single" DATA-AGENT='old'>
<p data-question>Which?</p><span data-choice>A</span><span data-choice>B</span>
</tmt-question></tmt-questions>
"#;

    fn agents(source: &str) -> Vec<String> {
        let report = crate::page_check::check(source).unwrap();
        report
            .question_sources
            .into_iter()
            .map(|q| q.agent)
            .chain(report.reviews.into_iter().map(|r| r.agent))
            .chain(report.readings.into_iter().map(|r| r.agent))
            .collect()
    }

    #[test]
    fn every_agent_block_names_the_creator_and_everything_else_is_kept() {
        let comment = "<!-- <tmt-question data-agent=\"kept\"> -->\n";
        let source = format!("{comment}{QUESTION}");
        let rewritten = rewrite_question_agents(&source, "a & \"b\"").unwrap();
        assert!(rewritten.starts_with(comment));
        assert_eq!(agents(&rewritten), ["a & \"b\""]);
        assert!(rewritten.contains("DATA-AGENT=\"a &amp; &quot;b&quot;\""));
        // Rewriting what is already rewritten changes nothing.
        assert_eq!(
            rewrite_question_agents(&rewritten, "a & \"b\"").unwrap(),
            rewritten
        );
    }

    #[test]
    fn a_block_without_an_agent_gets_one_and_an_unquoted_one_is_replaced() {
        let none = QUESTION.replace(" DATA-AGENT='old'", "");
        assert_eq!(agents(&none), [""]);
        assert_eq!(
            agents(&rewrite_question_agents(&none, "new").unwrap()),
            ["new"]
        );
        let bare = QUESTION.replace("'old'", "old");
        let rewritten = rewrite_question_agents(&bare, "new").unwrap();
        assert!(rewritten.contains("DATA-AGENT=\"new\">"));
        assert_eq!(agents(&rewritten), ["new"]);
        let valueless = QUESTION.replace("DATA-AGENT='old'", "data-agent");
        assert_eq!(
            agents(&rewrite_question_agents(&valueless, "new").unwrap()),
            ["new"]
        );
    }

    #[test]
    fn a_tag_that_never_closes_is_refused() {
        assert_eq!(
            rewrite_question_agents("<tmt-question data-id=\"x\"", "new"),
            None
        );
    }

    #[test]
    fn every_compiled_template_rewrites_to_what_a_creator_gets() {
        for template in &TEMPLATES {
            let rendered = template.source().unwrap();
            let old = creator_source(&rendered, "old");
            let new = creator_source(&rendered, "new");
            assert_eq!(
                rewrite_question_agents(&old, "new").as_deref(),
                Some(new.as_str()),
                "{}",
                template.name
            );
        }
    }

    #[test]
    fn a_new_page_is_admitted_only_as_the_rewritten_template() {
        let template = format!("<p>Notes</p>\n{QUESTION}");
        let page = rewrite_question_agents(&template, "ada").unwrap();
        assert!(admits_user(&template, &page, "ada"));
        assert!(!admits_user(&template, &page, "grace"));
        assert!(!admits_user(
            &template,
            &(page.clone() + "<p>more</p>"),
            "ada"
        ));
        assert!(!admits_user(&template, &template, "ada"));
    }
}
