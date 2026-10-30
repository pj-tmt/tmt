//! Fit whole metric tracks; the hourly strip never changes its clock granularity.
use super::metrics;
use crate::board::{
    app::{App, Hit, HitTarget},
    exchange_matrix,
    view::fit,
    workspace::OpsTreeRow,
};
use crate::config::{Pane, SavingsUnit};
use crate::labels::digest::SavingsSummary;
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use std::rc::Rc;
use tmt_cli_style::{Role, theme::screen};
use tmt_tui::components::strip;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    Agent,
    State,
    Activity,
    Tokens,
    Share,
    /// The supplier's saved estimate in the shared savings unit.
    Saved,
    /// Observed input at list price.
    Cost,
    /// The saved estimate at list price; steps aside while [`Field::Saved`] already shows money.
    SavedCost,
    Digest,
    Talks,
}

/// Columns a pane may leave out for width, in the order the hint names them.
const OPTIONAL: [(Field, &str); 5] = [
    (Field::Activity, "activity"),
    (Field::Share, "share"),
    (Field::Cost, "price"),
    (Field::SavedCost, "saved price"),
    (Field::Talks, "recipients"),
];

pub(super) fn tracks(width: u16, unit: SavingsUnit) -> Vec<(Field, u16)> {
    use Field::*;
    let wide = width >= 94;
    let mut fields = vec![
        (Agent, if wide { 12 } else { 14 }),
        (State, if wide { 12 } else { 13 }),
    ];
    // An 80-column terminal keeps the hourly strip; AGENT keeps nine cells there.
    if width >= 76 {
        fields.push((Activity, 24));
    }
    fields.extend([(Tokens, if wide { 7 } else { 8 }), (Saved, 7)]);
    if wide {
        fields.push((Cost, 8));
        if unit != SavingsUnit::Cost {
            fields.push((SavedCost, 9));
        }
    }
    fields.push((Digest, 10));
    // Share, then recipients, come back once they fit beside full-width agent names; the
    // Exchanges matrix beside the list keeps showing who talks to whom.
    if wide && span(&fields) + 6 <= width {
        let at = fields.iter().position(|(field, _)| *field == Saved);
        fields.insert(at.unwrap_or(fields.len()), (Share, 5));
    }
    if wide && span(&fields) + 10 <= width {
        fields.push((Talks, 9));
    }
    // Agent names yield first. Metrics are either whole or omitted.
    fields[0].1 = fields[0]
        .1
        .saturating_sub(span(&fields).saturating_sub(width));
    let mut used = 0;
    fields.retain(|(_, w)| {
        let fits = *w > 0 && used + *w <= width;
        if fits {
            used += *w + 1;
        }
        fits
    });
    fields
}

/// Cells the tracks take with one-cell gaps.
fn span(fields: &[(Field, u16)]) -> u16 {
    fields
        .iter()
        .map(|(_, w)| w + 1)
        .sum::<u16>()
        .saturating_sub(1)
}

