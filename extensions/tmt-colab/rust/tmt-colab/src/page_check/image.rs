//! Page images: `data:` images that make a page heavy, and `colab-attachment:` references the
//! renderer cannot use. The reference grammar and cap are the app's `document-images.ts`.

use std::collections::HashSet;

use super::{
    Finding, Reference, Report, Severity,
    question::is_uuid,
    tree::{Id, Tree},
};
use crate::limits::{ATTACHMENT_REFS, DATA_IMAGE_BYTES, DATA_IMAGES_TOTAL_BYTES};

const REFERENCE: &str = "colab-attachment:";

/// A reference the renderer resolves: 8 lowercase hex digits (a unique ID prefix) or a full ID.
pub(super) fn reference_token(src: &str) -> Option<&str> {
    let token = src.strip_prefix(REFERENCE)?;
    let prefix = token.len() == 8
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    (prefix || is_uuid(token)).then_some(token)
}

/// Preserve source bytes outside complete img src values. The parsed image count must agree
/// with the conservative source scan; otherwise no Save is safe and the repair stays pending.
pub(crate) fn repair_references(source: &str, old: &str, new: &str, short: bool) -> Option<String> {
    let matches = |value: &str| {
        reference_token(value).is_some_and(|token| token == old || (short && token == &old[..8]))
    };
    let tree = Tree::parse(source).ok()?;
    let expected = tree
        .elements()
        .into_iter()
        .filter(|id| tree.name(*id) == "img" && tree.attr(*id, "src").is_some_and(matches))
        .count();
    let ranges: Vec<_> = source_values(source)?
        .into_iter()
        .filter(|range| matches(&source[range.clone()]))
        .collect();
    if ranges.len() != expected {
        return None;
    }
    let mut result = String::with_capacity(source.len());
    let mut from = 0;
    for range in ranges {
        result.push_str(&source[from..range.start]);
        result.push_str(REFERENCE);
        result.push_str(new);
        from = range.end;
    }
    result.push_str(&source[from..]);
    // A matching count alone can be coincidental in unusual HTML. Verify that no admitted
    // old reference remains, so an ignored source range cannot stand in for a parsed image.
    let repaired = Tree::parse(&result).ok()?;
    if repaired
        .elements()
        .into_iter()
        .any(|id| repaired.name(id) == "img" && repaired.attr(id, "src").is_some_and(matches))
    {
        return None;
    }
    Some(result)
}

