//! One agent turn can answer several sends: the ones typed into the pane while it was still
//! working are read with the first. Core binds the turn's final to the first send only, so the
//! board paints that answer after the last send the turn covers, not under the first.
use crate::board::row_detail::Capture;

/// What one send tells the ordering: when it was sent and what Core captured for it.
pub(super) struct Probe<'a> {
    pub at_ms: u64,
    pub capture: Option<&'a Capture>,
}

/// Where a send's part of the transcript goes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Place {
    /// The send is in a group whose answer is painted under the group's last send.
    pub answered_below: bool,
    /// The send was typed during an earlier send's turn; that turn answers it too, so its own
    /// missing capture says nothing.
    pub mate: bool,
    /// Its own answer is painted under a later send of the group, not under it.
    pub hoisted: bool,
    /// Sends of its group whose answers are painted under this one.
    pub answers: Vec<usize>,
}

/// A send that has no capture of its own: Core bound no turn to it.
fn unanswered(probe: &Probe) -> bool {
    matches!(probe.capture, None | Some(Capture::NoAnswer))
}

/// Orders `sends` (oldest first, so the scan ends where a turn settled). A send with an answered turn takes every later send that has no
/// capture of its own and was sent before the turn settled as its group; the answer follows the
/// group's last send.
pub(super) fn place(sends: &[Probe]) -> Vec<Place> {
    let mut places = vec![Place::default(); sends.len()];
    for (index, send) in sends.iter().enumerate() {
        let Some(Capture::Answered {
            settled_ms: Some(settled),
            ..
        }) = send.capture
        else {
            continue;
        };
        // `sends` are oldest first, so the scan ends where the turn settled.
        let mates: Vec<usize> = (index + 1..sends.len())
            .take_while(|later| sends[*later].at_ms < *settled)
            .filter(|later| unanswered(&sends[*later]) && sends[*later].at_ms > send.at_ms)
            .collect();
        let Some(last) = mates.last().copied() else {
            continue;
        };
        places[index].hoisted = true;
        places[last].answers.push(index);
        for mate in &mates {
            places[*mate].mate = true;
        }
        for member in std::iter::once(index).chain(mates).filter(|m| *m != last) {
            places[member].answered_below = true;
        }
    }
    places
}

/// The delivered label of a send whose answer is painted below it.
pub(super) fn answered_below(label: &str) -> String {
    match label {
        "✓ delivered" => "✓ delivered · answered below".into(),
        other => other.into(),
    }
}

#[cfg(test)]
mod tests;
