//! Held quota windows on shared Usage rows; Standing owns source validity.
use crate::{
    board::{
        home::limits as rules,
        limits::{Outlook, Standing},
    },
    look::Look,
};
use ratatui::text::{Line, Span};
use tmt_cli_style::Role;
pub(in crate::board) fn usage_row(
    standings: Option<&[Standing]>,
    provider: &str,
    now: u64,
    width: u16,
    look: Look,
) -> Line<'static> {
    let figures = standings
        .and_then(|items| items.iter().find(|s| s.provider.id() == provider))
        .and_then(|s| match s.outlook(now) {
            Outlook::Known(figures) => Some(figures),
            _ => None,
        });
    let mut spans = usage_window("7d", figures.map(|f| f.weekly.percent), width, look);
    if let Some(short) = figures.and_then(|f| f.short) {
        spans.push(Span::raw("  "));
        let mut tail = usage_window("5h", Some(short.percent), width, look);
        spans.append(&mut tail);
    } else if let Some(figures) = figures {
        spans.push(Span::styled(
            format!("  resets {}", rules::reset(figures.weekly.resets_in_ms)),
            look.role(Role::Muted),
        ));
    } else {
        spans.push(Span::styled("  5h   —", look.role(Role::Muted)));
    }
    let line = Line::from(spans);
    if line.width() <= usize::from(width) {
        line
    } else {
        Line::styled(
            crate::board::view::fit(&line.to_string(), usize::from(width)),
            look.role(Role::Muted),
        )
    }
}

fn usage_window(name: &str, percent: Option<f64>, width: u16, look: Look) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled(format!("{name} "), look.role(Role::Muted))];
    if width >= 36 {
        let filled = percent.map_or(0, |value| (value / 10.0).round().clamp(0.0, 10.0) as usize);
        let bar = if percent.is_some() {
            format!("{}{}", "█".repeat(filled), "░".repeat(10 - filled))
        } else {
            "·".repeat(10)
        };
        spans.push(Span::styled(bar, look.role(Role::Muted)));
    }
    spans.push(Span::styled(
        format!("{:>4}", percent.map_or("—".into(), rules::percent)),
        look.role(Role::Text),
    ));
    spans
}

