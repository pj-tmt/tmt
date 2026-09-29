//! Column widths for rows that never wrap, and cells fitted to them. One
//! pure solver serves every aligned view: `tmt ls` style lists through
//! [`crate::table`], and extension boards and their text output directly.
//!
//! Widths are display cells: wide characters (CJK, most emoji) count two.
//! The solver only decides widths; drawing and color belong to the caller.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// How a cell sits in its width when the text is shorter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Right,
    Center,
}

/// Where text that does not fit is cut; the cut shows as `…`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Truncate {
    #[default]
    End,
    /// Keeps both ends, for paths and links.
    Middle,
}

/// One column's sizing, in display cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Track {
    /// The width it wants: a configured width, or its widest content.
    pub basis: usize,
    /// It never shrinks below this. A basis below `min` is raised to it.
    pub min: usize,
    pub max: Option<usize>,
    /// Its share of the width left over once every column has its basis;
    /// 0 never grows.
    pub grow: u16,
    /// Shrinking order: every column of a lower tier reaches its minimum
    /// before a higher tier shrinks. Within a tier the widest shrinks first.
    pub shrink: u8,
    /// Dropping order when even the minimums do not fit: the highest
    /// priority number steps aside first. None never steps aside.
    pub priority: Option<u16>,
}

impl Track {
    /// Exactly `width` cells: never shrinks or grows.
    pub fn fixed(width: usize) -> Self {
        Self {
            basis: width,
            min: width,
            max: Some(width),
            grow: 0,
            shrink: 0,
            priority: None,
        }
    }

    fn start(&self) -> usize {
        let width = self.basis.max(self.min);
        self.max.map_or(width, |max| width.min(max.max(self.min)))
    }
}

/// Each column's width, or None where it stepped aside. With no known width
/// every column keeps its basis and nothing is cut, dropped or grown.
///
/// Columns first shrink toward their minimums, tier by tier. If they still do
/// not fit, the column with the highest `priority` steps aside and the rest
/// are laid out again from their basis. Width left over goes to growing
/// columns in proportion to `grow`, up to their `max`.
pub fn solve(tracks: &[Track], available: Option<usize>, gap: usize) -> Vec<Option<usize>> {
    let Some(available) = available else {
        return tracks.iter().map(|track| Some(track.start())).collect();
    };
    let mut shown = vec![true; tracks.len()];
    loop {
        let mut widths: Vec<usize> = tracks.iter().map(Track::start).collect();
        shrink(tracks, &shown, &mut widths, available, gap);
        let over = total(&widths, &shown, gap) > available;
        let drop = tracks
            .iter()
            .enumerate()
            .filter(|(index, track)| shown[*index] && track.priority.is_some())
            .max_by_key(|(index, track)| (track.priority, *index))
            .map(|(index, _)| index);
        match (over, drop) {
            (true, Some(index)) => shown[index] = false,
            _ => {
                grow(tracks, &shown, &mut widths, available, gap);
                return widths
                    .into_iter()
                    .zip(shown)
                    .map(|(width, shown)| shown.then_some(width))
                    .collect();
            }
        }
    }
}

fn total(widths: &[usize], shown: &[bool], gap: usize) -> usize {
    let (sum, count) = widths
        .iter()
        .zip(shown)
        .filter(|(_, shown)| **shown)
        .fold((0usize, 0usize), |(sum, count), (width, _)| {
            (sum + width, count + 1)
        });
    sum + gap * count.saturating_sub(1)
}

/// Tier by tier, the widest shrinkable column gives way to the next widest
/// of its tier (or its own minimum), until the columns fit.
fn shrink(tracks: &[Track], shown: &[bool], widths: &mut [usize], available: usize, gap: usize) {
    let mut tiers: Vec<u8> = tracks.iter().map(|track| track.shrink).collect();
    tiers.sort_unstable();
    tiers.dedup();
    for tier in tiers {
        let in_tier = |index: usize| shown[index] && tracks[index].shrink == tier;
        while total(widths, shown, gap) > available {
            let widest = (0..widths.len())
                .filter(|&index| in_tier(index) && widths[index] > tracks[index].min)
                .max_by_key(|&index| (widths[index], std::cmp::Reverse(index)));
            let Some(index) = widest else { break };
            let excess = total(widths, shown, gap) - available;
            // Only columns that can still shrink set the next level, so a
            // column at its minimum never holds back the widest.
            let next = (0..widths.len())
                .filter(|&other| {
                    other != index && in_tier(other) && widths[other] > tracks[other].min
                })
                .map(|other| widths[other])
                .filter(|&width| width < widths[index])
                .max()
                .unwrap_or(0)
                .max(tracks[index].min);
            widths[index] -= excess.min(widths[index] - next).max(1);
        }
    }
}

