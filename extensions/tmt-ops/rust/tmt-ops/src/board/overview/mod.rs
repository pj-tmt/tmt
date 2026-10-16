//! Overview modules retain separate monitor and chart ownership.
pub(in crate::board) mod chart;
pub(super) mod cron_counts;

pub(super) mod actions;
pub(super) mod columns;
pub(super) mod composition;
pub(super) mod detail;
pub(super) mod digest;
pub(super) mod monitor;
pub(super) mod quota;
pub(super) mod sort;
pub(super) mod summary;
pub(super) mod timers;
pub(super) mod view;

#[cfg(test)]
mod capture;

#[cfg(test)]
mod performance;
