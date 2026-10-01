//! Shared column widths and bounded cell fitting for aligned views. One
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

/// A column's preferred width, in cells or a share of the data width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    Cells(usize),
    Percent(u8),
}

/// Ellipsis is the default; wrapping is bounded by the caller's line limit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Overflow {
    #[default]
    Ellipsis,
    Wrap {
        max_lines: u8,
    },
}

/// One column's sizing, in display cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Track {
    /// The width it wants: a configured width, or its widest content.
    pub basis: Basis,
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
            basis: Basis::Cells(width),
            min: width,
            max: Some(width),
            grow: 0,
            shrink: 0,
            priority: None,
        }
    }

    fn start(&self, basis: usize) -> usize {
        let width = basis.max(self.min);
        self.max.map_or(width, |max| width.min(max.max(self.min)))
    }
}

/// Each column's width, or None where it stepped aside. With no known width
/// cell-basis columns keep their basis and nothing is cut, dropped or grown.
/// A percent caller supplies a known or content-derived width budget.
///
/// Columns first shrink toward their minimums, tier by tier. If they still do
/// not fit, the column with the highest `priority` steps aside and the rest
/// are laid out again from their basis. Width left over goes to growing
/// columns in proportion to `grow`, up to their `max`.
pub fn solve(tracks: &[Track], available: Option<usize>, gap: usize) -> Vec<Option<usize>> {
    let Some(available) = available else {
        return tracks
            .iter()
            .map(|track| {
                Some(track.start(match track.basis {
                    Basis::Cells(width) => width,
                    Basis::Percent(_) => 0,
                }))
            })
            .collect();
    };
    let mut shown = vec![true; tracks.len()];
    loop {
        let bases = bases(tracks, &shown, available, gap);
        let mut widths: Vec<usize> = tracks
            .iter()
            .zip(bases)
            .map(|(track, basis)| track.start(basis))
            .collect();
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

/// Resolve shares after separators, before bounds/grow. Largest remainders
/// receive the remaining cells; ties go left. Hiding reruns this on the shown set.
fn bases(tracks: &[Track], shown: &[bool], available: usize, gap: usize) -> Vec<usize> {
    let count = shown.iter().filter(|shown| **shown).count();
    let data = available.saturating_sub(gap.saturating_mul(count.saturating_sub(1)));
    let mut result = vec![0; tracks.len()];
    let mut fractions = Vec::new();
    let mut percentages = 0u128;
    let mut assigned = 0usize;
    for (index, track) in tracks.iter().enumerate().filter(|(index, _)| shown[*index]) {
        match track.basis {
            Basis::Cells(width) => result[index] = width,
            Basis::Percent(percent) => {
                let product = data as u128 * u128::from(percent);
                result[index] = usize::try_from(product / 100).unwrap_or(usize::MAX);
                fractions.push((std::cmp::Reverse(product % 100), index));
                percentages += u128::from(percent);
                assigned = assigned.saturating_add(result[index]);
            }
        }
    }
    fractions.sort_unstable();
    let target = usize::try_from(data as u128 * percentages / 100).unwrap_or(usize::MAX);
    for (_, index) in fractions.into_iter().take(target.saturating_sub(assigned)) {
        result[index] = result[index].saturating_add(1);
    }
    result
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

/// Fit each visual line through the same escaping/alignment/cut owner. Wrap
/// prefers whitespace boundaries, hard-splits long words, and uses an end
/// ellipsis on the last bounded line. Continuations start at the cell's edge.
pub fn fit_lines(
    text: &str,
    width: usize,
    align: Align,
    truncate: Truncate,
    overflow: Overflow,
) -> Vec<String> {
    let Overflow::Wrap { max_lines } = overflow else {
        return vec![fit(text, width, align, truncate)];
    };
    if width == 0 {
        return vec![String::new()];
    }
    let escaped = crate::table::escape(text);
    let mut remaining = escaped.as_str();
    let mut fitted = Vec::new();
    for index in 0..usize::from(max_lines.max(1)) {
        if remaining.width() <= width || index + 1 == usize::from(max_lines.max(1)) {
            fitted.push(fit(remaining, width, align, Truncate::End));
            break;
        }
        let prefix = head(remaining, width);
        if prefix.is_empty() {
            // A wide scalar cannot fit even by itself. A bounded cut keeps
            // the exact cell width without retrying the same scalar forever.
            fitted.push(fit(remaining, width, align, Truncate::End));
            break;
        }
        let split = prefix
            .char_indices()
            .rev()
            .find(|(index, character)| *index > 0 && character.is_whitespace())
            .map_or(prefix.len(), |(index, _)| index);
        fitted.push(fit(
            remaining[..split].trim_end(),
            width,
            align,
            Truncate::End,
        ));
        remaining = remaining[split..].trim_start();
    }
    fitted
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
