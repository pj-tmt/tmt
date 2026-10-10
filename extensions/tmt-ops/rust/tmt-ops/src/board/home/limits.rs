//! The provider-limits strip under the HOME usage line: per provider the weekly
//! and short window as the share left and the time to reset, the estimated burn
//! of the week and whether it empties the week before it resets. A provider
//! without a current reading says so in words and never shows an old number.
//! Built like the other HOME strips: the `lg` and `md` steps are a
//! `tmt-switch`, and what each shows is assembled here.
use super::{
    bar::{self, Piece},
    scene::{self, Kept, Key},
};
use crate::{
    board::limits::{Figures, Left, Outlook, Standing},
    look::Look,
};
use ratatui::text::Line;
use std::sync::OnceLock;
use tmt_cli_style::Role;
use tmt_tui::binding::Template;

/// Shown from the first frame until the worker's first sample arrives, so the
/// strip's row never appears late and moves the body.
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

fn window(label: &str, left: &Left, wide: bool) -> String {
    format!(
        "{label} {}{} · {}",
        percent(left.percent),
        if wide { " left" } else { "" },
        reset(left.resets_in_ms)
    )
}

fn figures(figures: &Figures, wide: bool) -> Vec<Piece> {
    let mut pieces = vec![(window("7d", &figures.weekly, wide), Some(Role::Text))];
    if let Some(burn) = figures.burn {
        pieces.push((format!(" · ~{burn:.1}%/h"), Some(Role::Text)));
    }
    if figures.runs_out_before_reset {
        // Words, never colour alone; the registered warning mark leads at `md`.
        let text = if wide {
            " · runs out before reset"
        } else {
            " ! before reset"
        };
        pieces.push((text.into(), Some(Role::Waiting)));
    }
    if wide {
        pieces.push((
            match &figures.short {
                Some(short) => format!("   {}", window("5h", short, wide)),
                None => "   5h –".into(),
            },
            Some(Role::Text),
        ));
    }
    pieces
}

fn strip(standings: &[Standing], now_ms: u64, wide: bool) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for (i, standing) in standings.iter().enumerate() {
        if i > 0 {
            pieces.push(("  │  ".into(), Some(Role::Dim)));
        }
        pieces.push((format!("{}  ", standing.provider.label()), Some(Role::Text)));
        match standing.outlook(now_ms) {
            Outlook::Known(known) => pieces.extend(figures(&known, wide)),
            Outlook::Unknown { age_ms: None } => {
                pieces.push(("no reading yet".into(), Some(Role::Muted)));
            }
            Outlook::Unknown { age_ms: Some(ms) } => {
                pieces.push((format!("no reading for {}", age(ms)), Some(Role::Muted)));
            }
        }
    }
    pieces
}

/// The strip for `md` and wider, painted again only when its key changed.
/// `None` for a board that has no provider to show.
pub(super) fn limits_in(
    slot: &mut Kept<Option<Line<'static>>>,
    standings: Option<&[Standing]>,
    now_ms: u64,
    width: u16,
    look: Look,
) -> Option<Line<'static>> {
    static TEMPLATE: OnceLock<Template<()>> = OnceLock::new();
    const FILE: &str = "squad.home.limits.xml";
    let template = TEMPLATE.get_or_init(|| {
        let markup = format!(
            r#"<tmt-view version="1"><tmt-switch><tmt-case min="lg">{}</tmt-case><tmt-case min="md">{}</tmt-case><tmt-default/></tmt-switch></tmt-view>"#,
            bar::row("wide"),
            bar::row("medium")
        );
        scene::compile(FILE, &markup, &bar::schema(&["wide", "medium"]))
    });
    let (wide, medium) = match standings {
        None => {
            let pending = vec![(PENDING.to_owned(), Some(Role::Muted))];
            (pending.clone(), pending)
        }
        Some([]) => return None,
        Some(standings) => (
            strip(standings, now_ms, true),
            strip(standings, now_ms, false),
        ),
    };
    let key = Key {
        width,
        look,
        selected: None,
        data: bar::data(&[("wide", wide), ("medium", medium)]),
    };
    slot.get(key, |key| bar::lines(bar::paint(key, FILE, template)))
        .clone()
}

#[cfg(test)]
mod tests;
