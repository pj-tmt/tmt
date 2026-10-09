//! A fixed status line with caller-owned text, elapsed age and animation frames.
//! Painting never acquires time or changes the surrounding layout.
use super::strip;
use crate::{style::TextFlow, text};
use ratatui::{buffer::Buffer, layout::Rect, text::Line};
use std::{borrow::Cow, time::Duration};
use tmt_cli_style::{Depth, Role, Theme, grid::Align, theme::screen};

const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// The application supplies either complete display text or an elapsed age.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusLabel<'a> {
    Text(&'a str),
    /// A caller-owned prefix followed by the floored age and English `ago`.
    /// An empty prefix produces only the age, without a leading space.
    Age {
        prefix: &'a str,
        age: Duration,
    },
}

/// A single line within an application-reserved rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusSlot<'a> {
    pub label: StatusLabel<'a>,
    /// `None` hides the busy marker; `Some` selects a frame modulo ten.
    /// The caller owns cadence, delayed appearance and reduced motion.
    pub frame: Option<usize>,
}

// All age units and English wording live here; callers can supply preformatted
// Text instead. Duration cannot be negative and no wall clock is consulted.
fn age_label(prefix: &str, age: Duration) -> String {
    let seconds = age.as_secs();
    let (value, unit) = if seconds < 60 {
        (seconds, "s")
    } else if seconds < 3600 {
        (seconds / 60, "m")
    } else if seconds < 86400 {
        (seconds / 3600, "h")
    } else {
        (seconds / 86400, "d")
    };
    let gap = if prefix.is_empty() { "" } else { " " };
    format!("{prefix}{gap}{value}{unit} ago")
}

impl StatusSlot<'_> {
    /// Replace only the first row of `area`, including its trailing blanks.
    /// The caller presents the completed buffer with its other components.
    ///
    /// `Depth::None` (NO_COLOR) also disables animation: every busy frame uses
    /// steady `[busy]` text. A marker that cannot fit whole is omitted. Labels
    /// are escaped and clipped at grapheme boundaries by the existing fitter.
    pub fn paint(&self, buffer: &mut Buffer, area: Rect, theme: &Theme, depth: Depth) {
        if area.is_empty() {
            return;
        }
        let row = Rect::new(area.x, area.y, area.width, 1);
        let visible = row.intersection(buffer.area);
        if visible.is_empty() {
            return;
        }
        let style = screen::style(theme, Role::Dim, depth);
        for x in visible.left()..visible.right() {
            buffer[(x, visible.y)].reset();
            buffer[(x, visible.y)].set_style(style);
        }
        let label = match self.label {
            StatusLabel::Text(text) => Cow::Borrowed(text),
            StatusLabel::Age { prefix, age } => Cow::Owned(age_label(prefix, age)),
        };
        let mut label_area = row;
        if let Some(frame) = self.frame {
            let (marker, width) = if depth == Depth::None {
                ("[busy]", 6)
            } else {
                (FRAMES[frame % FRAMES.len()], 1)
            };
            let reserved = width + u16::from(!label.is_empty());
            let marker_area = Rect::new(row.x, row.y, width, 1);
            if reserved <= row.width && marker_area.intersection(buffer.area) == marker_area {
                strip::paint_left(buffer, marker_area, Line::raw(marker).style(style));
                label_area.x += reserved;
                label_area.width -= reserved;
            }
        }
        let fitted = text::fit_line(&label, label_area.width, TextFlow::Clip, Align::Left);
        strip::paint_left(buffer, label_area, Line::raw(fitted).style(style));
    }
}

#[cfg(test)]
mod tests;