/// Names the columns `width` leaves out and the width that shows them all, or counts them when
/// the names do not fit `room`; `None` when none is missing. A unit choice is not a missing
/// column: ≈ $ SAVED is only named while it can show.
pub(super) fn narrow_hint(width: u16, unit: SavingsUnit, room: usize) -> Option<String> {
    let shown = tracks(width, unit);
    let all = tracks(u16::MAX, unit);
    let missing: Vec<_> = OPTIONAL
        .iter()
        .filter(|(field, _)| {
            all.iter().any(|(other, _)| other == field)
                && !shown.iter().any(|(other, _)| other == field)
        })
        .map(|(_, name)| *name)
        .collect();
    let (last, rest) = missing.split_last()?;
    let names = if rest.is_empty() {
        (*last).to_owned()
    } else {
        format!("{} and {last}", rest.join(", "))
    };
    let named = format!("Widen to {} columns for {names}.", span(&all));
    if unicode_width::UnicodeWidthStr::width(named.as_str()) <= room || rest.is_empty() {
        Some(named)
    } else {
        Some(format!(
            "Widen to {} columns for {} more columns.",
            span(&all),
            missing.len()
        ))
    }
}
/// The width the table's tracks fit in this frame; `None` while the list is not painted.
pub(super) fn painted_width(app: &App) -> Option<u16> {
    app.digest_placement(app.conversation.borrow().area)
        .list
        .map(crate::board::pane_chrome::content_area)
        .filter(|inside| !inside.is_empty())
        .map(|inside| inside.width)
}
/// Whether the painted table carries the hourly strip; Details shows it when it does not.
pub(super) fn activity_shown(app: &App) -> bool {
    painted_width(app).is_some_and(|width| {
        tracks(width, app.savings_unit())
            .iter()
            .any(|(field, _)| matches!(field, Field::Activity))
    })
}
pub(super) fn price_column_shown(app: &App) -> bool {
    painted_width(app).is_some_and(|width| {
        tracks(width, app.savings_unit())
            .iter()
            .any(|(field, _)| matches!(field, Field::Cost))
    })
}
pub(super) fn hourly(app: &App, values: &[Option<u64>], selected: bool) -> Vec<Span<'static>> {
    let max = values.iter().flatten().copied().max().unwrap_or(0);
    values
        .iter()
        .map(|value| {
            let empty_style = if selected {
                app.look().selection()
            } else {
                app.look().role(Role::Dim)
            };
            let (text, style) = match value {
                None => ("?", empty_style),
                Some(0) => ("·", empty_style),
                Some(n) => {
                    let level = exchange_matrix::level(*n, max).unwrap_or(0);
                    let style = screen::neutral_ramp(&app.look().theme, app.look().depth, level);
                    let text = if style.bg.is_some() {
                        " "
                    } else {
                        ["▁", "▂", "▃", "▄", "▆", "█"][usize::from(level)]
                    };
                    (text, style)
                }
            };
            Span::styled(text, style)
        })
        .collect()
}
/// Qualifies every list-price figure the table shows.
pub(super) const PRICE_NOTE: &str = "Input tokens at list price; not a bill.";