/// Scan attribute boundaries without serializing HTML. Comments and raw text are inert;
/// incomplete or unusual syntax refuses the repair rather than guessing editable ranges.
fn source_values(source: &str) -> Option<Vec<std::ops::Range<usize>>> {
    let bytes = source.as_bytes();
    let lower = source.to_ascii_lowercase();
    let mut at = 0;
    let mut values = Vec::new();
    while let Some(offset) = source[at..].find('<') {
        at += offset;
        if source[at..].starts_with("<!--") {
            at += source[at + 4..].find("-->")? + 7;
            continue;
        }
        if lower[at..].starts_with("<!doctype ") || lower[at..].starts_with("<!doctype>") {
            let mut quote = None;
            loop {
                let byte = *bytes.get(at)?;
                at += 1;
                if quote == Some(byte) {
                    quote = None;
                } else if quote.is_none() {
                    if byte == b'\"' || byte == b'\'' {
                        quote = Some(byte);
                    } else if byte == b'>' {
                        break;
                    }
                }
            }
            continue;
        }
        at += 1;
        let closing = bytes.get(at) == Some(&b'/');
        at += usize::from(closing);
        let start = at;
        while bytes
            .get(at)
            .is_some_and(|b| b.is_ascii_alphanumeric() || b"-:_".contains(b))
        {
            at += 1;
        }
        if at == start {
            return None;
        }
        let name = &lower[start..at];
        // These contexts can suppress otherwise valid image tags. Refuse instead of risking
        // a coincidental count match with a differently encoded image elsewhere in the page.
        if !closing && matches!(name, "template" | "select") {
            return None;
        }
        let mut src_seen = false;
        loop {
            while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
                at += 1;
            }
            match bytes.get(at) {
                Some(b'>') => {
                    at += 1;
                    break;
                }
                Some(b'/') if bytes.get(at + 1) == Some(&b'>') => {
                    at += 2;
                    break;
                }
                None => return None,
                _ if closing => return None,
                _ => {}
            }
            let start = at;
            while bytes
                .get(at)
                .is_some_and(|b| !b.is_ascii_whitespace() && !b"=/>\"'<".contains(b))
            {
                at += 1;
            }
            if at == start {
                return None;
            }
            let attr = &lower[start..at];
            while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
                at += 1;
            }
            let image_src = name == "img" && attr == "src";
            if image_src && src_seen {
                return None;
            }
            src_seen |= image_src;
            if bytes.get(at) != Some(&b'=') {
                continue;
            }
            at += 1;
            while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
                at += 1;
            }
            let quote = bytes.get(at).copied().filter(|b| *b == b'"' || *b == b'\'');
            at += usize::from(quote.is_some());
            let start = at;
            if let Some(quote) = quote {
                while bytes.get(at).is_some_and(|b| *b != quote) {
                    at += 1;
                }
                bytes.get(at)?;
            } else {
                while bytes
                    .get(at)
                    .is_some_and(|b| !b.is_ascii_whitespace() && *b != b'>')
                {
                    at += 1;
                }
            }
            if image_src {
                values.push(start..at);
            }
            at += usize::from(quote.is_some());
        }
        if !closing
            && matches!(
                name,
                "script"
                    | "style"
                    | "textarea"
                    | "title"
                    | "xmp"
                    | "iframe"
                    | "noembed"
                    | "noframes"
                    | "plaintext"
            )
        {
            let end = format!("</{name}");
            loop {
                at += lower[at..].find(&end)?;
                if bytes
                    .get(at + end.len())
                    .is_some_and(|b| b.is_ascii_whitespace() || b"/>".contains(b))
                {
                    break;
                }
                at += end.len();
            }
        }
    }
    Some(values)
}

fn is_data_image(src: &str) -> bool {
    src.get(..11)
        .is_some_and(|start| start.eq_ignore_ascii_case("data:image/"))
}

pub(super) fn check(tree: &Tree, images: &[(Id, usize)], report: &mut Report) {
    let finding = |(id, ordinal): (Id, usize), code| Finding {
        element: "img".to_owned(),
        ordinal,
        id: tree.attr(id, "data-id").map(str::to_owned),
        line: tree.line(id),
        severity: Severity::Warning,
        code,
    };
    let mut total = 0;
    let mut over_total = false;
    let mut references = HashSet::new();
    let mut over_references = false;
    for image in images {
        let Some(src) = tree.attr(image.0, "src") else {
            continue;
        };
        if is_data_image(src) {
            total += src.len();
            if src.len() > DATA_IMAGE_BYTES {
                report
                    .findings
                    .push(finding(*image, "image-data-oversized"));
            }
            // One finding for the whole page, on the image that crosses the cap.
            if total > DATA_IMAGES_TOTAL_BYTES && !over_total {
                over_total = true;
                report.findings.push(finding(*image, "image-data-total"));
            }
        } else if src.starts_with(REFERENCE) {
            match reference_token(src) {
                None => report
                    .findings
                    .push(finding(*image, "attachment-ref-invalid")),
                Some(token) => {
                    if references.insert(src) {
                        report.references.push(Reference {
                            token: token.to_owned(),
                            ordinal: image.1,
                            line: tree.line(image.0),
                        });
                    }
                    if references.len() > ATTACHMENT_REFS && !over_references {
                        over_references = true;
                        report
                            .findings
                            .push(finding(*image, "attachment-ref-too-many"));
                    }
                }
            }
        }
    }
}
