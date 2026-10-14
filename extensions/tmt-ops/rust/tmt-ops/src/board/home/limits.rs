//! HOME's provider meters, derived only from its already-held observations.
//! Labels are inside reverse-filled bars; warnings remain outside. The neutral
//! track is never a cursor row. Width branches share one admitted scene owner.
use super::scene::{self, Kept, Key, Part};
use crate::{
    board::{
        limits::{Figures, Outlook, Standing, Trend},
        view::fit,
    },
    look::Look,
};
use ratatui::{style::Modifier, text::Line};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::OnceLock};
use tmt_cli_style::{Depth, Role, grid::Align};
use tmt_tui::binding::{Schema, Template};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(crate) const PENDING: &str = "Updating limits…";

// Remaining weekly balance, independent of the depletion forecast.
const LOW_REMAINING_PERCENT: f64 = 15.0;
const HEALTHY_REMAINING_PERCENT: f64 = 40.0;

const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
const DAY_MS: u64 = 24 * HOUR_MS;

/// `4d03h`, `3h14m` or `45m`, the form the providers' own footers use.
fn reset(ms: u64) -> String {
    let (days, hours, minutes) = (ms / DAY_MS, ms % DAY_MS / HOUR_MS, ms % HOUR_MS / MINUTE_MS);
    match (days, hours) {
        (0, 0) => format!("{minutes}m"),
        (0, _) => format!("{hours}h{minutes:02}m"),
        _ => format!("{days}d{hours:02}h"),
    }
}

/// How long ago, in the one largest whole unit: `12m`, `3h` or `2d`.
fn age(ms: u64) -> String {
    match ms {
        ms if ms < HOUR_MS => format!("{}m", ms / MINUTE_MS),
        ms if ms < DAY_MS => format!("{}h", ms / HOUR_MS),
        ms => format!("{}d", ms / DAY_MS),
    }
}

fn percent(value: f64) -> String {
    if value > 0.0 && value < 1.0 {
        "<1%".into()
    } else {
        format!("{value:.0}%")
    }
}

fn state(figures: &Figures) -> Role {
    remaining_role(figures.weekly.percent)
}

fn remaining_role(percent: f64) -> Role {
    if percent < LOW_REMAINING_PERCENT {
        Role::Blocked
    } else if percent <= HEALTHY_REMAINING_PERCENT {
        Role::Waiting
    } else {
        Role::Working
    }
}

fn warning(figures: &Figures) -> String {
    if !figures.runs_out_before_reset {
        return String::new();
    }
    let Some(ms) = figures.empties_in_ms() else {
        return String::new();
    };
    format!(
        " {} ~{}",
        if figures.empties_soon() { "✗" } else { "!" },
        reset(ms)
    )
}

fn piece(id: &str, text: String, role: Role) -> Value {
    json!({"id": id, "text": text, "role": role.name()})
}

/// Split at a display-cell boundary without splitting a wide character or
/// detaching its combining marks. Both pieces retain the complete inside label.
fn split_label(label: &str, fill: usize) -> (String, String) {
    let mut filled = String::new();
    let mut track = String::new();
    let mut cells = 0;
    for ch in label.chars() {
        let width = ch.width().unwrap_or(0);
        if cells < fill || (width == 0 && track.is_empty()) {
            filled.push(ch);
        } else {
            track.push(ch);
        }
        cells += width;
    }
    (filled, track)
}

fn balance_parts(label: &str, percent: f64, cells: usize, role: Role) -> Vec<Value> {
    let fill = ((cells as f64 * percent / 100.0).round() as usize)
        .max(usize::from(percent > 0.0))
        .min(cells);
    let (filled, track) = split_label(label, fill);
    vec![
        piece("filled", filled, role),
        piece("track", track, Role::Text),
    ]
}

fn rate_text(figures: &Figures) -> String {
    figures.burn.map_or_else(
        || "pace not known".into(),
        |rate| {
            if rate == 0.0 {
                "no use measured".into()
            } else {
                format!("~{rate:.1}%/h")
            }
        },
    )
}

fn meter(standing: &Standing, now_ms: u64, width: u16) -> Vec<Value> {
    match standing.outlook(now_ms) {
        Outlook::Known(figures) => window_parts(standing, &figures, "7d", figures.weekly, width),
        Outlook::Unknown { age_ms } => {
            let words = age_ms.map_or_else(
                || "no reading yet".into(),
                |ms| format!("no reading for {}", age(ms)),
            );
            vec![piece(
                "missing",
                fit(
                    &format!("{}  {words}", standing.provider.label()),
                    usize::from(width),
                )
                .trim_end()
                .into(),
                Role::Muted,
            )]
        }
    }
}

/// Reported windows share the header's admitted cells and quota calculations.
/// Only the weekly window has a measured pace and depletion warning.
fn rows(standings: Option<&[Standing]>, now_ms: u64, width: u16) -> Value {
    let Some(standings) = standings else {
        return json!([{"id":"pending", "pieces":[piece("missing", PENDING.into(), Role::Muted)]}]);
    };
    let mut result = Vec::new();
    for (index, standing) in standings.iter().enumerate() {
        let Outlook::Known(figures) = standing.outlook(now_ms) else {
            result.push(
                json!({"id":format!("provider{index}"), "pieces":meter(standing, now_ms, width)}),
            );
            continue;
        };
        for (window, left) in
            std::iter::once(("7d", figures.weekly)).chain(figures.short.map(|left| ("5h", left)))
        {
            result.push(json!({"id":format!("provider{index}-{window}"),
                "pieces":window_parts(standing, &figures, window, left, width)}));
        }
    }
    Value::Array(result)
}

