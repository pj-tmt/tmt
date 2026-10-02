//! Markup-only grapheme fitting; measurement and paint share this owner.
use crate::{geometry::Space, style::TextFlow};
use tmt_cli_style::table::escape;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

fn prefix(text: &str, width: usize) -> &str {
    let mut used = 0;
    for (at, grapheme) in text.grapheme_indices(true) {
        used += grapheme.width();
        if used > width {
            return &text[..at];
        }
    }
    text
}
fn suffix(text: &str, width: usize) -> &str {
    let mut used = 0;
    for (at, grapheme) in text.grapheme_indices(true).rev() {
        used += grapheme.width();
        if used > width {
            return &text[at + grapheme.len()..];
        }
    }
    text
}
/// Fit an already escaped visual line; never split a grapheme.
pub(crate) fn fit(text: &str, width: usize, middle: bool, ellipsis: bool) -> String {
    let fitted = if text.width() <= width {
        text.to_owned()
    } else if !ellipsis || width == 0 {
        prefix(text, width).to_owned()
    } else if middle {
        let room = width - 1;
        let front = prefix(text, room - room / 2);
        let back = suffix(text, room / 2);
        format!(
            "{front}{}…{back}",
            " ".repeat(room - front.width() - back.width())
        )
    } else {
        format!("{}…", prefix(text, width - 1))
    };
    format!("{fitted}{}", " ".repeat(width - fitted.width()))
}
/// Logical visual lines use geometry's recorded budget, before viewport cuts.
pub fn lines(text: &str, width: u16, flow: TextFlow) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let escaped = escape(text);
    let width = usize::from(width);
    let limit = match flow {
        TextFlow::Wrap => usize::from(u16::MAX),
        TextFlow::Clamp(n) => usize::from(n),
        _ => 1,
    };
    let wrapping = matches!(flow, TextFlow::Wrap | TextFlow::Clamp(_));
    let mut remaining = escaped.as_str();
    let mut result = Vec::new();
    for index in 0..limit {
        if !wrapping || remaining.width() <= width || index + 1 == limit {
            result.push(fit(
                remaining,
                width,
                flow == TextFlow::Middle,
                flow != TextFlow::Clip,
            ));
            break;
        }
        let head = prefix(remaining, width);
        if head.is_empty() {
            result.push(fit(remaining, width, false, true));
            break;
        }
        let split = head
            .grapheme_indices(true)
            .rev()
            .find(|(at, g)| *at > 0 && g.chars().all(char::is_whitespace))
            .map_or(head.len(), |(at, _)| at);
        result.push(fit(remaining[..split].trim_end(), width, false, false));
        remaining = remaining[split..].trim_start();
    }
    result
}
/// Same escaping, widths and bounded wrapping as `lines`.
pub fn measure(text: &str, flow: TextFlow, space: Space) -> [u16; 2] {
    let escaped = escape(text);
    let natural = escaped.width().min(usize::from(u16::MAX)) as u16;
    let width = match space {
        Space::Cells(n) => n,
        Space::MaxContent => natural,
        Space::MinContent => escaped
            .split_whitespace()
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or(0)
            .min(usize::from(u16::MAX)) as u16,
    };
    [natural.min(width), lines(text, width, flow).len() as u16]
}
