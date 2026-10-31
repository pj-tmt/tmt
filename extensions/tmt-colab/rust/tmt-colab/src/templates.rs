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
/// page, so the copy reuses them. Only parser-emitted start tags with exact source ranges are
/// rewritten; unmapped ranges stay unchanged. The result is parsed again with the page checker:
/// `None` unless every admitted block names the creator and the question IDs remain the source's
/// own, in order. Unfinished tag-like text stays literal and registers no block.
pub fn rewrite_question_agents(source: &str, creator: Option<&str>) -> Option<String> {
    let escaped = creator.map(escape_attribute);
    let before = crate::page_check::check(source).ok()?;
    let mut rewritten = String::with_capacity(source.len() + 64);
    let mut at = 0;
    let tags = crate::page_check::start_tags(source)?;
    let tags: Vec<_> = tags
        .into_iter()
        .filter(|tag| AGENT_BLOCKS.contains(&tag.name.as_str()))
        .collect();
    for tag in &tags {
        let Some(range) = &tag.range else { continue };
        if range.start < at {
            continue;
        }
        let raw = &source[range.clone()];
        let Some(updated) = rewrite_tag(raw, 1 + tag.name.len(), escaped.as_deref()) else {
            continue;
        };
        rewritten.push_str(&source[at..range.start]);
        rewritten.push_str(&updated);
        at = range.end;
    }
    rewritten.push_str(&source[at..]);
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
        && admitted <= tags.len()
        && named
            .into_iter()
            .all(|agent| agent == creator.unwrap_or_default()))
    .then_some(rewritten)
}

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `tag` with its first `data-agent` attribute set to `value` (already escaped), or added after the
/// tag name when it has none; without a `value` the attribute is removed (a block with no agent is
/// unassigned). `name_end` is the byte length of `<` and the name.
fn rewrite_tag(tag: &str, name_end: usize, value: Option<&str>) -> Option<String> {
    let bytes = tag.as_bytes();
    let blank = |at: usize| bytes.get(at).is_some_and(u8::is_ascii_whitespace);
    let mut at = name_end;
    loop {
        let whitespace = at;
        while blank(at) || bytes.get(at) == Some(&b'/') {
            at += 1;
        }
        if at >= bytes.len() || bytes[at] == b'>' {
            return Some(match value {
                Some(value) => format!(
                    "{} data-agent=\"{value}\"{}",
                    &tag[..name_end],
                    &tag[name_end..]
                ),
                None => tag.to_owned(),
            });
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
            return Some(match (value, range) {
                (Some(value), Some((start, end))) => {
                    format!("{}\"{value}\"{}", &tag[..start], &tag[end..])
                }
                (Some(value), None) => {
                    format!("{}=\"{value}\"{}", &tag[..after_name], &tag[after_name..])
                }
                (None, Some((_, end))) => format!("{}{}", &tag[..whitespace], &tag[end..]),
                (None, None) => format!("{}{}", &tag[..whitespace], &tag[after_name..]),
            });
        }
    }
}

