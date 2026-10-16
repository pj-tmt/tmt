//! The Digest tab's Usage report panel: the document `tmt digest usage report
//! --json` prints, shown as the supplier decided it. A dedicated reader acquires
//! it off the paint path, only while the panel is on screen; paint and input
//! read nothing. Wording and layout belong to the panel module.
mod model;
mod paint;
mod panel;
mod reader;
#[cfg(test)]
mod tests;

pub(super) use model::{Report, Since};
pub(super) use paint::{paint, zone};
pub(super) use panel::{View, rows};
pub(super) use reader::{Delivery, Reader, Timing};

/// What the board holds for the panel: the window it shows and the newest read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub since: Since,
    report: Option<Report>,
    failure: Option<(Since, String)>,
}

impl State {
    /// Keeps the last good report when a later read fails; the rule's own time
    /// says how old it is.
    pub fn apply(&mut self, delivery: Delivery) {
        match delivery.result {
            Ok(report) => {
                self.failure = None;
                self.report = Some(report);
            }
            Err(message) => self.failure = Some((delivery.since, message)),
        }
    }

    pub fn view(&self) -> View<'_> {
        if let Some(report) = self
            .report
            .as_ref()
            .filter(|report| report.since == self.since)
        {
            return View::Report(report);
        }
        match &self.failure {
            Some((since, message)) if *since == self.since => View::Failed(message),
            _ => View::Loading,
        }
    }
}