pub(super) fn paint(frame: &mut Frame, app: &App, area: Rect) {
    let mut inside =
        crate::board::pane_chrome::paint(frame, app, area, Pane::Rows, "Agents by TOK");
    if inside.is_empty() {
        return;
    }
    let unit = app.savings_unit();
    let fields = tracks(inside.width, unit);
    let mut x = inside.x;
    for (field, width) in &fields {
        let caption = match field {
            Field::Agent => "AGENT",
            Field::State => "STATE",
            Field::Activity => "ACTIVITY · last 1d →",
            Field::Tokens => "TOK",
            Field::Share => "SHARE",
            Field::Saved => "SAVED ▾",
            Field::Cost => "≈ $ USED",
            Field::SavedCost => "≈ $ SAVED",
            Field::Digest => "DIGEST",
            Field::Talks => "TALKS TO",
        };
        if matches!(field, Field::Saved) {
            // The heading names the unit switch the summary and Details share.
            app.workspace_strip_hits.borrow_mut().push((
                Rect::new(x, inside.y, *width, 1),
                crate::board::app::Choice::SavingsUnit,
            ));
        }
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(x, inside.y, *width, 1),
            Line::styled(
                fit(caption, usize::from(*width)),
                app.look().role(Role::Muted),
            ),
        );
        x += width + 1;
    }
    inside.y += 1;
    inside.height -= 1;
    let rows = app.workspace_rows();
    let ids: Vec<_> = rows.iter().filter_map(metrics::id).collect();
    // A percent is never marked as a bound, so a partial one names its coverage here.
    let coverage = (unit == SavingsUnit::Percent)
        .then(|| super::savings(app))
        .filter(|sum| sum.percent.value.is_some() && sum.percent.included < sum.total)
        .map(|sum| {
            format!(
                "SAVED % covers {} of {} agents",
                sum.percent.included, sum.total
            )
        });
    let notes = fields
        .iter()
        .any(|(field, _)| matches!(field, Field::Cost))
        .then_some(PRICE_NOTE.to_owned())
        .into_iter()
        .chain(coverage)
        .collect::<Vec<_>>()
        .join(" · ");
    // A narrow pane drops whole columns; say which, so a missing price is not a missing feature.
    let room = usize::from(inside.width)
        .saturating_sub(unicode_width::UnicodeWidthStr::width(notes.as_str()))
        .saturating_sub(if notes.is_empty() { 0 } else { 3 });
    let footer = [Some(notes), narrow_hint(inside.width, unit, room)]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if inside.height > 1 && !footer.is_empty() {
        strip::paint_left(
            frame.buffer_mut(),
            Rect::new(inside.x, inside.bottom() - 1, inside.width, 1),
            Line::styled(
                fit(&footer, usize::from(inside.width)),
                app.look().role(Role::Muted),
            ),
        );
        inside.height -= 1;
    }
    let report = metrics::report(app);
    let talks = super::talks(app);
    let total = metrics::tokens(report, &ids);
    {
        let mut starts = app.row_starts.borrow_mut();
        starts.clear();
        starts.extend(0..rows.len());
    }
    if app.follow
        && let Some(index) = rows
            .iter()
            .position(|row| Some(&row.target) == app.workspace.selected.as_ref())
    {
        app.scrolls
            .reveal_range(Pane::Rows, index..index + 1, inside, rows.len());
    }
    app.scrolls.show_paint(
        frame,
        Pane::Rows,
        inside,
        rows.len().max(1),
        app.look(),
        |frame, viewport, offset| {
            for (index, row) in rows
                .iter()
                .enumerate()
                .skip(offset)
                .take(usize::from(viewport.height))
            {
                let cell = Rect::new(
                    viewport.x,
                    viewport.y + (index - offset) as u16,
                    viewport.width,
                    1,
                );
                let selected = Some(&row.target) == app.workspace.selected.as_ref();
                if selected {
                    frame.buffer_mut().set_style(cell, app.look().selection());
                }
                let row_ids = metrics::identities(row, &rows);
                let reading = metrics::tokens(report, &row_ids);
                // Both saved columns read one summary per row.
                let row_saved = fields
                    .iter()
                    .any(|(field, _)| matches!(field, Field::Saved | Field::SavedCost))
                    .then(|| row_savings(app, row, &row_ids));
                let mut x = cell.x;
                for (field, width) in &fields {
                    let slot = Rect::new(x, cell.y, *width, 1).intersection(cell);
                    x += width + 1;
                    if slot.width != *width {
                        continue;
                    }
                    if matches!(field, Field::State)
                        && let Some(label) = app.activity_label(row)
                    {
                        crate::board::workspace::columns::paint_activity(
                            frame.buffer_mut(),
                            app,
                            row,
                            &label,
                            slot,
                            selected,
                        );
                        continue;
                    }
                    let spans = if matches!(field, Field::Activity) {
                        hourly(app, &metrics::activity(report, &row_ids), selected)
                    } else {
                        let text = match (field, &row_saved) {
                            (Field::Saved, Some(summary)) => saved(summary, unit),
                            // The ≈ $ heading already qualifies the money; the cell does not
                            // repeat the ≈.
                            (Field::SavedCost, Some(summary)) => {
                                saved(summary, SavingsUnit::Cost).replacen('≈', "", 1)
                            }
                            _ => value(app, row, *field, &talks, reading, total, &row_ids),
                        };
                        // Numeric tracks never show a clipped amount.
                        let text = if matches!(
                            field,
                            Field::Tokens
                                | Field::Share
                                | Field::Saved
                                | Field::Cost
                                | Field::SavedCost
                        ) && unicode_width::UnicodeWidthStr::width(text.as_str())
                            > usize::from(*width)
                        {
                            "—".into()
                        } else {
                            fit(
                                &crate::board::notes::single_line(&text),
                                usize::from(*width),
                            )
                        };
                        vec![Span::styled(
                            text,
                            if selected {
                                app.look().selection()
                            } else {
                                app.look().role(Role::Text)
                            },
                        )]
                    };
                    strip::paint_left(frame.buffer_mut(), slot, Line::from(spans));
                    if matches!(field, Field::Digest)
                        && metrics::id(row).is_some_and(|id| {
                            super::model::labels(&app.labels, id)
                                .iter()
                                .any(|(_, _, choice)| *choice)
                        })
                    {
                        app.chip_hits.borrow_mut().push((slot, index));
                    }
                }
                app.hits.borrow_mut().push(Hit {
                    x: cell.x,
                    y: cell.y,
                    width: cell.width,
                    target: HitTarget::Row(index),
                });
            }
        },
    );
}
fn value(
    app: &App,
    row: &OpsTreeRow,
    field: Field,
    talks: &super::Talks,
    reading: Option<(u128, bool)>,
    total: Option<(u128, bool)>,
    ids: &[&str],
) -> String {
    match field {
        Field::Agent => row.title.clone(),
        Field::State => "—".into(),
        Field::Activity => unreachable!(),
        Field::Tokens => metrics::token_text(reading),
        Field::Share => metrics::share(reading, total),
        // The ≈ $ headings already qualify their money; the cells do not repeat the ≈.
        Field::Cost => metrics::input_price(app, metrics::report(app), ids)
            .map_or("—".into(), |micros| {
                crate::board::savings::usd_compact(micros).replacen('≈', "", 1)
            }),
        Field::Saved | Field::SavedCost => unreachable!(),
        Field::Digest if metrics::id(row).is_none() => {
            metrics::held(app, ids).map_or("—".into(), |count| format!("{count} held"))
        }
        Field::Digest => {
            let mode = metrics::mode(app, metrics::id(row));
            if metrics::id(row).is_some_and(|id| {
                super::model::labels(&app.labels, id)
                    .iter()
                    .any(|(_, _, choice)| *choice)
            }) {
                format!("{mode} ▾")
            } else {
                mode
            }
        }
        Field::Talks => metrics::id(row)
            .and_then(|id| talks.busiest(id))
            .map_or("—".into(), |(id, _)| metrics::name(app, id)),
    }
}