/// Whether `source` is exactly what a user template's page `template_source` gives a new page
/// created by `creator` (none for an anonymous page): the same bytes with only the agents of its
/// blocks rewritten.
pub fn admits_user(template_source: &str, source: &str, creator: Option<&str>) -> bool {
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

/// The look every template starts from: the generated starter of the page-style reference, the glue
/// on top of the injected design system and its editorial layer (#5794), without its `body` rule
/// (max-width, margin, padding). That rule frames `canvas` and `freeform` pages; on `column` and
/// `full` Colab sets the width and the gaps, and a padding above the first block would add one.
fn base_style() -> String {
    const FRAME: &str =
        "  body {\n    max-width: 1280px;\n    margin: 0 auto;\n    padding: 16px;\n  }\n";
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
        let rewritten = rewrite_question_agents(&source, Some("a & \"b\"")).unwrap();
        assert!(rewritten.starts_with(comment));
        assert_eq!(agents(&rewritten), ["a & \"b\""]);
        assert!(rewritten.contains("DATA-AGENT=\"a &amp; &quot;b&quot;\""));
        // Rewriting what is already rewritten changes nothing.
        assert_eq!(
            rewrite_question_agents(&rewritten, Some("a & \"b\"")).unwrap(),
            rewritten
        );
    }

    #[test]
    fn a_block_without_an_agent_gets_one_and_an_unquoted_one_is_replaced() {
        let none = QUESTION.replace(" DATA-AGENT='old'", "");
        assert_eq!(agents(&none), [""]);
        assert_eq!(
            agents(&rewrite_question_agents(&none, Some("new")).unwrap()),
            ["new"]
        );
        let bare = QUESTION.replace("'old'", "old");
        let rewritten = rewrite_question_agents(&bare, Some("new")).unwrap();
        assert!(rewritten.contains("DATA-AGENT=\"new\">"));
        assert_eq!(agents(&rewritten), ["new"]);
        let valueless = QUESTION.replace("DATA-AGENT='old'", "data-agent");
        assert_eq!(
            agents(&rewrite_question_agents(&valueless, Some("new")).unwrap()),
            ["new"]
        );
    }

    #[test]
    fn unfinished_tag_text_is_preserved_without_registering_a_question() {
        let source = "<tmt-question data-id=\"x\"";
        assert_eq!(
            rewrite_question_agents(source, Some("new")).as_deref(),
            Some(source)
        );
        assert!(agents(source).is_empty());
    }

    #[test]
    fn every_compiled_template_rewrites_to_what_a_creator_gets() {
        for template in &TEMPLATES {
            let rendered = template.source().unwrap();
            let old = creator_source(&rendered, "old");
            let new = creator_source(&rendered, "new");
            assert_eq!(
                rewrite_question_agents(&old, Some("new")).as_deref(),
                Some(new.as_str()),
                "{}",
                template.name
            );
        }
    }

    #[test]
    fn attribute_and_raw_text_lookalikes_are_not_markup_to_rewrite() {
        for prefix in [
            "<div title=\"<tmt-question data-agent='kept'>\">Notes</div>\n",
            "<script>const text = \"<tmt-question data-agent='kept'>\";</script>\n",
            "<style>p::before { content: \"<tmt-review data-agent='kept'>\"; }</style>\n",
            "<textarea><tmt-read data-agent='kept'></textarea>\n",
            "<!-- <tmt-question data-agent='kept'> -->\n",
        ] {
            let source = format!("{prefix}{QUESTION}");
            for creator in [Some("a & \"b\""), None] {
                let actual = rewrite_question_agents(&source, creator).unwrap();
                let expected_block = match creator {
                    Some(_) => {
                        QUESTION.replace("DATA-AGENT='old'", "DATA-AGENT=\"a &amp; &quot;b&quot;\"")
                    }
                    None => QUESTION.replace(" DATA-AGENT='old'", ""),
                };
                assert_eq!(actual, format!("{prefix}{expected_block}"));
                assert_eq!(agents(&actual), [creator.unwrap_or_default()]);
            }
        }
    }

    #[test]
    fn an_unmapped_tag_is_unchanged_and_admission_decides_the_copy() {
        // The first '<' is emitted after the tokenizer has consumed/reconsumed the second.
        // That boundary does not identify an exact start range; never search backwards for it.
        let source = QUESTION.replace("<tmt-question data-id", "<<tmt-question data-id");
        assert_eq!(
            rewrite_question_agents(&source, Some("old")).as_deref(),
            Some(source.as_str())
        );
        assert_eq!(rewrite_question_agents(&source, Some("new")), None);
    }

    #[test]
    fn parser_positions_preserve_utf8_crlf_bom_and_chunk_boundaries() {
        for prefix in [
            "\u{feff}",
            "\r\n",
            "héllo &amp;",
            "&amp",
            "a<!-- before -->",
        ] {
            let source = format!("{prefix}{QUESTION}");
            let expected = format!(
                "{prefix}{}",
                QUESTION.replace("DATA-AGENT='old'", "DATA-AGENT=\"new\"")
            );
            assert_eq!(
                rewrite_question_agents(&source, Some("new")),
                Some(expected)
            );
        }
        let prefix = format!("<p>{}</p>", "é".repeat(8190));
        let question = QUESTION.replace(
            " data-kind=\"single\"",
            " title=\"x > y\" data-kind=\"single\"",
        );
        let source = format!("{prefix}{question}");
        assert_eq!(
            rewrite_question_agents(&source, Some("new")),
            Some(format!(
                "{prefix}{}",
                question.replace("DATA-AGENT='old'", "DATA-AGENT=\"new\"")
            ))
        );
    }

    #[test]
    fn a_new_page_is_admitted_only_as_the_rewritten_template() {
        let template = format!("<p>Notes</p>\n{QUESTION}");
        let page = rewrite_question_agents(&template, Some("ada")).unwrap();
        assert!(admits_user(&template, &page, Some("ada")));
        assert!(!admits_user(&template, &page, Some("grace")));
        assert!(!admits_user(
            &template,
            &(page.clone() + "<p>more</p>"),
            Some("ada")
        ));
        assert!(!admits_user(&template, &template, Some("ada")));
    }

    #[test]
    fn an_anonymous_copy_has_no_agent_on_any_block_and_nothing_else_changes() {
        let source = format!("<p>Intro</p>\n{QUESTION}");
        let rewritten = rewrite_question_agents(&source, None).unwrap();
        assert!(
            !rewritten.to_ascii_lowercase().contains("data-agent"),
            "{rewritten}"
        );
        assert_eq!(rewritten, source.replace(" DATA-AGENT='old'", ""));
        assert_eq!(agents(&rewritten), [""]);
        // Quoted and bare values go too, and a block that never had one is left as it is.
        let bare = QUESTION.replace("'old'", "old");
        assert!(
            !rewrite_question_agents(&bare, None)
                .unwrap()
                .contains("old")
        );
        let valueless = QUESTION.replace("DATA-AGENT='old'", "data-agent");
        assert!(
            !rewrite_question_agents(&valueless, None)
                .unwrap()
                .to_ascii_lowercase()
                .contains("data-agent")
        );
        let none = QUESTION.replace(" DATA-AGENT='old'", "");
        assert_eq!(rewrite_question_agents(&none, None).unwrap(), none);
        let page = rewrite_question_agents(&source, None).unwrap();
        assert!(admits_user(&source, &page, None));
        assert!(!admits_user(&source, &page, Some("ada")));
        assert!(!admits_user(&source, &source, None));
    }
}
