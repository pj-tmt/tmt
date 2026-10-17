//! Cached Overview strips over the held view; body geometry belongs to the workspace.

use crate::{
    board::app::{App, HomeHeaderUsage, View},
    look::Look,
};
use ratatui::{layout::Rect, text::Line};

/// The strips outside the body, held in the view like the body's sections.
/// Its working/idle counts follow the board's activity stale rule.
pub(crate) fn summary_of(app: &App, view: &View, width: u16, look: Look) -> Line<'static> {
    let home = view.home.as_ref().expect("a home view has a model");
    super::bar::summary_in(
        &mut view.derived.borrow_mut().home.summary,
        home,
        width,
        look,
        app.activity_stale(std::time::Instant::now()),
    )
}

pub(crate) fn usage_of(
    app: &App,
    usage: &HomeHeaderUsage<'_>,
    width: u16,
    look: Look,
) -> Option<Line<'static>> {
    let view = app.view.as_ref().unwrap();
    let mut digits = app.home_header_digits(usage, std::time::Instant::now());
    let keys = app.home_header_counter_keys();
    let slots = super::bar::usage_slots(usage, width);
    for &(index, _, cell) in &slots {
        digits[index] = app
            .home_counters
            .borrow_mut()
            .admitted_digits(&keys[index], cell);
    }
    let line = super::bar::usage_in(
        &mut view.derived.borrow_mut().home.usage,
        usage,
        width,
        look,
        Some(&digits),
    );
    if line.is_some() && !view.history_pending {
        for (index, x, cell) in slots {
            app.home_counters.borrow_mut().visible(
                &keys[index],
                Rect::new(x, 0, cell, 1),
                String::new(),
                tmt_cli_style::grid::Align::Right,
                false,
            );
        }
    }
    line
}

/// The provider-limits strip at every width, only while the usage meter is
/// on, since that switch also drives the sampling that feeds it.
pub(in crate::board) fn limits_enabled(app: &App) -> bool {
    app.view.as_ref().is_some_and(|view| {
        view.home.is_some() && view.home_rate.values().any(|rate| rate.settings.enabled)
    })
}

pub(crate) fn limits_of(app: &App, width: u16, look: Look) -> Vec<Line<'static>> {
    let Some(view) = app.view.as_ref() else {
        return Vec::new();
    };
    if !limits_enabled(app) {
        return Vec::new();
    }
    super::limits::limits_in(
        &mut view.derived.borrow_mut().home.limits,
        app.limits.as_deref(),
        crate::status::now_ms(),
        width,
        look,
    )
}

/// The strips painted from nothing, for tests of the strips themselves.
#[cfg(test)]
pub(crate) use super::bar::{summary, usage};
