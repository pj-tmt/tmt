//! Whole-token file references over held message bytes. Matching wrapping quotes
//! are syntax; their contents stay literal. Recognition never inspects the filesystem.
use super::*;
use std::ops::Range;

#[derive(Clone, Debug)]
pub(super) struct Reference {
    pub range: Range<usize>,
    pub path: String,
}

impl Reference {
    pub fn label(&self) -> String {
        let name = self.path.rsplit('/').next().unwrap_or(&self.path);
        format!("⧉ {}", crate::board::notes::sanitize(name))
    }
}

pub(super) fn fit(text: &str, width: usize) -> String {
    crate::board::view::fit(text, width.min(unicode_width::UnicodeWidthStr::width(text)))
}

pub(super) fn scan(text: &str) -> Vec<Reference> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let character = text[at..].chars().next().expect("character boundary");
        if character.is_whitespace() {
            at += character.len_utf8();
            continue;
        }
        let start = at;
        let quote = matches!(character, '\'' | '"').then_some(character);
        if let Some(quote) = quote {
            at += quote.len_utf8();
            while at < text.len() {
                let next = text[at..].chars().next().expect("character boundary");
                if matches!(next, '\n' | '\r') {
                    break;
                }
                at += next.len_utf8();
                if next == quote && text[at..].chars().next().is_none_or(char::is_whitespace) {
                    break;
                }
            }
        } else {
            while at < text.len() {
                let next = text[at..].chars().next().expect("character boundary");
                if next.is_whitespace() {
                    break;
                }
                at += next.len_utf8();
            }
        }
        let raw = &text[start..at];
        let path = match quote {
            Some(quote) => raw
                .strip_prefix(quote)
                .and_then(|text| text.strip_suffix(quote)),
            None => Some(raw),
        };
        if let Some(path) = path
            && path.starts_with('/')
            && !path.ends_with('/')
            && !path.chars().any(char::is_control)
        {
            found.push(Reference {
                range: start..at,
                path: path.to_owned(),
            });
        }
    }
    found
}

/// A reference inside or crossing a code/link/literal container stays inert.
/// Inline syntax wholly inside a path belongs to its literal filename instead.
pub(super) fn retain_prose(references: &mut Vec<Reference>, events: &[(Event<'_>, Range<usize>)]) {
    let mut blocked: Vec<_> = events
        .iter()
        .filter_map(|(event, range)| {
            matches!(
                event,
                Event::Code(_)
                    | Event::Html(_)
                    | Event::InlineHtml(_)
                    | Event::FootnoteReference(_)
                    | Event::Start(
                        Tag::CodeBlock(_)
                            | Tag::Link { .. }
                            | Tag::Image { .. }
                            | Tag::HtmlBlock
                            | Tag::FootnoteDefinition(_)
                            | Tag::MetadataBlock(_)
                            | Tag::DefinitionList
                    )
            )
            .then_some(range.clone())
        })
        .collect();
    blocked.sort_by_key(|range| range.start);
    let (mut at, mut end) = (0, 0);
    references.retain(|reference| {
        let mut admitted = reference.range.start >= end;
        while at < blocked.len() && blocked[at].start < reference.range.end {
            let range = &blocked[at];
            if range.end > reference.range.start
                && (range.start <= reference.range.start || range.end > reference.range.end)
            {
                admitted = false;
            }
            end = end.max(range.end);
            at += 1;
        }
        admitted
    });
}

pub(super) fn inline(event: &Event<'_>) -> bool {
    matches!(
        event,
        Event::Code(_)
            | Event::Html(_)
            | Event::InlineHtml(_)
            | Event::FootnoteReference(_)
            | Event::Start(Tag::Emphasis | Tag::Strong | Tag::Link { .. } | Tag::Image { .. })
            | Event::End(TagEnd::Emphasis | TagEnd::Strong | TagEnd::Link | TagEnd::Image)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_keep_exact_ranges_and_only_remove_matching_wrapping_quotes() {
        let source = "see '/tmp/one file.txt' and \"/tmp/another_file.txt\" /tmp/bare.txt";
        let found = scan(source);
        assert_eq!(
            found
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            [
                "/tmp/one file.txt",
                "/tmp/another_file.txt",
                "/tmp/bare.txt"
            ]
        );
        assert_eq!(
            found
                .iter()
                .map(|file| &source[file.range.clone()])
                .collect::<Vec<_>>(),
            [
                "'/tmp/one file.txt'",
                "\"/tmp/another_file.txt\"",
                "/tmp/bare.txt"
            ]
        );
        for source in [
            "prefix/tmp/file.txt",
            "'relative /tmp/file.txt'",
            "'/tmp/unclosed file.txt",
            "'/tmp/file.txt'prose",
            "https://example.com/tmp/file.txt",
            "/",
            "/tmp/",
        ] {
            assert!(scan(source).is_empty(), "{source:?}");
        }
    }
}