fn window_parts(
    standing: &Standing,
    figures: &Figures,
    window: &str,
    left: crate::board::limits::Left,
    width: u16,
) -> Vec<Value> {
    let weekly = window == "7d";
    let mut extra = format!(" reset {}", reset(left.resets_in_ms));
    let mut pace_visible = false;
    if weekly {
        let pace = format!(" · pace {}", rate_text(figures));
        if extra.width() + pace.width() + 18 <= usize::from(width) {
            extra.push_str(&pace);
            pace_visible = true;
        }
    }
    if extra.width() + 18 > usize::from(width) {
        extra.clear();
        pace_visible = false;
    }
    let warning = if weekly {
        warning(figures)
    } else {
        String::new()
    };
    let warning = if extra.width() + warning.width() + 18 <= usize::from(width) {
        warning
    } else {
        String::new()
    };
    let trend = if pace_visible
        && figures
            .burn
            .is_some_and(|rate| rate.is_finite() && rate > 0.0)
        && !warning.is_empty()
    {
        figures.trend.map(|trend| match trend {
            Trend::Up => (" ↗", Role::Waiting),
            Trend::Same => (" →", Role::Muted),
            Trend::Down => (" ↘", Role::Working),
        })
    } else {
        None
    };
    let cells = 28.min(usize::from(width).saturating_sub(
        extra.width() + warning.width() + trend.map_or(0, |(text, _)| text.width()),
    ));
    let critical = format!(" ({window}) {} left", percent(left.percent));
    let name = fit(
        &standing.provider.label(),
        cells.saturating_sub(critical.width()),
    );
    let label = fit(&format!("{}{critical}", name.trim_end()), cells);
    let mut pieces = balance_parts(
        &label,
        left.percent,
        cells,
        if weekly {
            state(figures)
        } else {
            remaining_role(left.percent)
        },
    );
    if !warning.is_empty() {
        pieces.push(piece(
            "warning",
            warning,
            if figures.empties_soon() {
                Role::Blocked
            } else {
                Role::Waiting
            },
        ));
    }
    pieces.push(piece("reset", extra, Role::Muted));
    if let Some((arrow, role)) = trend {
        pieces.push(piece("trend", arrow.into(), role));
    }
    pieces
}

/// The same admitted pieces as the provider strip, including role changes;
/// clipped or hidden provider rows cannot invalidate the frame.
pub(in crate::board) fn clock_marks(
    standings: Option<&[Standing]>,
    now_ms: u64,
    width: u16,
    count: usize,
) -> String {
    serde_json::to_string(
        &rows(standings, now_ms, width)
            .as_array()
            .unwrap()
            .iter()
            .take(count)
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

fn branch(name: &str) -> String {
    format!(
        r#"<tmt-repeat each="$.{name}" as="row"><tmt-row id-bind="row.id" class="w-full h-1"><tmt-repeat each="row.pieces" as="piece"><tmt-text id-bind="piece.id" bind="piece.text" token-bind="piece.role" class="shrink-0"/></tmt-repeat></tmt-row></tmt-repeat>"#
    )
}

pub(super) fn limits_in(
    slot: &mut Kept<Vec<Line<'static>>>,
    standings: Option<&[Standing]>,
    now_ms: u64,
    width: u16,
    look: Look,
) -> Vec<Line<'static>> {
    static TEMPLATE: OnceLock<Template<()>> = OnceLock::new();
    const FILE: &str = "squad.home.limits.xml";
    let template = TEMPLATE.get_or_init(|| {
        let piece = Schema::Object(BTreeMap::from([
            ("id".into(), Schema::StableId),
            ("text".into(), Schema::Scalar),
            ("role".into(), Schema::Scalar),
        ]));
        let row = Schema::Object(BTreeMap::from([
            ("id".into(), Schema::StableId),
            ("pieces".into(), Schema::Collection(Box::new(piece))),
        ]));
        let schema = Schema::Object(BTreeMap::from([(
            "providers".into(),
            Schema::Collection(Box::new(row)),
        )]));
        let markup = format!(
            r#"<tmt-view version="1">{}</tmt-view>"#,
            branch("providers")
        );
        scene::compile(FILE, &markup, &schema)
    });
    let key = Key {
        width,
        look,
        selected: None,
        data: json!({
            "providers":rows(standings, now_ms, width),
        }),
    };
    slot.get(key, |key| {
        scene::paint(
            FILE,
            template,
            &key.data,
            key.width,
            &mut |Part { id, role, .. }| {
                let id = id
                    .and_then(<[String]>::last)
                    .map(String::as_str)
                    .unwrap_or("");
                let style = if id.starts_with("filled") {
                    look.role(role).add_modifier(Modifier::REVERSED)
                } else if id.starts_with("track") {
                    if look.depth == Depth::None {
                        look.role(Role::Text).add_modifier(Modifier::UNDERLINED)
                    } else {
                        look.role(Role::Text).patch(look.role(Role::Track))
                    }
                } else {
                    look.role(role)
                };
                (style, Align::Left)
            },
        )
        .lines
    })
    .clone()
}

pub(in crate::board) mod detail;

#[cfg(test)]
mod tests;
