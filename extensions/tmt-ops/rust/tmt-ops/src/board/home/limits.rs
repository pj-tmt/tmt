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
    if figures.empties_soon() {
        Role::Blocked
    } else if figures.runs_out_before_reset {
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

fn window_text(figures: &Figures) -> String {
    figures.short.map_or_else(
        || "5h reading not available".into(),
        |left| format!("5h {}", percent(left.percent)),
    )
}

fn meter(standing: &Standing, now_ms: u64, width: u16) -> Vec<Value> {
    let provider = standing.provider.label();
    let figures = match standing.outlook(now_ms) {
        Outlook::Known(figures) => figures,
        Outlook::Unknown { age_ms } => {
            let words = age_ms.map_or_else(
                || "no reading yet".into(),
                |ms| format!("no reading for {}", age(ms)),
            );
            return vec![piece(
                "missing",
                fit(&format!("{provider}  {words}"), usize::from(width))
                    .trim_end()
                    .into(),
                Role::Muted,
            )];
        }
    };
    let warning = warning(&figures);
    let reset_text = format!(" │ {}", reset(figures.weekly.resets_in_ms));
    let cells = 18.min(usize::from(width).saturating_sub(warning.width() + reset_text.width()));
    let critical = format!(" {} (7d) ", percent(figures.weekly.percent));
    let name = fit(&provider, cells.saturating_sub(critical.width() + 1));
    let label = fit(&format!(" {}{critical}", name.trim_end()), cells);
    let fill = ((cells as f64 * figures.weekly.percent / 100.0).round() as usize)
        .max(usize::from(figures.weekly.percent > 0.0))
        .min(cells);
    let (filled, track) = split_label(&label, fill);
    let mut pieces = vec![
        piece("filled", filled, state(&figures)),
        piece("track", track, Role::Text),
    ];
    if !warning.is_empty() {
        pieces.push(piece("warning", warning, state(&figures)));
    }
    pieces.push(piece("reset", reset_text, Role::Muted));
    if figures
        .burn
        .is_some_and(|rate| rate.is_finite() && rate > 0.0)
    {
        let extra = format!(" {}", rate_text(&figures));
        let used: usize = pieces
            .iter()
            .map(|piece| piece["text"].as_str().unwrap().width())
            .sum();
        if used + extra.width() <= usize::from(width) {
            pieces.push(piece("detail", extra, Role::Muted));
            if let Some(trend) = figures.trend {
                let (arrow, role) = match trend {
                    Trend::Up => (" ↗", Role::Waiting),
                    Trend::Same => (" →", Role::Muted),
                    Trend::Down => (" ↘", Role::Working),
                };
                if used + pieces.last().unwrap()["text"].as_str().unwrap().width() + arrow.width()
                    <= usize::from(width)
                {
                    pieces.push(piece("trend", arrow.into(), role));
                }
            }
        }
    }
    pieces
}

fn rows(standings: Option<&[Standing]>, now_ms: u64, width: u16) -> Value {
    let Some(standings) = standings else {
        return json!([{"id":"pending", "pieces":[piece("missing", PENDING.into(), Role::Muted)]}]);
    };
    let mut rows = Vec::new();
    let group = if width >= super::WIDE { 2 } else { 1 };
    for (index, chunk) in standings.chunks(group).enumerate() {
        let mut pieces = Vec::new();
        for (at, standing) in chunk.iter().enumerate() {
            if at > 0 {
                pieces.push(piece(&format!("gap{at}"), "       ".into(), Role::Dim));
            }
            let available = if group == 2 {
                width.saturating_sub(7) / 2
            } else {
                width
            };
            for mut part in meter(standing, now_ms, available) {
                let id = part["id"].as_str().unwrap();
                part["id"] = Value::String(format!("{id}{at}"));
                pieces.push(part);
            }
        }
        rows.push(json!({"id":format!("provider{index}"), "pieces": pieces}));
    }
    Value::Array(rows)
}

/// Hidden values use the same projection/budget as the meter, so detail never
/// repeats a rate/window/label that already survives in the current strip.
pub(in crate::board) fn hidden_limits(
    standings: &[Standing],
    now_ms: u64,
    width: u16,
) -> Vec<(&'static str, String)> {
    let available = if width >= super::WIDE {
        width.saturating_sub(7) / 2
    } else {
        width
    };
    let mut fields = Vec::new();
    for standing in standings {
        let provider = standing.provider.label();
        let parts = meter(standing, now_ms, available);
        let shown = parts
            .iter()
            .map(|part| part["text"].as_str().unwrap())
            .collect::<String>();
        let label = parts
            .iter()
            .filter(|part| matches!(part["id"].as_str(), Some("filled" | "track")))
            .map(|part| part["text"].as_str().unwrap())
            .collect::<String>();
        if !label.contains(&provider) {
            fields.push(("provider", provider.clone()));
        }
        let Outlook::Known(figures) = standing.outlook(now_ms) else {
            continue;
        };
        let reset = reset(figures.weekly.resets_in_ms);
        if !shown.contains(&reset) {
            fields.push(("reset", format!("{provider} {reset}")));
        }
        let rate = rate_text(&figures);
        if !shown.contains(&rate) {
            fields.push(("rate", format!("{provider} {rate}")));
        }
        let short = window_text(&figures);
        if !shown.contains(&short) {
            fields.push(("window", format!("{provider} {short}")));
        }
    }
    fields
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

#[cfg(test)]
mod tests;
