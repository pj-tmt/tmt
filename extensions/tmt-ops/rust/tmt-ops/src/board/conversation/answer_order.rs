//! Consecutive direct sends that one agent turn answers read as one message block: a single
//! header, the bodies stacked in order, one status line and then the answer. Core binds the
//! turn's final to the first send only (the ones typed while it worked report `no_answer`), so
//! the board groups them itself and paints the answer under the block's status.
use crate::board::row_detail::Capture;

/// What one send tells the grouping: when it was sent, what Core captured for it, and whether
/// it is a direct (passthrough) message at all.
pub(super) struct Probe<'a> {
    pub at_ms: u64,
    pub capture: Option<&'a Capture>,
    /// Only direct messages group; a request, an annotation, a failed send or a send that an
    /// agent question or reply came between never joins a block.
    pub direct: bool,
}

/// Where a send stands in its block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Role {
    /// Alone: header, body, status and its own answer.
    #[default]
    Single,
    /// Opens a block: the header and the body.
    First,
    /// Inside a block: the body alone.
    Member,
    /// Closes a block: the body, the block's status and then its answer.
    Last,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Place {
    pub role: Role,
    /// On the last send: the sends whose answered turn is painted under the block's status.
    pub answers: Vec<usize>,
    /// The send holds no answer line of its own: its turn is the block's, painted by the last.
    pub grouped: bool,
}

impl Place {
    pub fn header(&self) -> bool {
        matches!(self.role, Role::Single | Role::First)
    }
    pub fn status(&self) -> bool {
        matches!(self.role, Role::Single | Role::Last)
    }
}

/// A send that has no capture of its own that ended a turn: Core bound none to it, or has not
/// yet decided.
fn free(probe: &Probe) -> bool {
    matches!(
        probe.capture,
        None | Some(Capture::NoAnswer | Capture::Pending)
    )
}

/// Whether `later` belongs to the block `first` opened: `first`'s turn is still open, or its
/// answered window had not settled when `later` was sent.
fn joins(first: &Probe, later: &Probe) -> bool {
    if !later.direct || !free(later) || later.at_ms <= first.at_ms {
        return false;
    }
    match first.capture {
        // Sent just now: its capture has not been read yet.
        None => true,
        Some(Capture::Pending | Capture::Bound) => true,
        Some(Capture::Answered {
            settled_ms: Some(settled),
            ..
        }) => later.at_ms < *settled,
        _ => false,
    }
}

/// Places `sends` (oldest first, adjacent in the transcript, so the scan ends where a turn
/// settled).
pub(super) fn place(sends: &[Probe]) -> Vec<Place> {
    let mut places = vec![Place::default(); sends.len()];
    let mut index = 0;
    while index < sends.len() {
        let first = &sends[index];
        let mut last = index;
        if first.direct {
            while last + 1 < sends.len() && joins(first, &sends[last + 1]) {
                last += 1;
            }
        }
        if last > index {
            for (member, place) in places.iter_mut().enumerate().take(last + 1).skip(index) {
                place.grouped = true;
                place.role = match member {
                    m if m == index => Role::First,
                    m if m == last => Role::Last,
                    _ => Role::Member,
                };
            }
            places[last].answers = (index..=last)
                .filter(|member| matches!(sends[*member].capture, Some(Capture::Answered { .. })))
                .collect();
        }
        index = last + 1;
    }
    places
}

/// The one status line of a block, from its members' lines oldest first: the newest line that
/// is not a plain delivery (the turn still working, a send held or queued), else `✓ delivered`.
pub(super) fn block_status<'a>(lines: impl DoubleEndedIterator<Item = &'a str>) -> String {
    let mut newest = None;
    for line in lines.rev() {
        newest.get_or_insert(line);
        if line != "✓ delivered" {
            return line.to_owned();
        }
    }
    newest.unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests;
