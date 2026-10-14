//! Overview modules retain separate monitor and chart ownership.
pub(in crate::board) mod chart;
pub(super) mod cron_counts;

pub(super) mod actions;
pub(super) mod columns;
pub(super) mod detail;
pub(super) mod monitor;
pub(super) mod sort;
pub(super) mod view;

#[cfg(test)]
mod capture;

#[cfg(test)]
mod performance;