pub(in crate::board) fn usage_clock_mark(
    standings: Option<&[Standing]>,
    now: u64,
    width: u16,
    look: Look,
) -> String {
    let mut providers: Vec<_> = standings
        .into_iter()
        .flatten()
        .map(|standing| standing.provider.id())
        .collect();
    providers.sort_unstable();
    providers.dedup();
    format!(
        "{:?}",
        providers
            .into_iter()
            .map(|provider| usage_row(standings, provider, now, width, look))
            .collect::<Vec<_>>()
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use tmt_cli_style::Depth;
    #[test]
    fn usage_windows_share_rows_neutral_roles_and_the_existing_time_fences() {
        use crate::board::limits::{Provider, Reading, Window};
        let now = 1_000_000;
        let look = Look::new(tmt_cli_style::Theme::default());
        let mut standings = vec![Standing {
            provider: Provider::named("claude"),
            latest: Some(Reading {
                observed_at_ms: now,
                weekly: Window {
                    left: 82.0,
                    resets_at_ms: now + 86_400_000,
                },
                short: Some(Window {
                    left: 98.0,
                    resets_at_ms: now + 3_600_000,
                }),
            }),
            burn: None,
            trend: None,
        }];
        let known = usage_row(Some(&standings), "claude", now, 36, look);
        assert_eq!(known.to_string(), "7d ████████░░ 82%  5h ██████████ 98%");
        assert_eq!(known.width(), 36);
        assert!(
            known
                .spans
                .iter()
                .filter(|span| !span.content.trim().is_empty())
                .all(|span| span.style == look.role(Role::Text)
                    || span.style == look.role(Role::Muted))
        );
        assert_eq!(
            usage_row(Some(&standings), "claude", now, 16, look).to_string(),
            "7d  82%  5h  98%"
        );
        standings[0].latest.as_mut().unwrap().weekly.left = 0.0;
        assert!(
            usage_row(Some(&standings), "claude", now, 36, look)
                .to_string()
                .starts_with("7d ░░░░░░░░░░  0%")
        );
        standings[0].latest.as_mut().unwrap().weekly.left = 100.0;
        standings[0]
            .latest
            .as_mut()
            .unwrap()
            .short
            .as_mut()
            .unwrap()
            .left = 100.0;
        let full = usage_row(Some(&standings), "claude", now, 36, look);
        assert_eq!(full.width(), 36);
        assert!(full.to_string().ends_with("100%"));
        for time in [now + 86_400_000, now + 7_200_001] {
            let stale = usage_row(Some(&standings), "claude", time, 36, look).to_string();
            assert!(stale.contains("—") && !stale.contains("82%") && !stale.contains("0%"));
        }
        let missing = usage_row(None, "codex", now, 36, look).to_string();
        assert!(missing.contains("—") && !missing.contains('%'));
        standings[0].latest.as_mut().unwrap().short = None;
        let reset = usage_row(Some(&standings), "claude", now, 36, look).to_string();
        assert!(reset.contains("resets 1d00h") && !reset.contains("5h"));
    }
    fn known(name: &str, percent: f64, short: Option<f64>, now: u64) -> Standing {
        Standing {
            provider: crate::board::limits::Provider::named(name),
            latest: Some(crate::board::limits::Reading {
                observed_at_ms: now,
                weekly: crate::board::limits::Window {
                    left: percent,
                    resets_at_ms: now + 7 * 24 * 60 * 60 * 1000,
                },
                short: short.map(|left| crate::board::limits::Window {
                    left,
                    resets_at_ms: now + 5 * 60 * 60 * 1000,
                }),
            }),
            burn: Some(1.0),
            trend: Some(crate::board::limits::Trend::Up),
        }
    }
    #[test]
    fn inline_windows_keep_neutral_roles_at_every_depth_and_invalid_ids_stay_out() {
        assert!(crate::board::limits::Provider::from_driver("\u{4ee3}\u{7406}").is_none());
        assert!(crate::board::limits::Provider::from_driver("e\u{301}").is_none());
        let now = 1_791_600_000_000;
        let readings = [
            known("long-provider-name-12345", 65.0, Some(80.0), now),
            known("low", 0.5, None, now),
        ];
        for depth in [Depth::TrueColor, Depth::Ansi16, Depth::None] {
            let look = Look {
                theme: tmt_cli_style::Theme::default(),
                depth,
            };
            for width in [16, 36] {
                for standing in &readings {
                    let row = usage_row(Some(&readings), standing.provider.id(), now, width, look);
                    assert!(row.width() <= usize::from(width));
                    assert!(
                        row.spans
                            .iter()
                            .all(|span| span.style == look.role(Role::Text)
                                || span.style == look.role(Role::Muted)
                                || span.style == Default::default())
                    );
                }
                assert!(
                    usage_row(Some(&readings), "low", now, width, look)
                        .to_string()
                        .contains("<1%")
                );
                assert!(
                    !usage_row(None, "missing", now, width, look)
                        .to_string()
                        .contains("0%")
                );
            }
        }
    }
    #[test]
    fn inline_clock_marks_follow_only_displayed_window_and_age_changes() {
        let now = 1_791_600_000_000;
        let look = Look::new(tmt_cli_style::Theme::default());
        let mut readings = [
            known("with-short", 95.0, Some(98.0), now),
            known("reset-only", 55.0, None, now),
        ];
        for reading in &mut readings {
            reading.latest.as_mut().unwrap().weekly.resets_at_ms += 1_800_000;
        }
        for width in [16, 36] {
            let mark = usage_clock_mark(Some(&readings), now, width, look);
            assert_eq!(
                mark,
                usage_clock_mark(Some(&readings), now + 5, width, look)
            );
            if width == 36 {
                assert_ne!(
                    mark,
                    usage_clock_mark(Some(&readings), now + 3_600_000, width, look)
                );
            } else {
                assert_eq!(
                    mark,
                    usage_clock_mark(Some(&readings), now + 3_600_000, width, look)
                );
            }
            assert_ne!(
                mark,
                usage_clock_mark(Some(&readings), now + 7_200_001, width, look)
            );
            let mut reverse = readings.clone();
            reverse.reverse();
            assert_eq!(mark, usage_clock_mark(Some(&reverse), now, width, look));
        }
    }
}