/// Leftover width in proportion to `grow`, capped at each `max`; what
/// rounding leaves goes one cell at a time from the left.
fn grow(tracks: &[Track], shown: &[bool], widths: &mut [usize], available: usize, gap: usize) {
    loop {
        let left = available.saturating_sub(total(widths, shown, gap));
        let growing: Vec<usize> = (0..widths.len())
            .filter(|&index| {
                shown[index]
                    && tracks[index].grow > 0
                    && tracks[index].max.is_none_or(|max| widths[index] < max)
            })
            .collect();
        if left == 0 || growing.is_empty() {
            return;
        }
        let shares: usize = growing
            .iter()
            .map(|&index| usize::from(tracks[index].grow))
            .sum();
        let mut given = 0;
        for &index in &growing {
            let room = tracks[index]
                .max
                .map_or(usize::MAX, |max| max - widths[index]);
            let share = (left * usize::from(tracks[index].grow) / shares).min(room);
            widths[index] += share;
            given += share;
        }
        if given == 0 {
            for &index in growing.iter().take(left) {
                widths[index] += 1;
            }
        }
    }
}

/// The width of a cell spanning `columns`: the shown columns' widths and the
/// gaps between them. Zero when every spanned column stepped aside.
pub fn span(widths: &[Option<usize>], columns: std::ops::Range<usize>, gap: usize) -> usize {
    let shown: Vec<usize> = widths[columns].iter().flatten().copied().collect();
    shown.iter().sum::<usize>() + gap * shown.len().saturating_sub(1)
}

/// Exactly `width` display cells of escaped text (control and line-separator
/// characters shown escaped, as in [`crate::table::escape`]): padded by
/// `align`, or cut with `…` where
/// `truncate` says. A wide character that would cross the cut is left out
/// and the cell padded, so every result is exactly `width` cells.
pub fn fit(text: &str, width: usize, align: Align, truncate: Truncate) -> String {
    // Cells hold user and agent data: control characters never reach the
    // terminal (escaping is idempotent, so escaped input is unchanged).
    let text = crate::table::escape(text);
    let used = text.width();
    if used <= width {
        let pad = width - used;
        let (before, after) = match align {
            Align::Left => (0, pad),
            Align::Right => (pad, 0),
            Align::Center => (pad / 2, pad - pad / 2),
        };
        return format!("{}{text}{}", " ".repeat(before), " ".repeat(after));
    }
    if width == 0 {
        return String::new();
    }
    let room = width - 1;
    match truncate {
        Truncate::End => {
            let front = head(&text, room);
            format!("{front}…{}", " ".repeat(room - front.width()))
        }
        // The pad sits at the cut, so the kept ends stay at the edges.
        Truncate::Middle => {
            let front = head(&text, room - room / 2);
            let back = tail(&text, room / 2);
            let pad = room - front.width() - back.width();
            format!("{front}{}…{back}", " ".repeat(pad))
        }
    }
}

/// The longest prefix at most `cells` wide.
fn head(text: &str, cells: usize) -> &str {
    let mut used = 0;
    for (index, character) in text.char_indices() {
        used += character.width().unwrap_or(0);
        if used > cells {
            return &text[..index];
        }
    }
    text
}

/// The longest suffix at most `cells` wide.
fn tail(text: &str, cells: usize) -> &str {
    let mut used = 0;
    for (index, character) in text.char_indices().rev() {
        used += character.width().unwrap_or(0);
        if used > cells {
            return &text[index + character.len_utf8()..];
        }
    }
    text
}

#[cfg(test)]
mod tests;
