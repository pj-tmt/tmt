//! Cron presentation consumes service projections; it never acquires jobs or ticks.

use crate::look::Look;
use ratatui::text::{Line, Span};
use tmt_cli_style::Role;
use unicode_width::UnicodeWidthStr;

/// Already formatted future instant from the read-only service projection.
pub(super) struct Next<'a> {
    pub time: &'a str,
    pub owner: &'a str,
    pub message: &'a str,
}

/// Clock text is derived from #1317's read-only status by the acquisition owner.
pub(super) struct Summary<'a> {
    pub count: usize,
    pub next: Option<Next<'a>>,
    pub clock: &'a str,
    pub clock_role: Role,
}

fn display(text: &str) -> String {
    super::notes::sanitize(text)
        .lines()
        .next()
        .unwrap_or("")
        .to_owned()
}

/// Fit the optional preview first, then owner, retaining count/time/clock/action.
pub(super) fn line(summary: Summary<'_>, width: u16, look: Look) -> Line<'static> {
    let width = usize::from(width);
    let count = format!(
        "{} {}",
        summary.count,
        if width >= 100 { "cron jobs" } else { "jobs" }
    );
    let mut clock = display(summary.clock);
    let mut parts = vec![
        ("⑤ ".to_owned(), Role::Dim),
        ("⏱ ".to_owned(), Role::Accent),
        (count, Role::Text),
    ];
    let next = summary.next;
    if let Some(next) = &next {
        parts.push((" · next ".into(), Role::Dim));
        parts.push((display(next.time), Role::Text));
    }
    let clock_budget = width.saturating_sub(
        parts.iter().map(|(text, _)| text.width()).sum::<usize>()
            + " · clock: ".width()
            + " · c list".width(),
    );
    if clock.width() > clock_budget {
        clock = super::view::fit(&clock, clock_budget);
    }
    if let Some(next) = next {
        let fixed: usize = parts.iter().map(|(text, _)| text.width()).sum::<usize>()
            + " · clock: ".width()
            + clock.width()
            + " · c list".width();
        let owner = display(next.owner);
        let room = width.saturating_sub(fixed);
        if !owner.is_empty() && owner.width() + 1 <= room {
            parts.push((format!(" {owner}"), Role::Text));
            let room = room.saturating_sub(owner.width() + 2);
            let message = display(next.message);
            if !message.is_empty() && room >= 4 {
                parts.push((
                    format!(" {}", super::view::fit(&message, room.min(message.width()))),
                    Role::Muted,
                ));
            }
        }
    }
    parts.push((" · clock: ".into(), Role::Dim));
    parts.push((clock, summary.clock_role));
    parts.push((" · ".into(), Role::Dim));
    parts.push(("c list".into(), Role::Muted));
    let demand: usize = parts.iter().map(|(text, _)| text.width()).sum();
    if demand > width {
        // Tiny panes use the same grapheme fitter as the rest of the board.
        let compact = format!(
            "⑤ ⏱ {} · {} · c list",
            summary.count,
            display(summary.clock)
        );
        return Line::styled(
            super::view::fit(&compact, width),
            look.role(summary.clock_role),
        );
    }
    Line::from(
        parts
            .into_iter()
            .map(|(text, role)| Span::styled(text, look.role(role)))
            .collect::<Vec<_>>(),
    )
}

/// Display the service's instant in its stored zone, without recomputing slots.
pub(super) fn time(slot_ms: i64, now_ms: i64, zone: &str) -> Option<String> {
    let zone = jiff::tz::TimeZone::get(zone).ok()?;
    let slot = jiff::Timestamp::from_millisecond(slot_ms)
        .ok()?
        .to_zoned(zone.clone());
    let now = jiff::Timestamp::from_millisecond(now_ms)
        .ok()?
        .to_zoned(zone);
    Some(
        slot.strftime(if slot.date() == now.date() {
            "%H:%M"
        } else {
            "%a %m-%d %H:%M"
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary<'a>(message: &'a str, clock: &'a str) -> Summary<'a> {
        Summary {
            count: 4,
            next: Some(Next {
                time: "13:30",
                owner: "tmt-lead",
                message,
            }),
            clock,
            clock_role: Role::Dim,
        }
    }

    #[test]
    fn home_summary_retains_clock_action_and_future_time_at_supported_widths() {
        for width in [160, 100, 80] {
            for clock in ["tmt-core:cron %41", "no clock · tmt sq cron run", "unknown"] {
                let rendered = line(summary("merge queue sweep", clock), width, Look::default());
                let text = rendered.to_string();
                assert!(rendered.width() <= usize::from(width));
                assert!(text.contains("4 "));
                assert!(text.contains("next 13:30"));
                assert!(text.contains(&format!("clock: {clock}")));
                assert!(text.ends_with("c list"));
            }
        }
    }

    #[test]
    fn preview_steps_aside_without_clipping_clock_or_action() {
        let message = "merge queue sweep and check every pending review, then ask the leads for their decisions";
        let wide = line(summary(message, "tmt-core:cron %41"), 160, Look::default());
        let narrow = line(
            summary(message, "no clock · tmt sq cron run"),
            80,
            Look::default(),
        );
        assert!(wide.to_string().contains("merge queue sweep"));
        assert!(narrow.to_string().contains("next 13:30"));
        assert!(
            narrow
                .to_string()
                .ends_with("clock: no clock · tmt sq cron run · c list")
        );
        assert!(narrow.width() <= 80);
    }

    #[test]
    fn empty_job_list_has_no_invented_next_slot() {
        let text = line(
            Summary {
                count: 0,
                next: None,
                clock: "unknown",
                clock_role: Role::Waiting,
            },
            80,
            Look::default(),
        )
        .to_string();
        assert!(text.contains("0 jobs"));
        assert!(!text.contains("next"));
        assert!(text.contains("clock: unknown"));
    }

    #[test]
    fn display_preview_neutralizes_terminal_controls_and_keeps_source_exact() {
        let message = "  investigate\u{1b}[31m\u{202e}界 e\u{301}\nsecond line  ";
        let text = line(summary(message, "holder\u{1b}[2J"), 160, Look::default()).to_string();
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains('\u{202e}'));
        assert!(!text.contains("second line"));
        assert!(text.contains("  investigate"));
        assert!(message.ends_with("second line  "));
    }

    #[test]
    fn tiny_widths_stay_inside_the_available_cells() {
        for width in 0..80 {
            let rendered = line(
                summary("界界 e\u{301}", "no clock · tmt sq cron run"),
                width,
                Look::default(),
            );
            assert!(rendered.width() <= usize::from(width), "width {width}");
        }
    }

    #[test]
    fn long_clock_holder_does_not_displace_future_time_or_action() {
        let clock = format!("{} %41", "holder界".repeat(30));
        let rendered = line(summary("preview", &clock), 80, Look::default());
        assert!(rendered.to_string().contains("next 13:30"));
        assert!(rendered.to_string().ends_with("… · c list"));
        assert!(rendered.width() <= 80);
    }

    #[test]
    fn future_instant_label_uses_the_stored_zone_and_distinguishes_dates() {
        let now = "2026-10-04T14:30:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond();
        let next = "2026-10-04T15:30:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond();
        assert_eq!(
            time(next, now, "Asia/Tokyo").as_deref(),
            Some("Mon 10-05 00:30")
        );
        assert_eq!(time(next, now, "UTC").as_deref(), Some("15:30"));
        assert_eq!(time(next, now, "unavailable-zone"), None);
        assert_eq!(time(i64::MAX, now, "UTC"), None);
    }
}
