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
