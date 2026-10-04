//! Grapheme fitting for markup and Squad board text; CLI lists keep their fitter.
use crate::{geometry::Space, style::TextFlow};
use tmt_cli_style::{grid::Align, table::escape};
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
fn fit_escaped(text: &str, width: usize, middle: bool, ellipsis: bool, align: Align) -> String {
    if text.width() <= width {
        let padding = width - text.width();
        let before = match align {
            Align::Left => 0,
            Align::Right => padding,
            Align::Center => padding / 2,
        };
        return format!(
            "{}{text}{}",
            " ".repeat(before),
            " ".repeat(padding - before)
        );
    }
    let fitted = if !ellipsis || width == 0 {
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
/// One escaped visual line; alignment is within the recorded text budget.
pub fn fit_line(text: &str, width: u16, flow: TextFlow, align: Align) -> String {
    fit_escaped(
        &escape(text),
        usize::from(width),
        flow == TextFlow::Middle,
        flow != TextFlow::Clip,
        align,
    )
}
/// Logical visual lines use geometry's recorded budget, before viewport cuts.
pub fn lines(text: &str, width: u16, flow: TextFlow) -> Vec<String> {
    fit_lines(text, width, flow, Align::Left)
}
/// Bounded logical lines, aligned by the application without another fitting policy.
pub fn fit_lines(text: &str, width: u16, flow: TextFlow, align: Align) -> Vec<String> {
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
        let head = prefix(remaining, width);
        if !wrapping || head.len() == remaining.len() || index + 1 == limit {
            result.push(fit_escaped(
                remaining,
                width,
                flow == TextFlow::Middle,
                flow != TextFlow::Clip,
                align,
            ));
            break;
        }
        if head.is_empty() {
            result.push(fit_escaped(remaining, width, false, true, align));
            break;
        }
        let split = head
            .grapheme_indices(true)
            .rev()
            .find(|(at, g)| *at > 0 && g.chars().all(char::is_whitespace))
            .map_or(head.len(), |(at, _)| at);
        result.push(fit_escaped(
            remaining[..split].trim_end(),
            width,
            false,
            false,
            align,
        ));
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
