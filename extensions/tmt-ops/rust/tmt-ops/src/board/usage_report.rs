//! A bounded background reader of public Core usage and message aggregates.
//! The shared TOK range and acquired UUID roster are its only demand; paint and
//! input project held evidence without starting commands.
mod acquire;
pub(in crate::board) mod model;
mod reader;
#[cfg(test)]
mod tests;

pub(super) use acquire::Request;
pub(super) use model::{Report, Since};
pub(super) use reader::{Delivery, Reader, Timing};

/// One held observation and its latest refresh failure.
pub enum View<'a> {
    Loading,
    Failed(&'a str),
    Report(&'a Report),
}

/// The range itself is saved by workspace preferences, never in this cache.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
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

    pub fn refresh_error(&self, since: Since) -> Option<&str> {
        self.failure
            .as_ref()
            .filter(|(range, _)| *range == since)
            .map(|(_, message)| message.as_str())
    }

    pub fn view_for(&self, since: Since) -> View<'_> {
        if let Some(report) = self.report.as_ref().filter(|report| report.since == since) {
            return View::Report(report);
        }
        match &self.failure {
            Some((failed_since, message)) if *failed_since == since => View::Failed(message),
            _ => View::Loading,
        }
    }
}