/// A member's own estimate is one status lookup; `all` reads the sum the summary tiles show.
fn row_savings(app: &App, row: &OpsTreeRow, ids: &[&str]) -> Rc<SavingsSummary> {
    if metrics::id(row).is_some() {
        Rc::new(metrics::savings(app, ids))
    } else {
        super::savings(app)
    }
}

/// One unit's amount: missing or uncombinable values read `—`, a known zero reads `0`, and a
/// sum that leaves members out reads as a lower bound, like TOK. A percent is a ratio, not a
/// sum, so it is never marked as a bound.
fn saved(summary: &SavingsSummary, unit: SavingsUnit) -> String {
    let (amount, included) = match unit {
        SavingsUnit::Tokens => (
            summary
                .tokens
                .map(|tokens| crate::source::tokens(u128::from(tokens))),
            summary.included,
        ),
        SavingsUnit::Percent => {
            return summary
                .percent
                .value
                .map_or("—".into(), crate::board::savings::percent_text);
        }
        SavingsUnit::Cost => (
            summary.cost.value.map(crate::board::savings::usd_compact),
            summary.cost.included,
        ),
    };
    amount.map_or("—".into(), |amount| {
        let bound = if included < summary.total { "≥" } else { "" };
        format!("{bound}{amount}")
    })
}
