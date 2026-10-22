//! Fit whole metric tracks; the hourly strip never changes its clock granularity.
use super::metrics;
use crate::board::{
    app::{App, Hit, HitTarget},
    exchange_matrix,
    view::fit,
    workspace::OpsTreeRow,
};
use crate::config::Pane;
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
};
use tmt_cli_style::{Role, theme::screen};
use tmt_tui::components::strip;

#[derive(Clone, Copy)]
enum Field {
    Agent,
    State,
    Activity,
    Tokens,
    Share,
    Cost,
    Digest,
    Talks,
}
fn tracks(width: u16) -> Vec<(Field, u16)> {
    use Field::*;
    let wide = width >= 94;
    let activity = width >= 70;
    let mut fields = vec![
        (Agent, if wide { 12 } else { 14 }),
        (State, if wide { 12 } else { 13 }),
    ];
    if activity {
        fields.push((Activity, 24));
    }
    fields.push((Tokens, if wide { 7 } else { 8 }));
    if wide {
        fields.extend([(Share, 5), (Cost, 9)]);
    }
    fields.push((Digest, 10));
    if wide {
        fields.push((Talks, 9));
    }
    // Agent names yield first. Metrics are either whole or omitted.
    let needed: u16 = fields
        .iter()
        .map(|(_, w)| w + 1)
        .sum::<u16>()
        .saturating_sub(1);
    fields[0].1 = fields[0].1.saturating_sub(needed.saturating_sub(width));
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
pub(super) fn price_column_shown(app: &App) -> bool {
    app.digest_placement(app.conversation.borrow().area)
        .list
        .is_some_and(|area| {
            let inside = crate::board::pane_chrome::content_area(area);
            !inside.is_empty()
                && tracks(inside.width)
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
/// Shown instead of the price note while the pane is too narrow for the share, price and
/// recipient columns.
pub(super) const NARROW_HINT: &str = "Widen to 94 columns for share, price and recipients.";

pub(super) fn paint(frame: &mut Frame, app: &App, area: Rect) {
    let mut inside =
        crate::board::pane_chrome::paint(frame, app, area, Pane::Rows, "Agents by TOK");
    if inside.is_empty() {
        return;
    }
    let fields = tracks(inside.width);
    let mut x = inside.x;
    for (field, width) in &fields {
        let caption = match field {
            Field::Agent => "AGENT",
            Field::State => "STATE",
            Field::Activity => "ACTIVITY · last 1d →",
            Field::Tokens => "TOK",
            Field::Share => "SHARE",
            Field::Cost => "≈ $ in",
            Field::Digest => "DIGEST",
            Field::Talks => "TALKS TO",
        };
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
    if fields.iter().any(|(field, _)| matches!(field, Field::Cost)) && inside.height > 1 {
        let footer = Rect::new(inside.x, inside.bottom() - 1, inside.width, 1);
        strip::paint_left(
            frame.buffer_mut(),
            footer,
            Line::styled(
                "Input tokens at list price; not a bill.",
                app.look().role(Role::Muted),
            ),
        );
        inside.height -= 1;
    } else if inside.height > 1 && !inside.is_empty() {
        // A narrow pane drops whole columns; say which, so a missing price is not a missing feature.
        let footer = Rect::new(inside.x, inside.bottom() - 1, inside.width, 1);
        strip::paint_left(
            frame.buffer_mut(),
            footer,
            Line::styled(
                fit(NARROW_HINT, usize::from(inside.width)),
                app.look().role(Role::Muted),
            ),
        );
        inside.height -= 1;
    }
    let rows = app.workspace_rows();
    let ids: Vec<_> = rows.iter().filter_map(metrics::id).collect();
    let report = metrics::report(app);
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
                        let text = value(app, row, *field, report, reading, total, &row_ids);
                        // Numeric tracks never show a clipped amount.
                        let text = if matches!(field, Field::Tokens | Field::Share | Field::Cost)
                            && unicode_width::UnicodeWidthStr::width(text.as_str())
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
    report: Option<&crate::board::usage_report::Report>,
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
        Field::Cost => metrics::input_price(app, report, ids)
            .map_or("—".into(), crate::board::savings::usd_text),
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
            .and_then(|id| metrics::recipient(report?, id))
            .map_or("—".into(), |(id, _)| metrics::name(app, id)),
    }
}
